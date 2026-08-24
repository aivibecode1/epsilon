# M15 — TUI usability overhaul (pi/opencode patterns ported)

Date: 2026-08-23 · Status: shipped. Response to user testing: "TUI is no way usable
at all — loop pi-agent and opencode deeply." This milestone ports the *patterns* that
make those references feel right, not their code.

## What was built

- **Picker viewport fix (the big one)**: `/models` rendered ALL rows in a fixed-height
  box — anything past ~10 entries was invisible AND unreachable (index kept moving into
  unrendered space). Now uses a centered scroll window ported from pi's `select-list`
  (`scroll_start = clamp(selected - viewport/2, 0, total-viewport)`), with an
  `n–m/total` position indicator in the footer line.
- **Editor hardening** (`editor_ops.rs`, pure + unit-tested): Ctrl+←/→ word jumps
  (unicode-aware: alnum+_ words, punctuation runs as units), Home/End, Delete,
  Ctrl+U kill-to-start, Ctrl+W kill-word-back — the pi/opencode editing baseline.
- **Markdown transcript rendering** (`markdown.rs` via pulldown-cmark): assistant
  messages now render headings (accent+bold), **bold**, *italic*, inline code styled,
  fenced blocks as dim gutter rows, bullet lists with indentation, blockquotes/rules.
  Replaces M13's strip-markers hack; raw text still goes to the MODEL untouched
  (display-only interpretation).
- **Spinner + elapsed time** while running (braille frames, pi loader convention),
  replacing the static status text.
- **Contextual hint bar** between transcript and input (pi keybinding-hints pattern):
  shows the keys that matter right now — dialog keys when a permission ask is up,
  picker keys when it's open, "Esc aborts" mid-run, command hints when idle.
- **Mouse wheel**: scrolls the transcript; navigates the picker list.
- **Bracketed paste**: multi-line pastes land in the editor as sanitized text instead
  of executing garbage keystrokes.
- **Slash autocomplete** (opencode convention): typing `/mo` pops candidates above the
  editor; Tab completes. Unknown `/word` already errors (M14).
- **Visual prefixes**: user lines get `❯`, tool results get `✓`/`✗`.
- **theme.rs**: central palette struct replacing scattered color literals.

## Test-caught notes

- My word-navigation test expectations were wrong twice (char-index arithmetic across
  two-space gaps and Greek letters) — the implementation was right; fixed the tests by
  hand-computing indices. Unicode cursor math is exactly why these ops live in pure
  functions with tests rather than inline in the key handler.
- crossterm 0.29 keeps bracketed-paste commands in `event::`, not `terminal::`
  (unlike some docs/examples).
- A stray `[System.IO.File]::WriteAllText` to the wrong path created a phantom
  `src/main.rs` ("x") that cargo auto-discovered as a bin target — deleted; another
  entry for the never-bulk-edit-files lesson.

## Verification

```
cargo fmt --all -- --check              # clean
cargo clippy --workspace --all-targets  # zero warnings
cargo test --workspace                  # 146 passed (was 136)
cargo run -q -p epsilon-cli -- --demo   # demo unchanged
cargo run -q -p epsilon-cli -- -p smoke # exit 0
```

New tests: scroll window centering/clamping (6), editor ops incl. unicode (2),
markdown mapping (5), plus existing suites green.

## Remaining TUI backlog (explicitly future)

Multi-line editor buffer (Shift+Enter newlines), kill-ring, themes via TOML palette
files, virtualized transcript for very long sessions. All additive on today's seams.
