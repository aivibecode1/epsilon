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
| [tui-redesign.md](tui-redesign.md) | TUI presentation layer | **split done**; the owner's "shrink and freeze" measured on 2026-09-21 — no shrink, typing works, `scripts/console-probe.py` is the conhost harness. `@`-file completion still ignores `.gitignore` |
| [session-store-integrity.md](session-store-integrity.md) | Session-store repair + entry cache | **fixed in the tree, battery running, uncommitted**: a repair no longer deletes the header, `pop_last` no longer desyncs the read cache |
| [f09-path-policy.md](f09-path-policy.md) | F09 filesystem identity | M84 scheduling slice closed (d0ac16b); resolved containment OPEN, next design task |
| [surface05-paged-reads.md](surface05-paged-reads.md) | SURFACE-05 paged reads | **landed** 27cd677 with its evidence logs tracked; full workspace suite green on a fresh test profile; note can be retired once the milestone doc's reinstall line is reconciled |
| [port-revalidation.md](port-revalidation.md) | Autonomous audit, restructure and complete pi port | audit 13773b4; … M101 df338b7; **2026-09-21: M96–M100 and M102–M104 dev logs reconstructed, battery `m105c`, console probe** |

## Next safe action

1. **Finish the 2026-09-21 round**: battery `m105c` green → commit the session-store fix
   (with its milestone log), the `epsilon-ext` frame-cap test, the reconstructed
   M96–M100/M102–M104 logs, and the tooling (`scripts/gate.ps1`, `scripts/console-probe.py`,
   `audit/evidence/console-probe-2026-09-21.md`). Reinstall the global `epsilon.exe`
   (the store change is CLI-visible), then clean `target/`.
2. **`at_file_rows` shares the ignore answer** (`epsilon_core::ignore`) — one small
   slice that makes `audit/11`'s G5 row true instead of aspirational.
3. **F08's live half**: `reindex` returns `bool`, so an unknown-`kind.type` final line
   is indistinguishable from a torn write and is truncated. A typed tail diagnosis is
   the fix; design in `audit/evidence/g3-g4-checkpoint-restore-design.md`.
4. **F07**: `persist` and `pop_last_message` discard their errors, so a failed durable
   write leaves memory and the model view divergent with no signal.
5. Then the port gaps `audit/11` still lists: G2 (resolved containment, a design task)
   and the remaining `audit/03-port-plan.md` parity rows.
6. The maintained-doc rewrite is still incomplete: `docs/history.md` is a 57 KB
   timeline read by nobody, and `docs/README.md` needs to be the map a newcomer
   actually follows.






