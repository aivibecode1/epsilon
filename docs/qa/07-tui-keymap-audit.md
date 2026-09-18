# TUI key-map audit (M29 follow-up)

Date: 2026-08-26. Prompted by a real bug the user hit: typing `x` in the editor
toggled tool-output expansion instead of inserting the letter. That bug class
(a bare letter bound to a command) is exactly what this audit sweeps for.

## The rule

Plain letters must TYPE. Commands need a modifier (Ctrl/Alt), a function key,
or an explicit modal state (permission dialog, picker, palette). No exceptions
in the main match arm — that is where M13's bare-`x` lived for months because
a test enshrined it.

## Audit result: all 26 letters

The main `match` in `handle_key` (`crates/epsilon-tui/src/app.rs`) binds every
plain character through one catch-all:

```text
KeyCode::Char(c) => app.insert_char_tracked(c)
```

Everything else is modifier-guarded. Full inventory as of this audit:

| Key | Action | Guard |
|---|---|---|
| Ctrl+C / Ctrl+Q | quit | pre-match check |
| Ctrl+M | model picker | CONTROL |
| Ctrl+T | thinking level cycle | CONTROL |
| Ctrl+P | command palette | CONTROL (+ trust gate) |
| Ctrl+A/E | line home / end | CONTROL |
| Ctrl+B/F | cursor char left/right | CONTROL |
| Ctrl+D | forward-delete char | CONTROL |
| Ctrl+J | newline (conhost-safe) | CONTROL |
| Ctrl+K | kill to line end | CONTROL |
| Ctrl+U | kill to line start | CONTROL |
| Ctrl+W | kill word back | CONTROL |
| Ctrl+Y | yank (paste from ring) | CONTROL |
| Ctrl+Z / Ctrl+. | undo / redo | CONTROL |
| Ctrl+X | expand/collapse tool outputs | CONTROL (since M29; was bare `x`, the bug) |
| Ctrl+O / Alt+E | external editor (`app.editor.external`, M59) | CONTROL / ALT (configurable; pi's own ctrl+g is taken here by the live refresh) |
| Alt+B/F | word left/right atomic | ALT |
| Alt+Backspace | kill word backward | ALT |
| Alt+Left | word left atomic | ALT |
| F2 / Shift+F2 | favorites cycle fwd/back | none needed (function key) + trust gate |
| Esc | popup dismiss → abort → quit | context ladder |
| Enter | submit (applies popup first) | — |
| Tab | apply highlighted suggestion | only when slash popup open |
| arrows, Home/End, PgUp/PgDn, Backspace, Delete | navigation/editing | — |

No other bare letter is captured. `x` was the only offender.

## Modal states own their keys by design

These capture plain letters ON PURPOSE while active, and each has a visible
surface so the mode is never invisible:

1. **Permission dialog** (`pending_ask`): `y/a/n/e` answer, everything else
   ignored. The dialog is rendered full-width with the `[y] allow · [a] always
   · [n] deny · [e] guidance` hint line.
2. **Model picker** (`picker_open`): letters filter the list. The picker owns
   half the screen when open.
3. **Command palette** (`palette_open`): letters type into the query box.
   Same visibility contract.

One subtlety verified during the audit: inside the picker, Ctrl+F/Ctrl+G are
matched with explicit CONTROL guards, and the guard-less `KeyCode::Char(c)`
arm below them cannot shadow them (match arms are ordered, guarded first).
The `unreachable!()` arm at the top of the picker's first match documents
that ordering invariant.

## Trust-gate completeness re-check (lesson 14)

Every entry point that persists config or mutates agent state was re-checked
against `trust_review_pending`:

- submits: gated in `submit_editor` (M22)
- initial task: staged, not run (M22 fix)
- palette open + its commands: gated (M22 audit)
- F2 cycling: gated (M27 audit)
- **Ctrl+P itself**: gated at open time
- **Ctrl+T (thinking cycle)**: NOT gated — reviewed and accepted as correct:
  `set_thinking` is session-only (an in-memory field on Agent, no config
  write, no tool execution). The gate exists to stop unreviewed RULES from
  executing; a thinking-level change runs nothing and persists nothing.
  Documenting that reasoning here so a future sweep does not "fix" it blind.
- **Ctrl+M / picker**: opens read-only UI. Model APPLY inside the picker does
  persist (`apply_model` → save_model_id + recents). Accepted: the picker is
  how you recover from a bad config (switch provider without running anything),
  and the M22 banner still holds every submit until review. Gating the apply
  would brick the documented recovery path.

## Test coverage

Key-path tests exist for: x-typing + Ctrl+X toggle, every Ctrl editor op,
F2 forward/backward/wrap, trust-gate blocks (palette, F2), slash popup nav,
Tab completion, Esc ladder, history recall. The audit added no new code —
it confirmed the map above matches reality and pinned it in this document.
