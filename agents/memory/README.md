# Agent Memory

Durable working memory for coding agents operating in this repository. Hermes (and
any other agent) compresses long conversations; these files are the deliberate,
intelligent form of that compression: hand-curated state that survives context
loss, written in the codebase where the work happens.

## Rules

1. **Read before working.** Load the index table below and any file matching your
   task. Trust code, gates, and `git log` over these notes when they disagree.
2. **Update after every slice.** When a work item's status, key decisions,
   evidence locations, or next step changes, update its file in the same turn —
   do not end the turn on stale state.
3. **One file per active work item.** Name files by topic slug
   (`f10-plan-mode-enforcement.md`, not `notes3.md`). Keep each file under ~80
   lines: status, decisions, changed files, evidence paths, next steps.
4. **Working state, not product docs.** Product truth lives in `docs/` and
   `audit/`. These files record *where work stands*: what is verified, what is
   pending, what was decided and why. Never put secrets or credentials here.
5. **Retire only after closeout.** Keep a work item until its documentation,
   verification, installation (when required), and cleanup are complete. Then
   move durable findings to the milestone/ledger and remove the working note.
6. **Checkpoint before context loss.** Before a large handoff or likely context
   compaction, save the current goal, verified facts versus assumptions, decisions,
   blockers, exact evidence paths, running process IDs, dirty-work boundaries,
   and the next safe action. Update existing topic files instead of appending a
   transcript. Read only the index and relevant topics when resuming.

Hermes performs context compression; these files preserve curated recovery state,
not a lossless copy of chat. The active default profile was verified with automatic
compression enabled, a 0.50 threshold and a 0.20 target ratio. Repository notes do
not themselves trigger runtime compression.

## Index

| File | Work item | Status |
|------|-----------|--------|
| [next-hardening.md](next-hardening.md) | Verified baseline and owner-work boundaries | reference before editing |
| [tui-redesign.md](tui-redesign.md) | TUI presentation layer | `ui/` landed (213 tests green, 7a94ccd + 59fdf2e); overlay chrome migrated for all list panels + prompts (uncommitted); `draw_at_file`/`draw_auth_picker` left, `app.rs` split open |
| [f09-path-policy.md](f09-path-policy.md) | F09 filesystem identity | M84 scheduling slice closed (d0ac16b); resolved containment OPEN, next design task |
| [surface05-paged-reads.md](surface05-paged-reads.md) | SURFACE-05 paged reads | **landed** 27cd677; full workspace suite green on a fresh test profile; note retired after the milestone doc's reinstall line is reconciled |
| [port-revalidation.md](port-revalidation.md) | Autonomous audit, restructure and complete pi port | audit 13773b4; M84 d0ac16b; M86-M88 36a226c/35da3d7/31b1794; M89 7e23e6e; M90 e663bd3; M91 fe386d1; M92 a238ce5; M93 5ab723f; M94 1392d37; M95 27cd677; M96 94df984 |

## Next safe action

The TUI is the owner's stated priority and its presentation layer is now a real
module tree under `crates/epsilon-tui/src/ui/`. Two things remain, in this order:

1. **Migrate the eight overlay functions in `app.rs`** (`draw_picker`,
   `draw_session_browser`, `draw_palette`, `draw_timeline`, `draw_at_file`,
   `draw_slash_autocomplete`, `draw_dialog`, `draw_auth_prompt`,
   `draw_auth_picker`) onto `ui::overlay::{Panel, Width, centered, list_row,
   hint_row}`. That is mechanical now, and it is what lets `crate::theme` — the
   compatibility shim — be deleted.
2. **Split `app.rs`** (~7000 lines: event loop + key handling + legacy draw).
   Target shape: `event.rs`, `update.rs`, `keys/` per mode, with the draw path
   reduced to the shell call plus overlays.

Then the checked-in terminal integration test: `.tmp-console/conprobe` (real
conhost) and `.tmp-console/ptyharness` (ConPTY + vt100) both work; they belong
in the repo as a test target rather than in a scratch directory.




