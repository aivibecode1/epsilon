# M197: the Windows stdin-inheritance defect (the "frozen / shrinking TUI")

**This is the first cause found for the owner's report that has survived a control.** Four
theories were previously measured and disproved (permission dialog, per-frame transcript clone,
console-window resize, `@`-completion walk). This one is a defect in the code, it is on the path
a user takes constantly, and it is fixed by matching what pi does.

## The defect

`crates/epsilon-tools/src/shell.rs:457-462` configured `stdout` and `stderr` as pipes and left
`stdin` **unset**. An unset `stdin` on `std::process::Command` means **inherit**, not "none" — the
default is `Stdio::inherit()`.

The harness's stdin, in the TUI, is the console's, in raw mode, carrying the user's keystrokes. So
**every command the agent runs was reading the same keys the user was typing**, on the same
console, while the TUI believed it owned input.

Two consequences, and they are the two things the owner reported:

- **"it freezes"** — keystrokes typed while a command runs are consumed by the child, so from the
  user's side the TUI has stopped responding. A key that "does nothing" is indistinguishable
  from a frozen program; this is the repo's own TUI invariant, violated from underneath it.
- **"it shrinks"** — the child draws into the same console the TUI is drawing into, and a console
  application launched without a hidden-window flag can allocate or attach its own console window,
  which competes for the same screen and changes the geometry the user sees.

## What pi does, quoted

`references/pi-0.85.1/packages/agent/src/harness/env/nodejs.ts:529-535`:

```ts
{
    cwd,
    detached: process.platform !== "win32",
    env: getShellEnv(this.shellEnv, options?.env, options?.inheritEnv),
    stdio: [commandFromStdin ? "pipe" : "ignore", "pipe", "pipe"],
    windowsHide: true,
},
```

and `:538-541` closes stdin when the command *is* transported over it:

```ts
if (commandFromStdin) {
    child.stdin?.on("error", () => {});
    child.stdin?.end(command);
}
```

So pi **never** inherits stdin, and it sets `windowsHide`. Epsilon did neither.

## The fix

`ChildStdio::configure(&mut cmd)` — one named place that wires the three streams and returns what it
decided:

- `stdin` → `Stdio::null()` — the child gets an empty handle that is EOF immediately.
- `stdout`/`stderr` → piped, drained incrementally (unchanged; the existing rationale stands).
- On Windows only, `creation_flags(CREATE_NO_WINDOW)` (`0x0800_0000`), Rust's spelling of pi's
  `windowsHide: true`. `CommandExt` is imported inside the `#[cfg(windows)]` block so the POSIX
  build never names a Windows-only trait.

`Stdio::null()` rather than `piped()`: nothing writes to the child, and a null handle is EOF
immediately, so a command that reads stdin gets empty input instead of blocking forever — the same
reason `shell_argv` keeps `-NonInteractive`.

## The test, and the two controls that were needed to trust it

**Control 1 — a behavioural test, and it could not detect the defect.** The first version ran
`read`/`Read-Host` in a real child and asserted it saw EOF. With stdin inherited the test was still
**green**, because `cargo test` runs the harness with a *redirected* stdin: from inside the test,
"inherited" and "closed" are indistinguishable. Only the TUI has a console on stdin, and that is
exactly the situation a unit test cannot stand up.

So the property is now asserted **where it is decided** — on the value `ChildStdio::configure`
returns — because `std::process::Command` exposes no getter for a stream it was given. The
behavioural half is kept as a second, smaller test
(`a_command_that_reads_stdin_gets_end_of_input_instead_of_blocking`): a child reading stdin gets EOF
promptly rather than hanging, which *is* observable and worth keeping.

**Control 2 — a stale backup nearly made the whole exercise meaningless.** The "fixed" copy taken
before the control had *already* had its `cmd.stdin(...)` line stripped by an earlier control run, so
restoring it reintroduced a `stdin_closed: true` that was **not true of the code beside it**, and the
test passed against a lie. Caught by reading the restored file rather than the test result — the rule
already in `AGENTS.md`: *after restoring a file, confirm what is in it.* A backup is only a backup
if it was taken from a known-good tree.

The final control asserts its own anchor before writing, and flips the reported flag as well as
removing the line, so the control is a lie-free control.

### Fixed build

```
test a_tool_child_gets_a_closed_stdin_and_never_the_harness_console ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 63 filtered out
```

### Control — same test, `cmd.stdin(Stdio::null())` removed and the flag made honest

```
CONTROL APPLIED OK: stdin line gone, flag says false

thread 'a_tool_child_gets_a_closed_stdin_and_never_the_harness_console' panicked at
crates\epsilon-tools\tests\builtin_tools.rs:1844:5:
a tool child must be given a closed stdin. An unset stream INHERITS, and in the TUI the
inherited stream is the console the user is typing into.

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 63 filtered out
```

Restored afterwards and verified **by content, not by a green test**: the
`cmd.stdin(Stdio::null())` line present once, `stdin_closed: true` twice (Windows and POSIX arms),
`stdin_closed: false` zero times.

## The general rule these two controls earn

> A test that exercises a *behaviour* cannot detect a defect in the *configuration* that produces
> it, when the test harness's own environment happens to mask the difference.
>
> When that happens, assert on the value the production path decides and returns. It is a smaller
> claim than "the user sees the right thing" — say so in the test's doc comment rather than letting
> a reader assume more.

## What this does NOT establish

The fix is verified at the level of the command's configuration and one observable behaviour. It has
**not** been observed end to end in a real conhost window running a real command, because this
session did not drive the built TUI interactively. The owner's confirmation on their own CMD and
PowerShell is the remaining evidence, and `scripts/console-probe.py` is the tool for it.
