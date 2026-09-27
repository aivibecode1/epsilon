# The Windows typing-freeze — root cause

**Status: root cause found, reproduced, measured. Fixes not yet implemented.**

## The answer

It is not a rendering freeze. It is an **input-ownership design with no feedback**,
plus a default policy that makes it fire on the first tool call of every session.

Causal chain, all four lines verified by reading and by measurement:

1. `crates/epsilon-safety/src/engine.rs:122` — `Rule::new("bash", "*", Action::Ask)`.
   The first `bash` of any TUI session opens a permission dialog.
2. `crates/epsilon-tui/src/app/keys.rs:384` — while `pending_ask.is_some()`, every key
   routes to `handle_approval_key`. The composer is unreachable.
3. `crates/epsilon-tui/src/app/keys.rs` (end of `handle_approval_key`) — `_ => {}`.
   Any key that is not `y`/`a`/`n`/`Esc`/`e` is **silently discarded**: no echo,
   no notice, no state change.
4. The dialog is a static frame, so the screen does not change while it waits.

Result: the model spends real tokens (measured `↑100 ↓50` before the ask resolved),
a dialog appears, and every later keystroke vanishes. The user sees a frozen screen
while tokens are genuinely being consumed. That is all three halves of the report.

**The irony worth remembering:** the comment immediately above that branch states
the rule the code breaks — *"keys sent anywhere the user cannot see them are keys
the user believes they pressed and did not."* The project knew. It wrote the rule
as a comment instead of as an invariant.

## How to reproduce (two minutes)

```powershell
Remove-Item .epsilon\sessions\tui.lock -ErrorAction SilentlyContinue   # see trap below
python scripts/console-probe.py --late --size 100x30 --enter --settle 5 `
        --keys "y" D:\eps-target\debug\epsilon.exe "say hi"
```

Read `turn_screen` (dialog up) and `settled_screen` (after `y` — it completes
normally). The harness is not broken; the user is simply never told to press `y`.

**Methodological trap that nearly gave a false answer:** in `--late` mode the
probe's `after_screen` is captured *before* typing (`console-probe.py:503`, typing
at `:506`). Reading it and concluding "input is dead in conhost" is wrong. Use
`typed_screen`, `turn_screen`, `settled_screen`.

## The fix, in priority order

- **F1** Never discard a key without feedback — push a transient notice naming what
  has the keyboard. Highest value in the whole audit; converts "frozen" into
  "I see why".
- **F2** Make the composer visibly inert while a modal owns the keyboard.
- **F3** Re-examine the `bash` default (keep-and-fix / allow-in-workspace /
  allow-once). The owner's call, not the audit's.
- **F4** `y`/`a`/`n` collide with composer text — the repo's own skill flags this
  at `.agents/skills/ratatui-tui/SKILL.md:251`.
- **F5** A panic must restore the terminal (no panic hook, no `Drop` guard today).

## The trap that cost time

`scripts/console-probe.py` ends with `taskkill /F`, which leaves a stale
`.epsilon/sessions/<mode>.lock`. The next launch then blocks for 30 s on
`STALE_AFTER`/`ACQUIRE_WAIT` (`lease.rs:50,59`) and shows a truncated
"waiting for this session to be released" frame with no working input.
**This was already documented on 2026-09-21** in
`audit/evidence/console-probe-2026-09-21.md` and never fixed.

Delete the lock before every probe run. This is also a real user-facing bug: a
crashed TUI bricks the workspace for 30 seconds per launch, and the recorded pid
is deliberately never probed for liveness.

## Why the test suite missed it

`crates/epsilon-cli/tests/terminal.rs` never types while a run is in flight, and
the faux provider is instant (`default_steps()`, `main.rs:1603-1611`) so there is
no window in which to type. A `SlowProvider` exists in `epsilon-core/src/testing`
but no CLI flag reaches it. Adding that flag is a prerequisite for the test that
would have caught this.
