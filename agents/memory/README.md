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
| [session-store-integrity.md](session-store-integrity.md) | Session-store repair + entry cache | **closed 2026-09-21**: committed `497093e`, battery `m105d` 1,023 passed / 0 failed across 59 binaries; evidence copied to `audit/evidence/gate-m105d.log` |
| [f09-path-policy.md](f09-path-policy.md) | F09 filesystem identity | M84 scheduling slice closed (d0ac16b); resolved containment OPEN, next design task |
| [surface05-paged-reads.md](surface05-paged-reads.md) | SURFACE-05 paged reads | **landed** 27cd677 with its evidence logs tracked; full workspace suite green on a fresh test profile; note can be retired once the milestone doc's reinstall line is reconciled |
| [port-revalidation.md](port-revalidation.md) | Autonomous audit, restructure and complete pi port | audit 13773b4; … M101 df338b7; **2026-09-21: M96–M100 and M102–M104 dev logs reconstructed, battery `m105c`, console probe** |

## Next safe action

1. **Round closed 2026-09-21** — eight commits, tree clean, `target/` and the scratch
   dirs removed (C: back to 32.7 GB free). The round's commits: `968cbba` the eight
   reconstructed dev logs, `873869a` the Windows battery + conhost probe, `829c3da` the
   audit corrections, `497093e` the M105 session-store fix, `74ed230` the ext frame-cap
   test, `893284f` the evidence + memory closeout, `4d180c7` G16, `6d51196` G17,
   `178c2fe` the installed-binary probe. Battery `m105d` green: 1,023 passed / 0 failed /
   1 ignored across 59 binaries. Global `epsilon.exe` reinstalled (30 min) and probed in
   a real conhost: 100×30 before and after, frame on screen, typed line in the composer.
2. **G16 — the cache-hit gauge divides a session total by one prompt** — top queued
   slice, because the owner asked for this number specifically and it currently pins at
   `cache 100%`. The per-call value is already in hand beside `latest_prompt_tokens`
   (`state.rs:294`/`:430`); carry it in `Metrics` and divide like for like, then rewrite
   `cache_rate_is_about_the_latest_prompt_not_the_session` so it fails against the
   session total. Full finding: `audit/11` G16.
3. **G17 — delete the dead second footer** (`crates/epsilon-tui/src/footer.rs`, all of it
   except `git_branch`), which also removes the duplicate `format_tokens` and the dead
   copy of the G16 formula.
4. **`at_file_rows` shares the ignore answer** (`epsilon_core::ignore`) — one small
   slice that makes `audit/11`'s G5 row true instead of aspirational. `epsilon-tui`
   already depends on `epsilon-core`, so no new edge.
5. **F08's live half**: `reindex` returns `bool`, so an unknown-`kind.type` final line
   is indistinguishable from a torn write and is truncated. A typed tail diagnosis is
   the fix; design in `audit/evidence/g3-g4-checkpoint-restore-design.md`.
6. **F07**: `persist` and `pop_last_message` discard their errors, so a failed durable
   write leaves memory and the model view divergent with no signal.
7. **The 30-second lockout after a kill** (`crates/epsilon-session/src/lease.rs`):
   reproduced with the console probe; the fix is a design decision because a pid
   liveness check wants a platform call and `unsafe` is forbidden outside a reviewed
   override.
8. Then the port gaps `audit/11` still lists: G2 (resolved containment, a design task)
   and the remaining `audit/03-port-plan.md` parity rows.
9. The maintained-doc rewrite is still incomplete: `docs/history.md` is a 57 KB
   timeline read by nobody, and `docs/README.md` needs to be the map a newcomer
   actually follows.






