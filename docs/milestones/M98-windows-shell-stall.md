# M98 — the six-second stall that read as a hang (`-NoProfile` on every tool call)

## What

The owner's report was "the moment I send a prompt it freezes" and a TUI that looked
unresponsive. The TUI was repainting correctly and the spinner kept turning — the *tool call* was
stalling. Every `bash` call on Windows spawned `powershell -Command`, which loads the user's
interactive PowerShell profile before it runs anything. Measured on this machine for a one-word
`echo`:

| invocation | round trip |
|---|---|
| `powershell -Command` (the profile loads) | **~6,200 ms** |
| `powershell -NoProfile -Command` | ~1,060 ms |
| `cmd /C` | ~70 ms |

Six seconds — that is what a one-line command costs, every time. From the TUI it is
indistinguishable from a deadlock: the spinner advances, the model waits, and nothing appears.
It is also the honest explanation of the approval-then-nothing sequence that had been read as a
freeze. The run was not stuck. It was paying for a profile it had no business running.

`-NoProfile` is now in the argument list. `-NoLogo` was already there; `-NonInteractive` stays,
because a command that prompts would otherwise wait forever on a pipe nobody is feeding.

The full loop, measured through the real `BashTool` and the real sandbox:

- before: 7,295 ms unsandboxed / 6,047 ms sandboxed
- after: 723 ms unsandboxed / 548 ms sandboxed

## Key decisions

- **The flag goes in the argv, and the argv is what the test checks.** The measurement is
  recorded in `shell_argv`'s own doc comment and pinned by
  `the_windows_shell_skips_the_user_profile`, because this is precisely the kind of flag a later
  cleanup drops as redundant — it looks like a duplicate of `-NoLogo` from the outside, and its
  absence is invisible on a machine whose profile is cheap.
- **`cmd /C` is ~15× faster and is deliberately not used.** The tool is named `bash` and
  documented as a POSIX-style shell. Silently handing the model `cmd` syntax would be a
  different and more confusing surprise than a slow start — the failure would arrive as a
  command that means something else rather than as a command that takes a second.
- **`shell_argv` became `pub` with a doc comment rather than staying a private helper.** The
  doc comment is the record: the table above, the reason `-NonInteractive` stays, and the
  reason `cmd /C` was refused. A private function's reasoning has nowhere to live except a
  commit message nobody re-reads.
- **The POSIX path is pinned in the same test module.** `the_posix_shell_is_bash_dash_c`
  asserts `bash -c <command>` on non-Windows, so the platform fork stays visible instead of the
  Windows branch quietly becoming the only one anybody looks at.
- **This is a diagnosis correction as much as a fix.** The stall had been attributed to the TUI
  repainting (`audit/11`'s G10); G10 is now recorded as *wrong* and closed by diagnosis, with
  G15 carrying the real cause.

## Gotchas

- **The profile is not incidental work — it is the user's interactive shell setup.** A profile
  that imports a module or runs a package manager turns every tool call into a six-second
  stall, and nothing in the harness said so. The tool's own start-up cost was invisible because
  it was attributed to the model thinking.
- **A per-call cost hides in a full-loop number.** The 6.2 s profile load and the 7.3 s
  end-to-end call are not the same measurement, and the difference is why the fix had to be
  measured through the real tool and the real sandbox rather than by timing `powershell` alone.
- **`-NonInteractive` was already correct and would have been easy to lose.** Reordering the
  arguments to put `-NoProfile` first is a change a reader can make without noticing that the
  three flags are three separate defences; the test asserts each one by name.
- **The probe that produced these numbers no longer exists.** It lived at `.tmp-console/bashtest`
  and was not checked in (see Verification).

## Verification

No battery log for this slice is preserved, and no gate log exists for M98 (`audit/evidence/`
holds `gate-m93b`, `gate-m94c`, `gate-m95b` and `m101-gates.log`). The evidence is the commit
message's own measurements, reproduced here as the commit states them:

- `cargo test -p epsilon-tools` — **91 passed, 0 failed**, including the new `shell_argv_tests`.
- The timings above come from a probe driving the real `BashTool` through
  `windows-job-object`, at `.tmp-console/bashtest`; the same table is recorded in
  `shell_argv`'s doc comment in `crates/epsilon-tools/src/shell.rs`.
- The test itself is the durable half of the evidence: `the_windows_shell_skips_the_user_profile`
  asserts `-NoProfile` is present, `-NonInteractive` is present, and the command is the last
  argument — so a regression fails a test rather than needing the timings re-measured.

The probe directory is **not** in the repository and its `bashtest` subdirectory no longer
exists on this machine; the numbers survive in the commit message and in `shell_argv`'s doc
comment, which is where a reader will actually meet them.

## Limits

- **The numbers are one machine's, for one command shape.** A one-word `echo`. A profile-heavy
  machine would widen the gap and an empty profile would narrow it; the *ordering* — profile
  loads, so the call waits — is the durable claim, and the exact milliseconds are not.
- **Nothing pins the *timing*, only the argv.** A future change that keeps `-NoProfile` but
  adds a slow wrapper would pass every test here. The measurement in the doc comment is the
  only thing that would make a reader look.
- **The shell is still Windows PowerShell 5.1, invoked as `powershell`.** PowerShell 7, WSL and
  git-bash are all available on a developer machine and none is selected; a model writing
  POSIX-style commands still gets them interpreted by PowerShell, which is a pre-existing
  fidelity gap this milestone did not touch.
- **`bash` remains a name that does not match the interpreter.** The decision above explains why
  `cmd /C` was refused, not why the tool is called `bash` on a platform with no `bash`; that
  naming problem predates this slice.
- **The full-loop timings were not re-measured after the fix on an independent occasion**, and
  the sandboxed figure depends on the job-object backend's own start-up cost, which this slice
  did not measure separately.
- **The probe is not preserved.** `.tmp-console/bashtest` was a scratch artifact and was not
  committed, so nobody can re-run the exact measurement that produced the table without
  rebuilding the probe. The test and the doc comment are the preserved evidence; the raw
  numbers are the commit message's.
- **Windows-only.** The POSIX branch is covered by an assertion, not by a Linux run; the Linux
  CI job has not executed on these bytes.
