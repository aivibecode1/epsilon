# M173 — the "acceptable trade" that was not one

## What this is

M172 closed a gap: filtering `exit` out of the sweep by name left `Command::Quit` with no
execution coverage, and the milestone recorded that as *"the one gap the design creates, in
exchange for not letting the sweep end its own session."*

It was a false trade, and the reason it looked reasonable is that I had not checked the one
thing that made it unnecessary. `type_and_enter` does not quit:

```rust
fn type_and_enter(app: &mut App, agent: &Arc<Agent>, text: &str) -> bool {
    for c in text.chars() {
        handle_key(app, agent, key(KeyCode::Char(c)));
    }
    handle_key(app, agent, key(KeyCode::Enter))
}
```

It **returns** `handle_key`'s quit signal. The caller decides whether to exit; the test
helper just reports it. So `/exit` could be typed, executed, and asserted like every other
command, with the assertion being *that it asks to quit* — which is stronger than the
"did something visible" the other 31 get, not weaker.

The filter is gone. The sweep now runs all 32 heads.

## Two assertions, not one

```rust
let should_quit = type_and_enter(&mut app, &agent, &format!("/{cmd}"));
if cmd == "exit" {
    assert!(should_quit, "`/exit` did not report the quit signal");
    continue;
}
assert!(
    !should_quit,
    "`/{cmd}` reported the quit signal, which would end the sweep's own session"
);
```

The second one is new and covers the other 31: a command that starts reporting the quit
signal would end the sweep's own session, so that is now a failure with a named cause
rather than a mysterious truncation partway through the loop.

## Control

`Command::Quit => return true` changed to `return false` in the dispatcher:

```
test result: FAILED. 0 passed; 1 failed
panicked at crates\epsilon-tui\src\app\tests.rs:2523:13:
`/exit` did not report the quit signal
```

Restored byte-identical and re-verified.

## The lesson, stated properly

M172 wrote down a gap in prose and moved on, having just spent a milestone removing a gap
of the same kind. **A limit I can cheaply close is not a limit, it is deferred work with a
respectable disguise.** The giveaway is that the write-up called it a trade — a trade
implies two real options, and here one of them had never been checked.

This is the same failure as M167's vacuous `/name` control and M160's two controls that
passed 363/363: accepting an unexamined claim about the code because it was convenient to
write down. The corrective is always the same: go read the function before writing the
sentence about it.

## Verification

- `epsilon-tui --lib`: **374 passed**, 0 failed — same count as M172, because the sweep count
  changed and the assertion count did not.
- Control above: 1 failed, at `tests.rs:2523`.
- Workspace battery `s302`: FMT=0, CLIPPY=0, TEST=0, DEMO=0; **62 binaries, 1,298 passed,
  0 failed**.

## Limits

- **Every limit M172 recorded still stands**: the reverse-drift check is source-shaped,
  aliases are covered through the head they alias rather than typed literally, and the
  sweep still asserts "did not panic, and did something visible" for the other 31.
- **`Command::Quit` is now covered for routing and its quit signal only.** That the app
  actually exits, unwinds cleanly, and persists the session on quit is a different claim
  about a different layer, and nothing here makes it.
- **`/exit` and its aliases `quit` and `q` are still one head's worth of coverage.** Typing
  `/quit` is not tested; it reaches the same arm.
