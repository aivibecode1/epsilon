# Read the Epsilon window's accessibility tree through Windows UI Automation (UIA).
#
#   pwsh -File scripts/desktop-a11y-probe.ps1                    # launches the IDE itself
#   pwsh -File scripts/desktop-a11y-probe.ps1 -TargetPid 1234    # attaches to a running one
#   pwsh -File scripts/desktop-a11y-probe.ps1 -Out <path>        # keep the dump
#   pwsh -File scripts/desktop-a11y-probe.ps1 -List             # what each pane exposes
#
# # Why this exists, and why it is not another unit test
#
# Every accessibility claim in this repository stopped one layer short:
# *"egui emitted the node into the AccessKit tree"*. Twenty-four desktop tests assert
# against `FullOutput::platform_output.accesskit_update` — a real `accesskit::TreeUpdate`,
# but one read **inside the process that produced it**. Nothing had ever checked that the
# tree reaches the **operating system's** accessibility API, which is the only thing an
# actual screen reader talks to. `docs/STATUS.md` said so in four places: *"no screen reader
# has ever been run against this window, on this machine or any other."*
#
# UIA is the Windows API NVDA and Narrator are built on. A client in **another process**
# asking UIA for the window's descendants is exactly the query a reader makes, and it is a
# layer of evidence none of the in-process tests can produce: it proves the tree crosses the
# process boundary, that roles survive the translation, and that names are announced.
#
# It is not a substitute for running a real reader — UIA gives the raw tree, while a reader
# decides what to *say* and in what order. It is the difference between "the harness believes
# it published something" and "the platform agrees". `desktop-reader-walk.ps1` covers the second
# half: it walks the same tree the way a reader walks it and reports the findings that are
# defects rather than style.
#
# **This probe is green: every node in the tree is named (M312).** It has exited 1 on one
# unnamed `ScrollBar` since it was written, and the window now names it — and names it *only*
# when it is really there. That last half is the whole difficulty, and it is written down at
# `crates/epsilon-desktop/src/app.rs` in `name_scrollbar`: `accesskit_node_builder` is
# get-or-create, so naming a bar that was never drawn puts a phantom in the tree a reader
# browses. The predicate therefore reproduces egui's own threshold from what `ScrollAreaOutput`
# publishes, and `a_scrollbar_is_named_only_when_there_is_one` asserts both directions — a
# pane that overflows is named, a pane that fits contributes no node at all.
#
# It is still **not a battery leg**, for a different reason now: it needs a desktop session and
# a window on screen, and the battery is meant to run without either.
#
# # The window does not have to render for this to work
#
# On the machine this was written for, no wgpu backend *presents* — the content area is blank.
# That does not matter here: accessibility is published by the application, not by the
# compositor, so the tree is complete on a window that draws nothing. That is why this probe
# works where a screenshot does not.

param(
    [int]$TargetPid = 0,
    [string]$Exe = "",
    [string]$Out = "",
    [int]$TimeoutSeconds = 20
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

# `$TargetPid`, not `$Pid`: PowerShell's `$PID` is the *current* process id and is
# read-only, so naming this parameter `Pid` fails at binding time with a message about a
# constant. Naming it after what it is also reads better at the call site.
$launched = $null
if ($TargetPid -eq 0) {
    if (-not $Exe) {
        $Exe = Join-Path $env:TEMP 'epsilon-target\debug\epsilon-desktop.exe'
    }
    if (-not (Test-Path $Exe)) {
        Write-Error "no binary at $Exe - build it first: cargo build -p epsilon-desktop"
        exit 127
    }
    $launched = Start-Process -FilePath $Exe -PassThru
    $TargetPid = $launched.Id
    Write-Output "launched $Exe (pid $TargetPid)"
}

try {
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $cond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ProcessIdProperty, $TargetPid)

    # **Pick the *real* window, and the first version of this did not.**
    #
    # A winit application owns more than one top-level window. The first match for a PID is
    # usually `class=Winit Thread Event Target` -- an internal, message-only window with no
    # name and no descendants. The probe selected it, reported `descendants: 0`, and
    # **exited 0**, because its only failure condition was "some node is unnamed" and zero
    # nodes have zero unnamed ones.
    #
    # That is the exact defect this repository keeps writing down: *a check that cannot see a
    # thing is indistinguishable from a check that found nothing.* It is recorded here rather
    # than quietly fixed, because the quiet fix is what makes it recur.
    #
    # So: poll for a child that is a real window *and* has children of its own, and fail
    # loudly if none appears.
    $win = $null
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ($null -eq $win -and (Get-Date) -lt $deadline) {
        $candidates = $root.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)
        foreach ($cand in $candidates) {
            $isWindow = $cand.Current.ControlType -eq
                [System.Windows.Automation.ControlType]::Window
            if (-not $isWindow) { continue }
            $kids = $cand.FindAll([System.Windows.Automation.TreeScope]::Descendants,
                [System.Windows.Automation.Condition]::TrueCondition)
            if ($kids.Count -gt 0) { $win = $cand; break }
        }
        if ($null -eq $win) { Start-Sleep -Milliseconds 500 }
    }
    if ($null -eq $win) {
        Write-Output "NO UIA WINDOW WITH CONTENT for pid $TargetPid after ${TimeoutSeconds}s."
        Write-Output "The process either exited, or published an empty accessibility tree."
        Write-Output "An empty tree is a FAILURE, not an empty result: it means nothing reached"
        Write-Output "the operating system's accessibility API, which is the whole question."
        exit 2
    }

    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add(("window: name={0} type={1} class={2}" -f `
        $win.Current.Name, `
        ($win.Current.ControlType.ProgrammaticName -replace '^ControlType\.', ''), `
        $win.Current.ClassName))

    $all = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.Condition]::TrueCondition)
    $lines.Add("descendants: $($all.Count)")
    $lines.Add('--- tree ---')

    $unnamed = New-Object System.Collections.Generic.List[string]
    $roles = @{}
    foreach ($e in $all) {
        $n = $e.Current.Name
        $t = $e.Current.ControlType.ProgrammaticName -replace '^ControlType\.', ''
        if ($roles.ContainsKey($t)) { $roles[$t]++ } else { $roles[$t] = 1 }
        if (-not $n) { $unnamed.Add($t) }
        $shown = if ($n) { $n } else { '<no name>' }
        if ($shown.Length -gt 90) { $shown = $shown.Substring(0, 90) + '...' }
        $lines.Add(("  {0,-22} {1}" -f $t, $shown))
    }

    $named = $all.Count - $unnamed.Count
    $lines.Add("--- named: $named of $($all.Count) ---")
    $lines.Add('--- roles ---')
    foreach ($k in ($roles.Keys | Sort-Object)) {
        $lines.Add(("  {0,-22} {1}" -f $k, $roles[$k]))
    }

    # **The finding that matters, printed last so it is what a reader of the output sees.**
    # An unnamed node is one a screen reader can only address by its role, which in a window
    # with several of the same role is no address at all.
    if ($unnamed.Count -gt 0) {
        $lines.Add("--- UNNAMED NODES: $($unnamed.Count) ---")
        foreach ($u in $unnamed) { $lines.Add("  $u") }
    } else {
        $lines.Add('--- UNNAMED NODES: none ---')
    }

    $text = $lines -join "`n"
    Write-Output $text
    if ($Out) {
        $dir = Split-Path -Parent $Out
        if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
        Set-Content -Path $Out -Value $text -Encoding utf8
        Write-Output "wrote $Out"
    }

    # A non-zero exit when a node is unnamed, so this can be used as a check rather than
    # only as a report. Reporting without failing is how a defect survives a green run.
    if ($unnamed.Count -gt 0) { exit 1 }
    exit 0
}
finally {
    if ($launched -and -not $launched.HasExited) {
        Stop-Process -Id $launched.Id -Force -ErrorAction SilentlyContinue
        Write-Output "stopped pid $($launched.Id)"
    }
}
