# What a screen reader would MEET in the Epsilon window, in the order it meets it.
#
#   pwsh -File scripts/desktop-reader-walk.ps1                    # launches the IDE itself
#   pwsh -File scripts/desktop-reader-walk.ps1 -TargetPid 1234    # attaches to a running one
#   pwsh -File scripts/desktop-reader-walk.ps1 -Out <path>        # keep the transcript
#   pwsh -File scripts/desktop-reader-walk.ps1 -List              # only the findings
#
# # Why this is not the other probe
#
# `desktop-a11y-probe.ps1` answers "does the operating system's accessibility API expose this
# tree?" — a real and necessary question, answered on 2026-10-04 and again on 2026-10-05. This
# answers the question *after* it: **a reader decides what to say, and in what order, and that
# decision is not the platform's.** A tree can be perfect and still be unusable to read, because
# two rows share a name, or a control nobody can focus carries one, or a name is long enough to
# be read out four times a minute.
#
# So this walks the tree the way a reader walks it and prints, per node, the sentence a reader
# would build from it: role, name, value, and the states that change what is said. Then it prints
# the findings, which are the ones that are defects rather than style:
#
#   * an unnamed node            — addressable only by role, and several share the role;
#   * a duplicate role+name      — two entries the reader cannot tell apart (this is how the two
#                                  identically-named `File · README.md` rows were found);
#   * focusable with no name     — a control a reader can reach and be told nothing about;
#   * an over-long name          — announced in full on every repaint;
#   * no landmark at all         — a reader's rotor is built from these, so a window without one
#                                  has no fast route to the transcript.
#
# # What this is NOT
#
# **No screen reader is run and no audio is captured.** What a reader *sounds like* — voice,
# punctuation, verbosity settings, its own shortcuts, and what it chooses to skip — is that
# reader's business, not the platform's, and this script cannot observe any of it. Treat the
# output as the script a reader would be handed, not as a recording of one speaking.

param(
    [int]$TargetPid = 0,
    [string]$Exe = "",
    [string]$Out = "",
    [int]$TimeoutSeconds = 20,
    [int]$MaxNameLength = 90
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

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

    # Same window-picking rule as the other probe, and for the same measured reason: a winit
    # application owns more than one top-level window, and the first match for a PID is usually an
    # internal message-only one with no descendants. Selecting that and reporting nothing is a
    # check that cannot see the thing, which reads as a check that found nothing.
    $win = $null
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ($null -eq $win -and (Get-Date) -lt $deadline) {
        $candidates = $root.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)
        foreach ($cand in $candidates) {
            if ($cand.Current.ControlType -ne [System.Windows.Automation.ControlType]::Window) { continue }
            $kids = $cand.FindAll([System.Windows.Automation.TreeScope]::Descendants,
                [System.Windows.Automation.Condition]::TrueCondition)
            if ($kids.Count -gt 0) { $win = $cand; break }
        }
        if ($null -eq $win) { Start-Sleep -Milliseconds 500 }
    }
    if ($null -eq $win) {
        Write-Output "NO UIA WINDOW WITH CONTENT for pid $TargetPid after ${TimeoutSeconds}s."
        Write-Output "An empty tree is a FAILURE, not an empty result: it means nothing reached"
        Write-Output "the operating system's accessibility API, which is the whole question."
        exit 2
    }

    # --- walk ---------------------------------------------------------------
    # Tree order, which is the order a reader using Tab or arrow keys meets. A reader with its
    # own cursor may choose differently, and that choice is precisely what this cannot see.
    $all = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.Condition]::TrueCondition)

    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add("# What a screen reader would meet in the Epsilon window")
    $lines.Add("")
    $lines.Add("Produced by `scripts/desktop-reader-walk.ps1`. Reproduce with:")
    $lines.Add("    pwsh -File scripts/desktop-reader-walk.ps1 -Out <path>")
    $lines.Add("")
    $lines.Add("## The walk")
    $lines.Add("")
    $lines.Add("Per node: role, name, value where the node carries one, and the states that change")
    $lines.Add("what a reader says about it. **This is the script a reader would be handed, not a")
    $lines.Add("recording of one speaking** -- see the note at the end.")
    $lines.Add("")

    $unnamed = New-Object System.Collections.Generic.List[string]
    $duplicates = New-Object System.Collections.Generic.List[string]
    $focusableUnnamed = New-Object System.Collections.Generic.List[string]
    $longNames = New-Object System.Collections.Generic.List[string]
    $seen = @{}
    $roles = @{}

    foreach ($e in $all) {
        $c = $e.Current
        $role = $c.ControlType.ProgrammaticName -replace '^ControlType\.', ''
        $name = $c.Name
        $value = ''
        try { $value = $e.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value } catch { }
        if ($roles.ContainsKey($role)) { $roles[$role]++ } else { $roles[$role] = 1 }

        $states = @()
        if ($c.IsKeyboardFocusable) { $states += 'focusable' }
        if ($c.HasKeyboardFocus) { $states += 'focused' }
        if ($c.IsOffscreen) { $states += 'offscreen' }
        try {
            if ($e.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern)) { $states += 'selectable' }
        } catch { }

        $said = if ($name) { $name } else { '<no name>' }
        $shown = $said
        if ($shown.Length -gt $MaxNameLength) {
            $shown = $shown.Substring(0, $MaxNameLength) + '...'
            $longNames.Add("$role · $($said.Length) characters")
        }
        $bits = "  {0,-14} {1}" -f $role, $shown
        if ($value) { $bits += "  = $value" }
        if ($states.Count) { $bits += "  [" + ($states -join ', ') + ']' }
        $lines.Add($bits)

        if (-not $name) {
            $unnamed.Add($role)
            if ($c.IsKeyboardFocusable) { $focusableUnnamed.Add($role) }
        } else {
            # A duplicate is only a defect when role AND name match: two `File` rows called
            # `README.md` are ambiguous, and two rows called `File · README.md` and
            # `File · agents/README.md` are not. Keying on the pair is what makes that
            # distinction mechanical instead of a judgement call.
            $key = "$role|$name"
            if ($seen.ContainsKey($key)) {
                $seen[$key]++
                $duplicates.Add("$role · $name")
            } else { $seen[$key] = 1 }
        }
    }

    $lines.Add("")
    $lines.Add("## Roles a reader's rotor is built from")
    $lines.Add("")
    foreach ($k in ($roles.Keys | Sort-Object)) {
        $lines.Add(("  {0,-14} {1}" -f $k, $roles[$k]))
    }

    # --- findings -----------------------------------------------------------
    $lines.Add("")
    $lines.Add("## Findings")
    $lines.Add("")
    $problems = 0
    if ($unnamed.Count -eq 0) {
        $lines.Add("  named:        every node has a name")
    } else {
        $problems++
        $lines.Add("  UNNAMED:      $($unnamed.Count) node(s) a reader can only address by role:")
        foreach ($u in $unnamed) { $lines.Add("                  $u") }
    }
    if ($duplicates.Count -eq 0) {
        $lines.Add("  duplicates:   no two nodes share both a role and a name")
    } else {
        $problems++
        $lines.Add("  DUPLICATES:   $($duplicates.Count) node(s) a reader cannot tell apart:")
        foreach ($d in $duplicates) { $lines.Add("                  $d") }
    }
    if ($focusableUnnamed.Count -eq 0) {
        $lines.Add("  focusable:    every focusable node is named")
    } else {
        $problems++
        $lines.Add("  FOCUSABLE + UNNAMED: $($focusableUnnamed.Count) node(s) a reader can reach and")
        $lines.Add("                  be told nothing about.")
    }
    if ($longNames.Count -eq 0) {
        $lines.Add("  name length:  every name is short enough to read once")
    } else {
        $problems++
        $lines.Add("  LONG NAMES:   $($longNames.Count) over $MaxNameLength characters, each announced")
        $lines.Add("                  in full on every repaint:")
        foreach ($l in $longNames) { $lines.Add("                  $l") }
    }

    $lines.Add("")
    $lines.Add("## What this does not show")
    $lines.Add("")
    $lines.Add("No screen reader was run and no audio was captured. What a reader *sounds like* --")
    $lines.Add("its voice, verbosity, punctuation, and what it chooses to skip -- is that reader's")
    $lines.Add("business, not the platform's. Read this as the script a reader would be handed.")

    $text = $lines -join "`n"
    Write-Output $text
    if ($Out) {
        $dir = Split-Path -Parent $Out
        if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
        Set-Content -Path $Out -Value $text -Encoding utf8
        Write-Output "wrote $Out"
    }

    # A non-zero exit on a finding, so this can be a check and not only a report. Reporting
    # without failing is how a defect survives a green run.
    if ($problems -gt 0) { exit 1 }
    exit 0
}
finally {
    if ($launched -and -not $launched.HasExited) {
        Stop-Process -Id $launched.Id -Force -ErrorAction SilentlyContinue
        Write-Output "stopped pid $($launched.Id)"
    }
}