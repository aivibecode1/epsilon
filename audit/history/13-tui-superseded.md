# 04 — The TUI

The reported defect: *"it freezes and shows no input or output the moment you start
typing, in both CMD and PowerShell, even though tokens are clearly being consumed
underneath."*

**It reproduces. It is not a rendering freeze, and the earlier explanations were
wrong.** Below is the measurement, the causal chain, and what to change. The
fuller performance findings are in [03](03-defects.md) (D6, D7, D8, D15).

## How this was measured

Not by reading the code. `scripts/console-probe.py` opens a genuine conhost
window, attaches to it, launches the TUI *from the shell inside that console*,
injects real `KEY_EVENT` records, and reads the screen back.

One methodological note, because it nearly produced a false conclusion here: in
`--late` mode the probe's `after_screen` field is captured **before** typing
(`console-probe.py:503` vs `:506`). Reading that field and concluding "input is
dead" is wrong. The fields that answer the question are `typed_screen` (after
typing), `turn_screen` (after Enter), and `settled_screen` (after answering).

The host was `ConsoleWindowClass` — a real conhost, not Windows Terminal. A
`ConPTY` test cannot see this class of problem at all, which is why
`crates/epsilon-cli/tests/terminal.rs` never caught it.

## What actually happens

Command:

```sh
python scripts/console-probe.py --late --size 100x30 --enter --settle 5 \
        --keys "y" target/debug/epsilon.exe "say hi"
```

Typing worked. Submitting worked. The run worked. **Nothing was frozen.**

`typed_screen` — the composer held the text:

```
idle                          ctx ?/164k · ↑0 ↓0 · R0 W0 CH — · DeepSeek V3.1
E P S I L O N
  ready — describe a task and press enter
? say hi
```

`turn_screen` — 5 seconds after Enter, the run is **parked on a permission
dialog**:

```
? running 5s · streaming      ctx <1%/164k · ↑100 ↓50 · R0 W0 CH — · $0.0001 · DeepSeek V3.1
? ? say hi
? ? bash $ echo epsilon-ready · pending
? echo epsilon-ready
  ? permission required ─────────────────────────────
  │ cap  bash                                        │
  │ what echo epsilon-ready                          │
  │ y allow once   a always (session)                │
  │ n deny         e deny + guidance                 │
```

`settled_screen` — after pressing `y`, the run completes normally:

```
idle                          ctx <1%/164k · ↑200 ↓100 · R0 W0 CH — · $0.0001 · DeepSeek V3.1
? ? say hi
? ? bash $ echo epsilon-ready · ok · 6064ms
? epsilon-ready
? ? permission allowed
? Run complete.
```

## The causal chain

Four lines of code, in order. Each is individually defensible; together they
produce exactly the reported symptom.

**1. The first `bash` of every session asks.**
`crates/epsilon-safety/src/engine.rs:122`:

```rust
Rule::new("bash", "*", Action::Ask),
```

The default ruleset allows `read`, `grep`, `glob`, `write`, `edit`, and asks for
`bash`, `webfetch`, `external_dir` and secret reads. `pi` has no permission layer
at all, so this is an Epsilon addition — a defensible one. But it means **the
first tool call in a fresh TUI session opens a modal dialog**, and a user who
does not know to press `y` has no way to know.

**2. While the ask is pending, the composer is unreachable.**
`crates/epsilon-tui/src/app/keys.rs:384`:

```rust
if app.pending_ask.is_some() && !app.keybindings.matches("app.exit", &key) {
    return handle_approval_key(app, key);
}
```

This sits above every other branch except the transcript search, so ordinary
typing goes to the dialog.

**3. Unrecognised keys are discarded without a sound.**
`crates/epsilon-tui/src/app/keys.rs`, end of `handle_approval_key`:

```rust
        _ => {}
    }
    false
}
```

`y`, `a`, `n`/`Esc`, `e` do something. **Everything else vanishes — no echo, no
notice, no visible state change.**

**4. The dialog does not change while it waits.** It is a static frame.

**Therefore:** a user types a prompt, presses Enter, the model spends real tokens
(measured: `↑100 ↓50` before the ask even resolved), a dialog appears, and every
subsequent keystroke is silently eaten. The screen is not changing because
nothing is happening. To the user: *frozen; no input, no output; tokens being
consumed underneath.* All three halves of the report, exactly.

## The part that makes this a design failure, not a typo

The comment directly above that branch states the principle this code breaks:

> *"the approval dialog does: keys sent anywhere the user cannot see them are
> keys the user believes they pressed and did not."*

The project articulated the rule, applied it to the search overlay, and then
implemented the violation in the one place where it hurts most — the modal that
fires on the first tool call of every session. The gap is not that nobody knew.
It is that the rule was written as a comment instead of as an invariant.

## What to change

Ordered by how much each removes from the report.

**F1. Never discard a key without feedback.** A key the active dialog does not
recognise must produce a visible, transient notice — for example
`that key went to the permission prompt — y allows, n denies`. This alone turns
"frozen" into "I see why nothing happened", which is the difference between an
unusable harness and an understandable one. It is a small change in
`handle_approval_key` and it is the single highest-value fix in this document.

**F2. Make the composer visibly inert while a modal owns the keyboard.** The
composer still shows its placeholder, so it looks live and editable. Dim it, or
replace the placeholder with a line naming what has the keyboard and how to get
it back. The state is already known (`pending_ask.is_some()`).

**F3. Re-examine the `bash` default.** Three honest options, and the choice is
the owner's, not this audit's:

- *Keep `Ask`, fix the feedback.* Maximum safety, and F1/F2 make it tolerable.
- *Allow within the workspace, ask outside it.* Matches the `external_dir` rule
  that already exists, and means a first-run user in their own project is not
  met by a modal on their first command. Arguably what `read`/`write`/`edit`
  already do.
- *Allow once, remember for the session.* The `a always (session)` affordance
  already exists; making it the default path removes the modal from the common
  case while keeping the boundary.

What should **not** happen is the status quo: a safety default whose cost is
that the product looks broken.

**F4. The approval dialog should not be the only place `y`/`a`/`n` mean
something.** The repo's own skill names this:
`.agents/skills/ratatui-tui/SKILL.md:251` — "Check a pending tool-call approval
can't be skipped by a keypress the terminal had already buffered before the
prompt appeared, and that its `y`/`n`/`a` keys don't collide with whatever those
keys already do in the surrounding mode." The dialog is drawn over the composer,
whose normal `y` is ordinary text. The collision is real in the sense that
matters: the same key means two things depending on invisible state.

**F5. A panic must restore the console.** [03](03-defects.md) D6. There is no
panic hook and no `Drop` guard; a panic leaves raw mode, the alternate screen and
a hidden cursor, which is *also* indistinguishable from a freeze. Cheap to fix
and it converts an unexplainable failure into a readable one.

## Performance, measured and not

The freeze above is not a performance problem. But there are real ones, and they
will become the freeze the moment the dialog problem is fixed and a user runs a
long session.

| Cost | Where | Why it matters |
|---|---|---|
| Whole transcript deep-cloned per drawn frame | `epsilon-core/src/lib.rs:1934-1937` → `history()` at `:1114-1116` | O(session) allocation at up to 30 Hz, purely to compute one integer. [03](03-defects.md) D7. |
| Render cache contradicts its own O(1) claim | `epsilon-tui/src/render_cache.rs:310-313` vs `:350-357`, `:373-374` | A `HashSet` over every transcript line, every frame. [03](03-defects.md) D8. |
| In-flight answer wrapped twice per frame, allocating per word | `app/view.rs:549-562`, `app/panels.rs:264-292`, `wrap.rs:120-143` | ~28,000 allocations per frame for a 40 KB answer. [03](03-defects.md) D8. |
| `$EDITOR` hand-off blocks the whole loop | `epsilon-tui/src/app/editor.rs:85-90` (`child.status()`) | Synchronous child wait with inherited stdio, called from the event loop at `app/event.rs:111-115`. The model keeps running; the screen does not. |
| `/share` blocks on a network subprocess | `app/commands.rs:1203` | `gh gist create` with `.output()`, no timeout, on the event loop. |

**One measurement that looks alarming and probably is not.** The tool card read
`ok · 6064ms` for `echo epsilon-ready`. That is the *permission-gated* duration,
and the dialog was up for ~5 s of it, so the real tool time is plausibly ~1 s.
`docs/milestones/M98-windows-shell-stall.md` records this class of cost being
6.2 s for a one-word `echo` when every Windows `bash` loaded the user's PowerShell
profile. Worth one clean measurement with the timer unambiguous before anyone
concludes the profile load is back; it is not obviously back.

## What would have caught this

One test, and the harness to write it already exists. `crates/epsilon-cli/tests/terminal.rs`
types into the composer and submits, but **never types while a run is in flight** —
and the reported bug is a run in flight. See [03](03-defects.md) D15.

The test that would have caught it: submit a turn, then while it is blocked on
an approval, type a character and assert that either the character is visible
somewhere or the screen says why it was not. The second half is the real
assertion: **"nothing happened" must never be a state the UI can be in.**

The other half is a slow provider, so there is time to type. `SlowProvider` exists
(`epsilon-core/src/testing`) and is used widely in the offline tests, but the CLI
has no flag to reach it — `default_steps()` (`main.rs:1603-1611`) runs the faux
provider at full speed, so the TUI integration suite never has a window in which
to type. Adding a small diagnostic flag for that is the second prerequisite.

## Honest limits

- The measurement above used the **faux** provider. No live provider call was
  made, so "tokens are clearly being consumed underneath" was reproduced with
  scripted token counts (`↑100 ↓50`) rather than a real model. The causal chain
  does not depend on which provider is in use, but the exact wording a user sees
  while a real model streams may differ.
- One host class (`ConsoleWindowClass`) and one size (100×30). Windows Terminal
  was not measured here, and the repo's own evidence notes that
  `TERM_PROGRAM`-based truecolor detection (`ui/theme.rs:93`) claims truecolor
  for terminals that may not have it.
- The `?` characters in the probe readback are an artifact of reading a
  non-UTF-8 console buffer, not a rendering defect. That was not independently
  confirmed.
