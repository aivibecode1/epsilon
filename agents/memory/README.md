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

- **IN FLIGHT (round 55): the review's findings F1–F7.** A delegated agent is answering the seven findings
  from `audit/evidence/m126-m127-review.md` (the footer's false 60-column claim, the card-open precedence
  implemented twice, the two glyph-table entries with no reader, the duplicated doc lines in `theme.rs`,
  the `zz_*`/`filler` author artefacts, doc hygiene, and `fg_of`'s `None`-vs-`Reset`). Files in play:
  `ui/footer.rs`, `ui/header.rs`, `ui/layout.rs`, `ui/theme.rs`, `app.rs`, `app/golden.rs`,
  `app/tests.rs`, `CHANGELOG.md`.
  **Last measured: `307 passed / 1 failed`, and the failure is the agent's OWN new guard** —
  `app::tests::the_card_open_rule_is_borrowed_and_not_restated`, the test it wrote for F2, failing before
  the fix lands (TDD order), **not a regression**: the count rose from 302 to 307, so nothing that passed
  before is broken. `cargo clippy -p epsilon-tui --all-targets -- -D warnings` is clean.
  **Do not commit until the crate is green**, then battery + reinstall (the footer and the theme are
  user-visible). Earlier in the slice the agent left a probe at `crates/epsilon-tui/tests/zz_f1_probe.rs`
  and removed it again — check no scratch file remains before staging, and stage **named files**.
- **M129 CLOSED** (battery `m129c`: all five gates exit 0, **1,165 passed / 0 failed / 1 ignored across
  61 binaries**; installed; docs and history row landed). The slice landed
  the audit's last two items and the review's B1:
  - **OSC 52 for `/copy`** (`crates/epsilon-tui/src/clipboard.rs`, new). Native first, terminal second;
    a local terminal with a working clipboard deliberately does **not** emit the sequence, because a
    redundant one races the native write for the same clipboard. `ESC ] 52 ; c ; <base64> BEL`, wrapped
    in the tmux/screen passthrough envelope and **only** the wrapped form; refused rather than truncated
    past pi's `MAX_ENCODED = 100_000`; parked on `App::osc_pending` and written through
    `terminal.backend_mut()` **after** `terminal.draw` returns, so it never lands inside a frame. The
    notice says **"sent via the terminal"** rather than "copied", because OSC 52 is a request: a
    terminal that ignores it (iTerm2 ships it off) says nothing, and claiming otherwise is exactly the
    audit's defect.
  - **`NO_COLOR` glyph separation**, and a **false claim made true**: `theme.rs:744` promised "widgets
    that rely on a band for legibility must fall back to a gutter glyph when `has_surfaces()` is false"
    and **no such branch existed** — all seven `LineKind`s returned the same glyph in every mode, so an
    answer and its reasoning trace were the same pixels. The old test passed because it asserted a
    hardcoded `" "`/`"|"` rather than distinctness. Now `assistant`/`thinking` have their own glyphs in
    both repertoires plus the fallback branch: Unicode `* answer` / `≡ reasoning`, ASCII `*` / `=`.
  - **B1's card duration** (my fix): `settle_call` returns the pending card's `started_at_ms`.
  - **`base64`**: already a workspace dependency at `=0.23.1` for M43, so the direct edge cost one line
    in `Cargo.lock` and no new build — which makes the acceptance decision cheaper than it looked.
  - **`scratch_dump_tests.rs` retired**: it did *not* own a shared helper (that was
    `tests_support::render_rows`, in its own module); deleting the file and its `mod` was enough.
  `cargo test -p epsilon-tui` **300 passed / 0 failed / 0 ignored**; `cargo check -p epsilon-cli` and
  `cargo clippy -p epsilon-tui --all-targets -- -D warnings` both clean.
  **Still owed: the `epsilon.exe` reinstall** (`/copy` and the transcript glyphs are user-visible), the
  M129 milestone doc and history row, and blockers **B2–B5**.
- **`m129b`'s single test failure was NOT a flake and NOT environment-sensitive — it was my battery
  reading the tree while the delegated author had that test's own negative control armed** (it had
  reverted `decide()`'s trigger in place from `remote || !local` to `!local`). The test reads no
  environment at all: `remote` is a **parameter** of `decide()`, which is exactly why the native result
  and the remote flag are parameters — every branch is reachable on every machine. **A control is
  indistinguishable from a bug to anyone reading the tree while it is armed**; that is a hazard of the
  negative-control method this repo mandates, and the defence is that controls stay short-lived and
  announced. The slice landed: OSC 52 for `/copy` with
  `a_failed_terminal_write_is_not_reported_as_a_copy`, `NO_COLOR` glyph separation for the remaining
  roles, the B1 card-duration fix, `base64` accepted as a workspace dependency (reason in the commit),
  and the delegated author's load-bearing `scratch_dump_tests.rs` retired (it owned a helper other tests
  called, so it could not simply be excluded). `cargo test -p epsilon-tui` is **299 passed / 0 failed**
  and clippy is clean on the crate.
  **`m129b`'s single test failure was NOT a flake and NOT environment-sensitive — it was my battery
  reading the tree while the delegated author had that test's own negative control armed** (it had
  reverted `decide()`'s trigger in place from `remote || !local` to `!local`). The test reads no
  environment at all: `remote` is a **parameter** of `decide()`, which is exactly why the native result
  and the remote flag are parameters — every branch is reachable on every machine. The only environment
  readers in the module are `is_remote()` and `Multiplexer::detect()`, called from `app.rs` at the
  `/copy` call site. **A control is indistinguishable from a bug to anyone reading the tree while it is
  armed**; that is a hazard of the negative-control method this repo mandates, and the defence is that
  controls stay short-lived. Corrected here so no later round chases the phantom.
  **The scratch module was also mis-described:** it did *not* own a shared helper — its helper was
  `tests_support::render_rows`, which lives in its own module — so deleting the file and its `mod`
  declaration was enough. The note that it "could not simply be excluded" was wrong, and the round-49
  build break was caused by removing the declaration while the file still held a *different* re-export.
  **Do not treat M129 as verified, and do not reinstate `target/` for it** — the reinstall is also
  still owed (`/copy` and the transcript glyphs are user-visible).
- **Blockers B2–B5 from `audit/evidence/m126-m127-review.md` are still open** (the light palette's
  ANSI-16 quiet roles are `DarkGray`, the same as dark, under a comment forbidding it; `EPSILON_THEME`
  with a bad value falls through to the config file instead of dark; `parse_diff`'s doc states the
  opposite of its pinned test; the split diff at ≥120 columns has never been drawn by a test).
  **The m127a reconciliation flake those notes used to describe is fixed**: M128 (`c63858e`) gives each
  run a per-process directory, so two concurrent runs cannot eat each other's fixtures, and
  `EPSILON_RECONCILE_DIR` still overrides. Battery `m128a` passed — all five gates, 1,148 passed /
  0 failed / 2 ignored across 61 binaries — and the independent review the M126/M127 slices owed is
  delivered at `audit/evidence/m126-m127-review.md`.
- **M126 CLOSED (`933837b`, `9ae64a2`) — tool cards and a real diff, battery `m126d` PASSES:** all
  five gates exit 0, **1,131 passed / 0 failed / 1 ignored across 61 binaries** (fifteen more than
  M125's 1,116); `epsilon-tui` 267 tests. Record: `docs/milestones/M126-tool-cards-and-diffs.md`. The
  one stubborn assertion was **the fixture's arithmetic, not the product**: its "short cards" hold a
  three-line diff against a twelve-row cap, so nothing is hidden and capping them gives back zero rows
  — the old `all_open_rows - 4` could never hold. Lesson recorded there: three wrong theories over four
  rounds (wrong question → refuted cache lead → print the numbers), so **print before theorising**.
  **Owed: the independent read-only review this slice has never had.**
- **M125 (`5070781`) closed TUI audit items 3, 4 and 5**, battery **m125a PASSES**: all five gates
  exit 0, **1,116 passed / 0 failed / 1 ignored across 61 binaries** (`audit/evidence/gate-m125a.log`),
  eighteen more than M124's 1,098. The footer's hints are now **derived from the live binding
  table** (the old literal advertised `? help`, which is not a bound key, and hid PageUp/Down,
  history, Ctrl+O), the false `need 60×12` message formats from `MIN_WIDTH`/`MIN_HEIGHT` instead of
  contradicting them, the header's context number is the **session context** — the same number the
  compaction trigger reads — instead of the last call's prompt size (which "agrees on turn 1 and
  diverges in the flattering direction afterwards"), and the approval dialog is ~6 rows with the
  danger border its comment had always claimed. Record: `docs/milestones/M125-tui-honesty.md`.
  **Owed: it had NO author negative controls and no independent review** (the delegated author never
  reported; the parent verified by running the suite and reading the diff), and the reinstall.
- **TUI audit items still open** (in the audit's order): tool cards with a real coloured diff (a
  diff is uncoloured markdown prose behind a global toggle today), a light theme (`Color::White` is
  body text), a frozen-view signal + jump-to-live (scrolling up during a run is silent, `G` does
  nothing, no scroll position indicator), `NO_COLOR` glyph separation for the remaining roles, and
  OSC 52 for `/copy` over SSH.
- **M124 (`6202b3a`) closed the hang**, and battery `m124a` **PASSES**: all five gates exit 0,
  **1,098 passed / 0 failed / 1 ignored across 61 binaries** (`audit/evidence/gate-m124a.log`),
  covering M122 + M123 + M124 together. The `serve`-mode gap below is the next F07 item.
- **TUI REDESIGN IS THE OWNER'S STATED PRIORITY** (2026-09-23): *"The TUI is extremely poor in
  terms of design and usability compared to the references."* The audit is
  `audit/evidence/tui-design-audit-2026-09-23.md` — **read it before touching `epsilon-tui`**. Its
  headline: `ui::transcript::decorate`/`row_style` (gutter, `▎`, per-role glyphs, filled bands)
  had **no production caller**, so the design existed, was tested, and was never drawn.
  **M123 (`2024ef4`) fixed its items 1 and 2**: `RenderCache` carries each row's `LineKind`
  (`RenderedLine.kind` + `visible_rows_with_kinds`), `draw_transcript` decorates every visible row,
  the doubled `❯ > ` prefix is gone, and the transcript rule was cut (rules 2 → 1; 80×24
  transcript 19 → 20 rows). Its tests read **cells from the real draw path** (`render_buffer` →
  `draw`) — the property the old suite lacked — with four negative controls restored
  byte-identically and touched. **Remaining, in the audit's order:** (3) footer honesty — it
  advertises `? help`, which is not a bound key, and hides PageUp/Down, history, Ctrl+O;
  (4) a context percentage that is the last prompt's size rendered as window occupancy;
  (5) approval dialog 11 rows → ~6 and the danger border its comment already claims; then tool
  cards with a real coloured diff, a light theme (`Color::White` is body text today), jump-to-live
  + scroll position, `NO_COLOR` glyph separation, and OSC 52.
- **M122 (`15c6198`) landed F07**: the turn loop no longer discards durable-write failures. Written
  by a delegated agent that went **inactive without reporting**, so it carries no inventory of the
  other discarded store results, no author negative controls and no independent review; the parent
  fixed three clippy errors and a formatting failure it never saw. Treat its list of sites as
  **incomplete** until someone greps again.
- **RESOLVED and CLOSED (M124, `6202b3a`): the battery hang is diagnosed and fixed.**
  `an_aborted_turn_lost_to_the_store_refuses_the_next_request` hung because its fault arithmetic
  was written for the *pre-F07* loop: `set_write_fault_for_tests(.., 1)` fails the **first** append
  — the prompt — and since F07 that failure returns out of `Agent::run` before the provider is
  called, so the test's `entered_rx.wait_for(..)` (no timeout) waited forever on a watch that could
  never flip. The third argument is a COUNT of failures, not an occurrence, so the seam could not
  express "fail the second append" at all — that is why the obvious fix did not exist. Fix: the
  seam gained an **occurrence** argument
  (`crates/epsilon-session/src/store.rs`: `set_write_fault_for_tests(op, msg, occurrence, times)`,
  plus `appends()` / `write_fault_pending_for_tests()`), the salvage test arms occurrence 2 and
  asserts its own sequence, it signals on the **delta** so the abort lands on the arm that keeps
  partial text, and every wait on the agent under test in `loop_scenarios.rs` is now bounded
  (`run_bounded`, 30s; 10s entry/delta waits). Negative controls: old arithmetic → fails in 10.0s
  naming the missing provider entry; timeout removed → hangs past 45s.
  **Battery m124a now PASSES**: all five gates exit 0, 1,098 passed / 0 failed / 1 ignored across
  61 binaries (`audit/evidence/gate-m124a.log`), covering M122 + M123 + M124 together. The
  `epsilon.exe` reinstall was started on those bytes. `docs/user/sessions.md` was corrected in the
  same commit.
  **Still open:** the `audit/`/defect-ledger F07 rows still describe the pre-fix swallow (only the
  user doc was fixed); **`serve` mode silently discards a failed-persist turn** —
  `crates/epsilon-cli/src/main.rs:1982` `let _ = agent.run(message)` with nothing subscribed to
  notices — which is exactly the F07 acceptance surface and deserves its own slice; and
  `pi_import_reconciliation::pi_import_carries_every_source_entry` failed once in a combined run
  and passed on every re-run (unexplained flake, worth a second look).
- **Unrelated dirty work — deliberately NOT staged:** `.agents/skills/ratatui-tui/**` (6 files) was
  modified by something other than this task. Preserve it; do not fold it into a milestone commit.
- **Rounds closed 2026-09-22/23 (tenth through thirteenth)** — M116 (`6462a44`): the
   preserved-entry notice is printed by both frontends, and the `model_change` arm stopped
   writing `provider: ""` when the field was absent. M117 (`4e292a7`): `find` announces the
   results its cap hides and `ls` gained pi's `limit`. M118 (`0d2110c`): the advertised byte
   caps are real (50 KiB, pi's `DEFAULT_MAX_BYTES`) and `glob`'s walk bound is announced.
   **M119 (`8c306c3` + `0d0333f`): the pi v2 importer — `epsilon --import <pi-file>` writes
   `<file>.epsilon.jsonl` beside the source and never touches it; verified on a real pi file
   (1018 entries, source byte-identical, the copy resumes).** Batteries `m116a` (1,053),
   `m117a` (1,057), `m118b` (1,066), `m119a` (1,077 across 60 binaries). Global `epsilon.exe`
   reinstalled and exercised.
- **Two process lessons from M119, both worth more than the code:**
   (a) a delegated agent **failed without reporting** and its work was still good — assess the
   tree rather than the report, and run the battery on it; but say in the record that the slice
   had no review. (b) **The first M119 commit missed two untracked files** (`import.rs`, the
   CLI test) because it staged explicit paths: the tree built and the battery passed, so the
   evidence was valid, but the commit alone would not have compiled. Run
   `git status --short --untracked-files=all` **before** staging after any delegated work.
- **Two follow-ups queued by earlier rounds landed 2026-09-23** (focused tests green:
  `cargo test -p epsilon-session -p epsilon-tools -p epsilon-contracts`; the round owner's battery
  is the evidence of record). (a) **pi's `thinkingSignature` has a slot now**:
  `ContentBlock::Thinking.signature`, optional and skipped when absent — a signature-less block is
  byte-identical to what this build wrote before, pinned by a test on the serialized bytes — and
  carried in `store.rs`'s `legacy_block`, the one content-part conversion both the v1 healer and
  the pi importer use, so there is no second path. The reconciliation test stopped reporting
  "no slot for it" (that sentence would now be the report lying) and compares signature values,
  asserting the source counts (1 in large-session, 49 in before-compaction = 47 non-empty + 2 that
  pi wrote empty). (b) **`glob` carries `find`'s 50 KiB cap**: same helper, `find`'s notice, count
  reports rows shown, and the two notices are mutually exclusive — the walk notice wins when the
  walk bound fired, because "N more match, use a narrower pattern" is false about files never
  visited (the cap is still applied there).
- **M119's unexplained number is settled (2026-09-23): the importer loses nothing — the count
   came from a file `--resume` had already repaired.** A real reconciliation
   (`crates/epsilon-session/tests/pi_import_reconciliation.rs`: serde_json over every line of both
   files, per `type` and `kind.type`, per role, with the active branch rebuilt from `parentId`)
   gives source 1,018 = imported 1,018 entries with **0 unmatched, 0 abandoned** on
   `large-session.jsonl`, and 1,002 = 1,002 on `before-compaction.jsonl`. The 932 / 391 / 1,037
   numbers are the *post-resume* file: `--resume` repairs before it prints, appending 18 synthetic
   tool results (`details.repair = "dangling_tool_call"`) for the fixture's 18 dangling tool calls
   plus one `turn_end` — 1,018 + 19 = 1,037, and 914 messages + 18 = 932. The CLI says it itself
   ("repaired …: 18 tool call(s) had no result"); the old artefact was counting that repaired file
   against the import's pre-repair report. Evidence:
   `audit/evidence/pi-import-reconciliation-2026-09-23/`. One real loss was found, a **field** and
   not an entry: `thinkingSignature` on thinking parts (1 entry in large-session, 47 in
   before-compaction) had no slot in `ContentBlock::Thinking` — **carried since the follow-up
   above** (`signature`), so that evidence log is history: it records what the pre-fix code did.
- **G19's one remaining item is a behaviour change, not a bug:** conversion still happens
   *in place*, so the original is kept rather than the conversion avoided. The importer plan's
   import-as-copy rule would make `--resume` produce a new file and leave the original
   untouched — a real UX question (which file am I in now?) that should be decided, not
   drifted into.
- **Nothing CONSUMES the typed model-switch entries.** M116 made `model_change` and
   `thinking_level_change` heal faithfully (a provider-less line falls back to `Custom` rather
   than lying with `""`), and both are typed, durable and reportable — but `model_view()` and
   `restore_history` ignore them, so a pi session's model switch still does not change the
   resumed session's model. **The consumer is the missing half**; it was deliberately not
   invented. Note for the next agent: the queue item that said "map these" was stale for two
   rounds — check `git blame` before writing code against a note.
- **G20's two real defects are closed** (M117, `4e292a7`): `find` announces the results its cap
   hides and that its byte cap exists, and `ls` takes pi's `limit` and reports it. The rest of
   G20 — Epsilon registering twelve tools where pi's default active set is four — is a
   **deliberate product divergence**, recorded in `audit/11` so nobody "fixes" it by
   removing capability to match a count.
- **The pi v2 importer (slice 4B)** — pi files whose entries carry `id`/`parentId` are
   refused at line 2 (`UnknownLineShape`); the v1 path now heals correctly. Plan:
   `audit/evidence/session-format-gap-plan.md`, which also argues import-as-copy.
- **SESSION-01b**: the branch/fork store operations exist and no user path calls them —
   `/tree` is an alias for the active-branch picker, `/fork` is a raw `fs::copy`.
- **The skills collision policy is wrong in shipped code**: `skills.rs` says its later-root
   shadowing "mirrors pi"; pi keeps the FIRST name and reports a collision diagnostic. Needs a
   design decision (which root wins, what the diagnostic says), not just a patch.
   shadowing "mirrors pi"; pi keeps the FIRST name and reports a collision diagnostic.
- **F07**: `persist` and `pop_last_message` discard their errors, so a failed durable write
   leaves memory and the model view divergent with no signal.
- **The 30-second lockout after a kill** (`crates/epsilon-session/src/lease.rs`): reproduced
   with the console probe; the fix is a design decision because a pid liveness check wants a
   platform call.
- **A replaced approval dialog is still silent** (`app/event.rs:191-197`), batched with an
   ask FIFO so it costs one battery.
- **`label`/`session_info` heal into `Custom`** — preserved, not applied: no pi fixture shows their v1 spelling, and both carry id references a positional v1 line may not resolve.
    a pi session's model switch has no effect when resumed. Small mapping (`provider`/
    `modelId`, `level`), deliberately left rather than half-done.
- **Then the port's remaining gaps: G2 (resolved containment) and the
    `audit/03-port-plan.md` parity rows the re-check ranked.
- **The maintained-doc rewrite is still incomplete: `docs/history.md` is a 57 KB timeline read
    by nobody, and `docs/README.md` needs to be the map a newcomer actually follows.






