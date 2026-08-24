# M16b — Editor v2 (pi editor behaviors on our single-line-first base)

Date: 2026-08-24 · Status: shipped. Kill-ring, atomic paste markers, multi-line
buffer, and persistent history — the pi `editor.ts` interaction set.

## What was built

### 1. Kill-ring (`kill_ring.rs`, pure + tested)
- Emacs semantics ported from pi's `kill-ring.ts`: Ctrl+U / Ctrl+W push kills,
  Ctrl+Y yanks. Consecutive kills accumulate into ONE ring entry (backward kills
  prepend); repeated Ctrl+Y cycles entries yank-pop style (rotate + replace).
- Depth 16 (handoff required ≥10), oldest dropped beyond capacity.
- The App tracks `last_kill_was_contiguous` so kill→kill merges but kill→edit→kill
  starts a fresh entry, and any non-yank edit ends a yank sequence.

### 2. Atomic paste markers
- Bracketed pastes become `[paste #N +L lines]` (or `[paste #N C chars]` for long
  single lines) in the buffer; payloads stored session-side keyed by id.
- Markers expand back to full text on submit — the model sees everything, the
  editor stays compact.
- `is_paste_marker` validates strictly (digits id, optional well-formed detail) so
  user-typed `[not a marker]` or `array[0]` pass through untouched.
- Word-nav gained atomic variants (`word_left_atomic`/`word_right_atomic`) so
  Ctrl+W removes a whole marker in one step (pi's `isAtomicSegment`).

### 3. Multi-line buffer
- Ctrl+J inserts `\n` (the reliable combo on legacy Windows consoles where
  Ctrl+Enter arrives as plain Enter — see memory/known-traps). Enter submits.
- Input box renders embedded newlines via ratatui `Text`; caret math walks the
  buffer tracking line index and per-line column, clamping to the box.

### 4. Persistent history (`epsilon-core::config`)
- `~/.config/epsilon/history`: JSON-lines (multiline entries serialize safely),
  capped 500, consecutive-duplicate collapse, corrupt lines skipped on read.
- Seeded into the recall ring at TUI startup → Up-arrow works across restarts;
  each submit appends best-effort (failures surface as a notice, never block).

## Key-path tests (the M14 lesson applied)
All new behavior drives `handle_key` with synthetic `KeyEvent`s:
Ctrl+W→Ctrl+Y round trip; consecutive-Ctrl+W accumulation; Ctrl+J newline +
Enter submit + history persistence; paste-marker expansion through the real
submit path. Plus pure-module coverage for ring depth/cycling, marker grammar,
marker round-trip expansion, and atomic word-nav.

## Test-caught bugs

- My first yank implementation had inverted boolean nonsense (`!n != true`) that
  compiled fine — the key-path test caught it immediately. Rewrote cleanly.
- History cap test asserted `"task 109"` when the loop wrote to 509; fixture bug,
  not code. The code was right: newest kept, oldest dropped.
- Clippy collapsed-if warnings on three nested ifs — merged with let-chains
  (edition 2024 makes these idiomatic).

## Verification

```
cargo fmt --all -- --check              # clean
cargo clippy --workspace --all-targets  # zero warnings
cargo test --workspace                  # 182 passed, 0 failed (was 169)
cargo run -q -p epsilon-cli -- --demo   # completes
cargo check -p epsilon-cli --features real
```
