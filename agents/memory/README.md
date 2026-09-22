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
| [next-hardening.md](next-hardening.md) | Machine procedure, traps that cost real time, and the open queue | reference before editing |
| [metrics-and-reader-honesty.md](metrics-and-reader-honesty.md) | The visibility numbers, the session reader, the `@` list, and the stall behind it (M106–M110) | **closed**: cache gauge fixed, dead footer deleted, F08 refused-not-truncated, `@` shares the ignore answer, and M109's per-keystroke stall fixed in M110; batteries `m106a`/`m109a`/`m110a` |
| [tui-redesign.md](tui-redesign.md) | TUI presentation layer | **split done**; the owner's "shrink and freeze" measured on 2026-09-21 — no shrink, typing works, `scripts/console-probe.py` is the conhost harness. The `@`-ignore gap it recorded is closed (M109) |
| [session-store-integrity.md](session-store-integrity.md) | Session-store repair + entry cache | **closed**: committed `497093e`, battery `m105d` 1,023 passed / 0 failed across 59 binaries; evidence copied to `audit/evidence/gate-m105d.log` |
| [f09-path-policy.md](f09-path-policy.md) | F09 filesystem identity | M84 scheduling slice closed (d0ac16b); resolved containment OPEN, next design task |
| [surface05-paged-reads.md](surface05-paged-reads.md) | SURFACE-05 paged reads | **landed** 27cd677 with its evidence logs tracked; full workspace suite green on a fresh test profile; note can be retired once the milestone doc's reinstall line is reconciled |
| [port-revalidation.md](port-revalidation.md) | Autonomous audit, restructure and complete pi port | audit 13773b4; … M101 df338b7; **2026-09-21: M96–M100 and M102–M104 dev logs reconstructed, battery `m105c`, console probe** |

## Next safe action

1. **Rounds closed 2026-09-22 (eighth and ninth)** — M114 (`d54dfb8`) healed pi's messages;
   **M115 (`5326b2f`) closed the other two parts of G19**: `SessionHeader` keeps the fields
   this build does not model (`provider`/`modelId`/`thinkingLevel`/`branchedFrom`), and
   `persist_migration` copies the previous bytes to `<file>.pre-epsilon` once — never
   overwriting — with both CLI call sites naming the path. Batteries `m114a` (1,043) and
   `m115b` (1,049, `m115a` failed clippy on a collapsible `if` and was stopped). Verified on
   pi's own fixture through the installed binary; global `epsilon.exe` reinstalled; disk
   cleaned to 32.2 GB.
2. **G19's one remaining item is a behaviour change, not a bug:** conversion still happens
   *in place*, so the original is kept rather than the conversion avoided. The importer plan's
   import-as-copy rule would make `--resume` produce a new file and leave the original
   untouched — a real UX question (which file am I in now?) that should be decided, not
   drifted into.
3. **Nothing CONSUMES the typed model-switch entries.** M116 made `model_change` and
   `thinking_level_change` heal faithfully (a provider-less line falls back to `Custom` rather
   than lying with `""`), and both are typed, durable and reportable — but `model_view()` and
   `restore_history` ignore them, so a pi session's model switch still does not change the
   resumed session's model. **The consumer is the missing half**; it was deliberately not
   invented. Note for the next agent: the queue item that said "map these" was stale for two
   rounds — check `git blame` before writing code against a note.
4. **The pi v2 importer (slice 4B)** — pi files whose entries carry `id`/`parentId` are
   refused at line 2 (`UnknownLineShape`); the v1 path now heals correctly. Plan:
   `audit/evidence/session-format-gap-plan.md`, which also argues import-as-copy.
5. **SESSION-01b**: the branch/fork store operations exist and no user path calls them —
   `/tree` is an alias for the active-branch picker, `/fork` is a raw `fs::copy`.
6. **The skills collision policy is wrong in shipped code**: `skills.rs` says its later-root
   shadowing "mirrors pi"; pi keeps the FIRST name and reports a collision diagnostic.
7. **F07**: `persist` and `pop_last_message` discard their errors, so a failed durable write
   leaves memory and the model view divergent with no signal.
8. **The 30-second lockout after a kill** (`crates/epsilon-session/src/lease.rs`): reproduced
   with the console probe; the fix is a design decision because a pid liveness check wants a
   platform call.
9. **A replaced approval dialog is still silent** (`app/event.rs:191-197`), batched with an
   ask FIFO so it costs one battery.
10. **`label`/`session_info` heal into `Custom`** — preserved, not applied: no pi fixture shows their v1 spelling, and both carry id references a positional v1 line may not resolve.
    a pi session's model switch has no effect when resumed. Small mapping (`provider`/
    `modelId`, `level`), deliberately left rather than half-done.
11. Then the port's remaining gaps: G2 (resolved containment) and the
    `audit/03-port-plan.md` parity rows the re-check ranked.
12. The maintained-doc rewrite is still incomplete: `docs/history.md` is a 57 KB timeline read
    by nobody, and `docs/README.md` needs to be the map a newcomer actually follows.






