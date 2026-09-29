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

## **Fixing one site was not fixing the defect**

**Four production spawn sites had the identical omission, and the first fix reached one of them.**

| Site | What it spawns | Fixed |
|---|---|---|
| `epsilon-tools/src/shell.rs` | the `bash` tool, unconfined | `8846654` |
| `epsilon-safety/src/sandbox/jobobject.rs:256` | the Windows confined path — **the default** | this commit |
| `epsilon-safety/src/sandbox.rs:442` | the bwrap confined path | this commit |
| `epsilon-mcp/src/stdio.rs:29` | every MCP server | this commit |
| `epsilon-llm/src/oauth_login.rs:1125` | `open_browser` → `cmd /C start` | this commit |

The last one was found by the guard below, not by reading: nothing about "open a browser" looks
like it runs a command, so it is exactly the site a human sweep misses. `open_browser` is not
interactive and was inheriting the TUI's console handle all the same.

**The lesson is about the shape of the fix, not its size.** A per-site test passes forever while
the next site is unwritten, and a green battery after the first fix said nothing about the other
four. The property that needed asserting was "no spawn anywhere leaves this unset" — a claim about
the *tree*, not about a function.

## The guard, and what it took to make it correct

`no_production_spawn_leaves_stdin_unset` (in `epsilon-cli/src/runtime.rs`, beside the existing
repo-hygiene guards) walks every `.rs` under `crates/`, finds each `Command::new`, and requires
`stdin` to be configured nearby. `Stdio::null()`, `Stdio::piped()`, an explicit
`Stdio::inherit()` (the external editor, which legitimately owns the terminal), a `.output()`
call (which pipes internally), and delegation to `ChildStdio::configure` all satisfy it.

**It took four attempts, and each failure was the guard lying rather than the code.** Worth
recording, because a guard that cries wolf gets deleted, and this one nearly was:

1. It matched **its own source** — the string `Command::new(` inside its comparison.
2. Its `#[cfg(test)]` filter used the byte offset of the marker and compared it against a **line
   index**, so it exempted the wrong half of `runtime.rs` and flagged every production spawn in
   any file whose test module came later.
3. A 6-line lookahead window missed `editor.rs`, which configures twelve lines down.
4. `#[cfg(test)]` on a *function* is not a test *module*; treating it as one counted
   `jobobject.rs`'s entire test module as production.

**Rule this earns:** when a guard's first run fails, decide deliberately whether it found something
real or whether the guard is wrong — and when it finds something real *twice in the same file*,
the second one is more likely a bug in the guard. Also: assert the guard's own blind spot, or it
has none.

### Fixed tree

```
test runtime::protocol_doc_tests::no_production_spawn_leaves_stdin_unset ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 138 filtered out
```

### Control — same test, one site's `stdin` line removed

```
crates\epsilon-safety\src\sandbox\jobobject.rs:256: let mut command = Command::new(&cmd.program);
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 138 filtered out
```

The control names the reverted site by file and line. The file was then restored and **verified by
content** (`stdin(Stdio::null())` present once), not by a green test — the rule that caught a lie
in the previous round of this same work.

## What this does NOT establish

The fix is verified at the level of the command's configuration and one observable behaviour. It has
**not** been observed end to end in a real conhost window running a real command, because this
session did not drive the built TUI interactively. The owner's confirmation on their own CMD and
PowerShell is the remaining evidence, and `scripts/console-probe.py` is the tool for it.
