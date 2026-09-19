# M59 — External editor (`$EDITOR` on the draft)

The last pi TUI affordance with no Epsilon counterpart, found by a parity sweep
of pi's `modes/interactive/` tree: pi's `app.editor.external` hands the current
draft to the user's own editor and reads the result back. Epsilon had nothing —
composing a long prompt, or reviewing a pasted stack trace before sending it,
meant editing inside the one-line-at-a-time input box.

## What landed

**Resolution** (`editor_command_from`, pure): explicit config wins, then
`$VISUAL`, then `$EDITOR` (pi's `getExternalEditorCommand` order). Nothing
configured is a *telling* error — it names `$EDITOR`, `$VISUAL`, and the exact
config shape to add — and it is reported at request time, in the transcript, not
by a silent no-op.

**The round trip** (`edit_with`, terminal-free): draft → temp file
(`epsilon-editor-<id>/prompt.md`) → the user's command → read back with exactly
one trailing newline trimmed (pi's `.replace(/\n$/, "")`), the temp directory
removed either way. Failure of the child (non-zero exit, spawn error) leaves the
draft untouched and says so, because losing a composed prompt is the one outcome
worse than the feature not working.

**The terminal hand-off** (`open_external_editor`, the new seam): the TUI
suspends — disable bracketed paste, `ratatui::try_restore()` (leave raw mode +
alternate screen), spawn the editor with inherited stdio through the platform
shell, wait, `ratatui::try_init()`, re-arm bracketed paste, clear. A *fresh*
terminal is installed rather than the old one reused, so ratatui's buffers start
blank and the next frame is a full repaint instead of a stale half-screen.

**Ownership**: handlers never touch the terminal. `/editor` and the keybinding
set `App::external_editor_requested`; the event loop performs the hand-off
between frames. That is also what makes the request testable without a TTY.

**Trigger**: `/editor` (discoverable, in the slash table and autocomplete) and
`app.editor.external` bindings — `Ctrl+O` and `Alt+E`, both configurable in
`~/.config/epsilon/keybindings.json`. pi's own default (`ctrl+g`) is taken here
by the live-model refresh, so the divergence is deliberate and documented.

## Key decisions

- **Suspend between frames, driven by a flag.** The alternative (passing the
  terminal into key handling) would give every handler a way to corrupt the
  screen; a flag keeps the terminal owned by exactly one function.
- **A fresh terminal after the child exits.** Re-using the old `Terminal` keeps
  its back buffer, which now describes a screen the editor overwrote — the
  repaint would skip anything it believes is already drawn.
- **The spawner is a parameter, not a global.** `edit_with` takes the spawn
  closure, so the whole write/edit/read/trim/cleanup path is covered by tests
  that never launch a process; the real path is a five-line shim.
- **Config through the config brick.** `epsilon_core::config::load_config_string`
  is new and generic (`[editor] command = "code --wait"`); the TUI does not parse
  TOML, matching how it reads favorites and recents.

## Gotchas

- The first wiring put the keybinding arm inside the wrong match (the editor
  mode's), where arms must return `bool` — the compiler caught it immediately.
  Worth noting only because the same edit silently displaced a neighbouring
  comment: after moving match arms, re-read the surrounding block.
- `ratatui::try_init`/`try_restore` are the pair the TUI already used for
  startup/teardown; the editor hand-off is the first time they are used
  mid-session, so re-arming bracketed paste after the child exits is on us
  (the startup path does it once, outside `drive`).
- Windows: the child runs through `cmd /C` so `code --wait` and friends work as
  the user wrote them; the file path is quoted because temp paths contain
  spaces.

## Verification

- `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets`
  zero diagnostics.
- `cargo test --workspace`: **441 passed, 0 failed** (436 before: +5 TUI tests).
  `--features real` lane: 125 passed, 0 failed.
- New coverage: resolution order (config → `$VISUAL` → `$EDITOR`, blanks
  ignored, error names both remedies), the round trip with a scripted spawner
  (draft written, edit applied, one trailing newline trimmed), a failing child
  keeping the draft, the request path refusing mid-run/dialog and reporting a
  missing editor, and keybinding/`/editor` taking the same path.
- `cargo run -q -p epsilon-cli -- --demo` completes.
