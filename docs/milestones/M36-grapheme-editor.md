# M36 — pi-parity P0 batch 4: grapheme-correct editor (U1)

Fourth execution batch against the 2026-09-03 parity audit. Single target:
audit U1 — the editor addressed everything in `char` indices, so backspace,
forward delete, and cursor movement split ZWJ families, flags, and combining
marks (data corruption in the user's own prompt line).

## What landed

1. **Cluster-boundary primitives** (`editor_ops.rs`): `prev_cluster_start`,
   `next_cluster_end`, `floor_boundary`, `ceil_boundary` over
   `unicode-segmentation` (extended graphemes — pi's `Intl.Segmenter`
   parity). Coordinates stay `char`-based, so undo snapshots, kill-ring math,
   paste markers, and history recall are untouched; only *returned positions*
   snap to boundaries.
2. **`editor_ops` hardening**: `delete_forward` consumes a whole cluster;
   `word_left`/`word_right` snap their punct-step results down/up;
   `line_home`/`line_end` snap off `\r\n` interiors (CRLF is one cluster).
3. **`app.rs` wiring**: backspace erases one cluster via byte-range replace;
   Left/Right and Ctrl+B/Ctrl+F step one cluster per press. Insert stays as-is
   (inserting at a boundary is always safe), yank/kill ranges ride the snapped
   helpers, so every cursor in the system is boundary-pinned by construction.
4. **Dependency**: `unicode-segmentation = "=1.13.3"` pinned in workspace deps
   (same precedent as `unicode-width` in M16a: already in `Cargo.lock`
   transitively, version taken from the lock, no network, no new package).

## Key decisions

- Snap, don't re-coordinate: migrating `cursor_chars` to cluster indices
  would ripple through 5k lines of `app.rs` plus undo/kill-ring/paste-marker
  math for zero user-visible gain over snapping. The invariant that matters
  ("no cursor parks mid-cluster") holds either way.
- `backward → floor, forward → ceil`: word/line targets snap in their
  direction of travel, so repeated presses still make progress and never
  stick.
- `delete_forward` on a (legacy-impossible, defensively handled) mid-cluster
  cursor floors first: it deletes the containing cluster rather than half of
  one.

## Gotchas

- `cargo test --features real` resolves real-only deps (reqwest/rustls tree)
  into `Cargo.lock`: most of this batch's lock diff is that legitimate
  addition, not the one-line `unicode-segmentation` edge.
  `unicode-segmentation` itself stays at the locked 1.13.3 — no duplicate.
- The patch tool's edition-2015 lint noise continues (workspace is edition
  2024); real `cargo check`/`clippy` remain the arbiters.

## Verification

- 2 new `editor_ops` tests: ZWJ family / combining acute / flag movement +
  whole-cluster delete + mid-cluster snapping; word/line snap off cluster
  interiors incl. CRLF.
- `cargo test -p epsilon-tui --lib`: 126/126 (124 pre-existing, incl.
  `ctrl_b_ctrl_f_move_cursor_by_char` on ASCII — behavior identical there).
- `cargo fmt --check`: clean. `cargo clippy -p epsilon-tui --all-targets`:
  zero warnings.
- `cargo check -p epsilon-cli --features real`: green (real-provider path,
  M13 lesson; also validates the lock additions).
