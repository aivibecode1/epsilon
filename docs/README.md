# Documentation

For users and contributors finding current guides, design intent and historical records.
The implementation is the authority when documents disagree; older design and milestone
claims are not proof that a capability works today.

## Product documentation

| File | What lives there | Read it when |
|---|---|---|
| [`../README.md`](../README.md) | the entry point: what Epsilon is, install, quick start | first |
| [`user/`](user) | product guides: install, configuration, auth, permissions, sessions, troubleshooting | you are using or configuring Epsilon |
| [`../CHANGELOG.md`](../CHANGELOG.md) | user-visible changes, milestone by milestone, newest first | you want to know what changed |

## Contributor documentation

Agent recovery state lives in [`agents/memory/`](../agents/memory/README.md):
curated open-work notes, evidence pointers and safe resumption steps, not product docs.

| Folder / file | What lives there | Read it when |
|---|---|---|
| [`../AGENTS.md`](../AGENTS.md) | the rules for anyone writing code here: gates, conventions, disk hygiene | before your first change |
| [`design/`](design) | **how and why**: the architecture tour, the ADR log, the skills design | you are changing behaviour |
| [`qa/`](qa) | design conversations turned into notes: why Rust, the brick methodology, the harness primer, TUI keymaps, OAuth client ids, provider extension points, the repository layout | you need the convention behind a choice |
| [`plans/README.md`](plans/README.md) | historical plans and their recorded statuses, not fresh authorization | you want to trace earlier scope |
| [`../audit/`](../audit) | the dated assessment: usability review, defect ledger F00–F19, port plan, verification log, reference comparison, roadmap | you want to know what is broken or what to do next |

## Historical record

| Folder / file | What lives there | Read it when |
|---|---|---|
| [`history.md`](history.md) | the milestone timeline and acceptance notes | you want the project's story in one table |
| [`milestones/`](milestones) | one dev log per milestone: what was built, decisions, bugs found, verification output | you are touching an area and want its history |
| [`research/`](research) | source-level studies of the reference harnesses and the gap analyses (dated evidence, not current status) | you are deciding what to build next |
| [`handoff/`](handoff) | archived handoff prompts from earlier milestones (historical, not active tasks) | you are tracing how a slice of work was delegated |
| [`glossary.md`](glossary.md) | the vocabulary — harness, turn, tool call, compaction, lease, … | you hit an unfamiliar term |
| `../references/` | the four reference harnesses' sources (not committed; research input) | you want to check a port against its origin |

## Reading paths

**New to coding agents?** [`qa/06-agent-harness-primer.md`](qa/06-agent-harness-primer.md) with
[`glossary.md`](glossary.md) open beside it, then the
[architecture guide](design/epsilon-architecture.md) — which now carries a dated header
saying which parts were verified when, so read a claim there as a hypothesis to check.

**New to the codebase?** Root README → [`design/epsilon-architecture.md`](design/epsilon-architecture.md)
→ the crate that owns what you are touching → its milestone log in `milestones/` → the tests in
that crate.

**Reviewing a change?** `AGENTS.md` (gates), then the ADR log
([`design/epsilon-decisions.md`](design/epsilon-decisions.md)) for the behaviour it changes,
then the milestone log that introduces it.

**Deciding what to build next?** [`../audit/02-defect-ledger.md`](../audit/02-defect-ledger.md) —
every row now carries a status (OPEN / CLOSED / CLOSED with a named residual / FIXED pending
verification), and the status is only as good as the milestone log it names —
[`../audit/03-port-plan.md`](../audit/03-port-plan.md) and
[`../audit/04-roadmap.md`](../audit/04-roadmap.md), then the relevant study in `research/`.

## Current hardening work

Newest first. Each entry is a two-line pointer; the linked log has the reasoning, the
decisions and what the slice could not do. The logs marked *(reconstructed)* were
written after the fact from the commits, because those milestones landed without one —
they say so themselves, and their `## Verification` sections name what could not be
recovered rather than inventing it.

**This list is not the timeline.** Every milestone has a row in [`history.md`](history.md) except
M80 — whose checkpoint work is recorded inside [`M81`](milestones/M81-observation-ownership.md)
rather than reconstructed — and almost every one has a log in [`milestones/`](milestones). Where
no log was written the pointer below goes to the timeline row, which is then the whole record:
M0, M29–M32, M120, M122–M124 and M128. This list is the subset a reader is most likely to want
first, newest at the top, and it is kept current from M83 forward.

- [M133 — `serve` mode stops discarding a run it refused](milestones/M133-serve-mode-reports.md):
  the last place a durable-write failure was still silent. The server's boot task did
  `let _ = agent.run(message).await;` with nothing subscribed to the agent's notices, so the error
  and core's own `Notice` both vanished. **The protocol genuinely cannot carry it** — a `Notice` is a
  broadcast event, not a replay log, and the boot task starts before the listener binds — so the
  server's log is the right channel rather than a second one being invented.
- [M131 + M132 — the review's seven findings, and the auth lock that reported a permission error](milestones/M131-M132-review-findings-and-auth-lock.md):
  a footer comment claiming a survival order the code did not have (and the **measurement** that
  contradicted the review's own account of it), one convention implemented twice where an agreement
  test provably cannot see the duplication, two glyph-table entries with no reader anywhere, and a
  lock that reported `Access is denied` for a transient Windows sharing violation — wrong twice over,
  since it is neither a permission failure nor permanent.
- [M130 — the review's four remaining blockers, and the production bug the third one hid](milestones/M130-review-blockers.md):
  the light palette's ANSI-16 quiet roles were the dark palette's `DarkGray` under a comment forbidding
  it — in the mode that is the *default* on a console that does not advertise itself, which is the user
  `--light` exists for; an unrecognised `EPSILON_THEME` fell through to the config file rather than to
  dark; `parse_diff`'s doc contradicted its own pinned test; and the split diff at ≥120 columns had
  never been drawn by a test, which is how the two halves were found landing on top of each other.
  Answers [`audit/evidence/m126-m127-review.md`](../audit/evidence/m126-m127-review.md).
- [M129 — OSC 52 for `/copy`, the glyph fallback that was only promised, and a real card duration](milestones/M129-osc52-and-glyph-fallback.md):
  over SSH, `/copy` printed a success notice while copying nothing; `theme.rs` promised a band-less
  gutter fallback that no branch implemented, so under `NO_COLOR` an answer and its reasoning were the
  same pixels; and every card reported `0ms` because the result stamped its start and end with the
  same `now`.
- [M128 — each reconciliation run gets its own directory](history.md):
  the test that failed once and passed on every re-run had a cause: a **fixed shared path**, so two
  runs at once renamed and deleted each other's fixtures.
- [M127 — a light theme, and a transcript that says when it is frozen](milestones/M127-light-theme-and-frozen-view.md):
  `Color::White` as body text is the one thing this project's own standard forbids, and on a light
  terminal the prose was near-invisible. A selectable light palette (a switch, never background
  sniffing) plus the frozen-view signal: a position badge and a jump-to-live key bound through the same
  table as its hint, so the two cannot disagree.
- [M126 — tool cards, a real diff, and four rounds spent on one assertion](milestones/M126-tool-cards-and-diffs.md):
  a tool call was one flat line and an edit's diff was prose — the artefact a coding agent shows most
  often, rendered unreadable. Cards with a row-budgeted body, unified below a named width and split
  above it, line numbers only above a column budget, and colour never the only signal. The stubborn
  assertion turned out to be the *fixture's* arithmetic, not the product's.
- [M125 — an honest footer, a true context number, and an approval box that fits](milestones/M125-tui-honesty.md):
  the footer advertised `? help`, which is not a bound key at all — it is text the composer parses
  into a command, so the hint named a key that did nothing until the reader pressed it *and* Enter —
  while PageUp/PageDown, history and Ctrl+O went unmentioned. Hints are now resolved from the **live
  binding table**, so a dead hint is not representable. The header showed the last call's prompt size
  as window occupancy — a true number and the wrong one, agreeing on turn 1 and diverging flatteringly
  after — and now shows the session context the compaction trigger itself reads.
- [M124 — the test that hung for ever, and the seam that could not express the fault](history.md):
  the battery stopped on a test that ran past 420 seconds. `set_write_fault_for_tests`'s third
  argument was a **count of failures, not an occurrence**, so it could not say "fail the second
  append"; and since M122 a failed prompt persist ends the run before the provider is entered, so the
  test waited for ever on a watch that could never flip. The seam gained an occurrence, the test now
  asserts its own sequence, and five joins are bounded so a wait that cannot resolve fails instead of
  hanging a round.
- [M123 — the transcript decoration existed, was tested, and had never been drawn](history.md):
  `ui::transcript::decorate` and `row_style` — the two-column gutter, the user marker, the per-role
  glyphs, the filled bands — had **no production caller**, only tests in the same file, while the real
  draw path emitted plain per-kind text through `RenderCache`. The suite reported decoration as
  covered and the user saw none of it. Tests for anything visual now read **cells from the real draw
  path**, the standard M127 and M129 were then held to.
- [M122 — the turn loop stops discarding durable-write failures](history.md): F07 — two store
  mutations threw their results away, so a failed durable write left memory and the model view
  silently divergent: the loop believed a turn was recorded while the transcript the next request is
  built from did not contain it. Both now report, so the user learns the session may be missing a
  turn. Written by a delegated author that never reported, so its inventory of the other discarded
  sites is incomplete rather than exhaustive.
- [M121 — carry pi's thinking signature, and cap `glob`'s output](milestones/M121-thinking-signature-and-glob-cap.md):
  `ContentBlock::Thinking` had no slot for pi's `thinkingSignature`, so 49 occurrences in one fixture
  and 1 in another were dropped on import; it now rides the **one** content-part conversion both
  readers use, which is why the importer needed no change — the payoff of M119 reusing the healing
  path instead of writing a second converter. `glob` gained the 50 KiB cap M118 left it, and decides
  the opposite way from `find` about which bound speaks: when the **walk** stopped, the byte notice's
  claims are false about files it never visited.
- [M120 — reconcile an import against its source, and settle M119's stray number](history.md):
  M119's evidence left one figure unexplained (373 tool results in the source, 391 in the import) and
  said so rather than smoothing it over, which is what made the follow-up possible. A real
  reconciliation proved a fresh import is **entry-for-entry identical** to its source — 1,018 =
  1,018, zero unmatched in either direction — and that the stray figures came from a file `--resume`
  had already repaired: it appends 18 synthetic tool results plus a `TurnEnd` before printing its
  summary. The lesson outlives the number: `--resume` mutates the session it resumes.
- [M119 — import pi's current session files](milestones/M119-pi-v2-import.md): a *current* pi file,
  whose entries carry the `id`/`parentId` envelope pi's own `migrateV1ToV2` adds, was refused at line
  2 — M114/M115 had made only the v1 files readable. `epsilon --import <pi-file>` converts one into
  `<file>.epsilon.jsonl` **beside** the source, leaving the source byte-identical, and the refusal it
  replaces now prints the exact command to run: a refusal a user cannot act on is half a feature. One
  converter, not two — it reuses the v1 healing path, because a second converter is how the two drift
  and one of them starts emptying messages again.
- [M118 — make the advertised bounds real, and announce the silent one](milestones/M118-tool-output-bounds.md):
  one rule applied three times: a tool must not have a bound it does not announce, and must not
  announce a bound it does not have. `find`'s description promised "1000 results or 64KB" and enforced
  neither byte — the cap is now real (50 KiB, pi's own `DEFAULT_MAX_BYTES`) and the description says
  50KB; `glob` stopped at the walker's budget in silence; `ls` gained pi's byte cap. Each notice names
  **only the bound that actually fired**, because sending a reader to raise a limit that is not what
  stopped them is worse than saying nothing.
- [M117 — `find` announces what it withholds, and `ls` gains pi's limit](milestones/M117-find-and-ls-bounds.md):
  the two *defects* inside G20's divergence, separated from the one deliberate choice (this harness's
  active tool set is its own). `find` stopped at its cap and returned a list that looked complete; it
  now appends pi's notice with the count of what is hidden. `ls` had no bound it could report and now
  takes pi's `limit` (default 500) and says so. Both notices are silent when nothing was withheld — a
  notice that fires when nothing is hidden trains the reader to ignore it.
- [M116 — preservation became loud, and a typed field stopped lying](milestones/M116-preserved-notice-and-typed-model-change.md):
  the notice `preserved_unknown()` had always been able to produce was printed by nobody, so a session
  with a foreign entry opened with a quiet gap and no explanation; both frontends now say it in one
  sentence owned by the store. The same round found the `model_change` arm was half-typed: the guard
  required only `modelId`, so a provider-less line healed to `ModelChange { provider: "" }` — an empty
  string in a typed field, which looks like data. Both fields must now be strings, or the line heals to
  `Custom`.
- [M115 — converting an older session keeps its header and its previous bytes](milestones/M115-header-and-backup.md):
  the other two halves of G19. A rewrite used to drop every header field this build does not
  name (`provider`, `modelId`, `thinkingLevel`, `branchedFrom`) — the M113 preservation rule,
  one layer up — and it rewrote the user's file with no way back. Now the header keeps what it
  does not model, and the previous bytes are copied to a `.pre-epsilon` sibling once, never
  overwritten, with the path named in the message.
- [M114 — the healer that read pi's messages and emptied them](milestones/M114-pi-legacy-healing.md):
  the migration path that makes pi's session files readable read `message.content` as a
  string while pi writes an **array of parts**, so every message came back empty, every role
  became a *user* turn, and every ISO timestamp became "now". Found by running it: resuming
  pi's own fixture reported success and turned a 668-character prompt into nothing. Now roles,
  parts and both timestamp spellings are read, and anything unmappable is preserved.
- [M113 — an entry kind this build does not know is kept, not used to close the session](milestones/M113-unknown-kinds-preserved.md):
  a session file carrying a `kind.type` from a newer build used to be **refused whole** — one
  foreign entry cost the entire conversation. It is now parsed as an opaque object, replayed
  on the next write, and left out of the model view: kept, not applied. The rule is lenient
  about the tag and strict about the body, because preserving a *known* kind with an
  unreadable body would open the session with a silent gap instead.
- [M112 — the cost model gets a shape, its dead twin leaves, and the header gets a legend](milestones/M112-cost-shape-and-legend.md):
  `CostBreakdown` returns the four components with the unscaled products private, so the
  total divides once (a refactor had quietly restored the four-truncations bug, and a test
  caught it); a model whose every rate is zero was about to print `$0+` and no longer does;
  the dead `CostMicro` type left `epsilon-contracts`; and `/help` now explains what the
  header's figures — including its `?`, `—` and `+` — actually mean.
- [M111 — the cost estimate prices what it was leaving out](milestones/M111-cost-accuracy.md):
  the header's cost figure added up two of the four disjoint token buckets a provider
  reports, so cache reads and cache writes were charged at **zero** — not cheap, free, in
  the one number a user checks to see what they are spending. The catalog now carries both
  cache prices (regenerated from models.dev), the arithmetic lives in a tested module
  instead of the draw path, and a session that moved tokens the registry cannot price
  reports a floor (`$0.42+`) rather than a total.
- [M110 — the `@` popup went blank, and the search behind it read everything](milestones/M110-at-popup-stall.md):
  the fix for a regression M109 shipped, found by the console probe. `@` completion loaded
  the ignore rules once per keystroke, and loading them searched gitignored trees too — in
  this repository, 404 MB of reference checkouts per character. One- and two-character
  queries worked; five or six showed the composer's *placeholder*, i.e. the app had not
  drawn since before the keys arrived. Fixed by caching the rules in the frontend and by
  pruning the search in the matcher, which makes every tool call cheaper as well.
- [M109 — `@` completion stops offering files the tools would refuse](milestones/M109-at-file-ignore.md):
  the `@` list had its own walker and its own hardcoded skips, so it disagreed with every
  read tool about which files exist. It now shares `epsilon_core::ignore`, and the slice
  found that the hidden-directory "escape hatch" both walks promised was dead logic —
  `is_ignored` cannot tell "re-included" from "never mentioned", so both now say so.
- [M108 — a complete line this build cannot read is refused, not deleted](milestones/M108-unknown-final-line.md):
  the session store dropped a final line it could not parse. That is right for a crash
  fragment and wrong for a *complete* line in an unknown shape — the first is debris, the
  second is the user's newest turn or a newer Epsilon's entry. Position used to decide
  whether data survived; `is_whole_json` decides now, and a refusal leaves the file
  byte-identical.
- [M107 — deleting the second footer a redesign left behind](milestones/M107-dead-footer.md):
  `epsilon-tui/src/footer.rs` was a whole superseded footer with one live function, and
  `clippy -D warnings` is structurally blind to it because `pub` items in a library crate
  are exempt from `dead_code`. It carried a second `format_tokens` with different rounding
  and a second copy of the cache-formula bug below.
- [M106 — the cache-hit gauge measures one prompt, not a session](milestones/M106-cache-hit-accuracy.md):
  the header divided the session-total cache reads by the latest prompt size, so the
  figure inflated every turn and pinned at `cache 100%`. The test named for the right
  property passed because it modelled the gauge wrongly — the numerator now comes from
  the same call as the denominator.
- [M105 — a repair keeps the session's identity, and a pop keeps its cache](milestones/M105-session-repair-integrity.md):
  two silent session-store defects, both reachable only after something has already gone wrong. A
  repair cut a crash-truncated file back to the last *indexed* entry — zero bytes when nothing was
  indexed — while the in-memory header still answered `Some`, so the next append skipped the header it
  believed was there and the session reopened headerless: fresh ids, no cwd, and for a delegated child
  no parent link. Separately, `pop_last` — which the loop calls for real to discard a message that
  overflowed the window — popped the index but not the read-through entry cache, and because the active
  branch resolves its leaf through those reads, the branch came back **empty**.
- [M104 — a restore verifies the file before overwriting it](milestones/M104-restore-verifies-the-file.md)
  *(reconstructed)*: reverting a checkpoint over a file the user edited after the snapshot
  silently discarded their work. Restore now refuses when the bytes on disk are not the
  bytes that were captured, and says which file and why.
- [M103 — the extension host protocol](milestones/M103-extension-host-protocol.md)
  *(reconstructed)*: `epsilon-ext` went from 5 tests to 30 by driving the real fixture
  process instead of a mock, which found a live defect — the frame cap only bounded a line
  whose newline had not arrived yet, so one oversized line came back as valid.
- [M102 — a summary cut off by the output limit is not committed](milestones/M102-length-stopped-summary.md)
  *(reconstructed)*: a compaction summary that hit the model's output ceiling is a truncated
  summary, not a summary; committing it replaced the transcript with a mutilated version of it.
- [M101 — the context files the harness was already detecting](milestones/M101-context-files.md):
  three ways the harness told the user something untrue. `SYSTEM.md` / `APPEND_SYSTEM.md` raised a
  trust prompt and never loaded; `AGENTS.override.md` was missing from the candidate list *and*
  `doctor` kept a second, shorter list of its own; ancestor context files were never read. All
  three fixed, with the important half being semantics rather than names — **one context file per
  directory, first existing wins**, because adding the override to the old loop would have made it
  additive and it would have stopped overriding. The audit was wrong in two places and this
  milestone records both: `.epsilon/prompts/` is slash templates, not system-prompt text, so it
  stays gated and unloaded; and the `skill` tool it said did not exist has existed since M97.
  Negative control: each fix reverted in place, 9 named tests fail. Landed df338b7.
- [M100 — a revert that cannot restore a file must not delete it](milestones/M100-restore-must-not-delete.md)
  *(reconstructed)*: `restore` read "no captured bytes" as "the file did not exist, so remove
  it", which is also what a *skipped* capture looks like. Deleting a file is the worst outcome a
  revert path can produce, and the entry already carried the field that distinguishes the cases.
- [M99 — `.gitignore` is honoured instead of merely promised](milestones/M99-gitignore.md)
  *(reconstructed)*: `find`'s description claimed it and nothing read one. The matcher lives in
  `epsilon-core::ignore` so the tools and the TUI share one answer — one half of which is still
  not true; see the `@`-file note in [`../audit/11-current-assessment.md`](../audit/11-current-assessment.md).
- [M98 — the six-second stall that read as a hang](milestones/M98-windows-shell-stall.md)
  *(reconstructed)*: every Windows `bash` call ran `powershell -Command`, which loads the user's
  interactive profile first — 6.2 s for a one-word `echo` against 1.06 s with `-NoProfile`. From
  inside the TUI that is indistinguishable from a deadlock, and it is what the owner's "it
  freezes when I send a prompt" actually was.
- [M97 — the `skill` tool the contract always promised](milestones/M97-skill-tool.md)
  *(reconstructed)*: the agent contract told the model to load a skill body "via the `skill`
  tool" and no such tool existed, so the model could see a skill and never open it.
- [M96 — three port gaps the fidelity audit proved](milestones/M96-compaction-and-overflow-fidelity.md)
  *(reconstructed)*: the overflow classifier matched a spaced spelling no provider sends, so the
  compaction-and-retry branch never fired for a real overflow; and the compaction prompt asserted
  a template it never sent, so every summary came back in a different shape.
- [M95 — paged reads](milestones/M95-paged-reads.md): SURFACE-05 — `read` finally pages the way
  its own description promised: pi's offset/limit window over the whole file (`read.ts:275-323`)
  with pi's continuation strings, past-end error, empty-file Ok and the `sed` fallback for an
  over-cap single line, streamed with whole-line boundaries (no U+FFFD cut, no partial line) and
  faithful real line counts. The same change moves the stale-read guard to a coverage model so a
  page of a large file can never grant a wholesale overwrite (F01), and `grep`'s 256 KiB cap is
  announced instead of silent. Focused runs green with a 7-test pre-fix RED; the read-only review
  returned one blocker in the same F01 class (an over-cap line *mid-page* replaced the page and
  recorded `capped`), now fixed — the over-cap branch is gated on the page's first line, with its
  own RED-screened test. Battery `m95b`: 876 passed / 0 failed / 0 ignored; landed 27cd677.
- [M94 — TUI responsiveness](milestones/M94-tui-responsiveness.md): the owner-reported
  unresponsiveness in cmd/pwsh, reproduced in a real console (~30 chars/s echo) and fixed in the
  run loop (drain input before drawing, coalesce agent events, ~30 fps ceiling with a due-frame
  wake, input/approval priority) — measured at ~one frame per burst afterwards; lands the
  in-tree TUI enhancement together with its `config.rs` dependency and the M36 pins.
- [M93 — task state when resuming](milestones/M93-task-state-on-resume.md): F18 — a resumed
  child is claimed `Running` in one lock hold before the background/foreground split, so the
  refusal, the abort cascade and eviction all see it; both run bodies are contained so a panic
  still lands a terminal record. Verified: battery `m93b` 798 passed / 0 failed, two
  negative-control probes (disabling the claim reds seven tests; disabling the containment reds
  both panic tests).

- [M92 — RPC protocol breadth](milestones/M92-rpc-protocol-breadth.md): the RPC-02 slice —
  correlated replies in pi's envelope while keeping the first wire's `ok`/`error` keys, an
  extracted testable dispatch path, `abort`/`get_state`/`set_thinking_level` ops, a lagged frame
  matching serve mode's, `Agent::model_id`, and `POST /abort` + `GET /state` over the shared
  `SessionState` struct with `request_id` echo and OpenAPI registration; verification pending.

- [M91 — review hardening](milestones/M91-review-hardening.md): the follow-ups the M89/M90
  reviews recorded as owed — session-reader guards against hand-corruption (a parent cycle and a
  `seq`/line-order mismatch, neither of which may hang or panic), the three missing CORE-02 loop
  tests (drain-point mirror, overflow-discard pop, mid-run store attach/detach), and a refused-
  with-reason anchor re-base.

- [M90 — session tree](milestones/M90-session-tree.md): the store half of pi's session-tree
  workflows — branch summaries with pi's exact target shapes, explicit no-leaf root navigation,
  `BranchSummary` projected on the active branch, and a fork that writes only the selected
  ancestry with fresh identity; the TUI surface is queued as SESSION-01b.

- [M89 — turn hooks](milestones/M89-turn-hooks.md): pi's `AgentLoopConfig` seams
  (`transform_context`, `convert_to_llm`, `prepare_next_turn`, `should_stop_after_turn`) exist
  as `TurnHooks`; a stop ends the run without draining queues, and a replacement context tracks
  the transcript and yields to a committed compaction.

- [M88 — mid-batch abort](milestones/M88-mid-batch-abort.md): a cancelled run refuses
  batched calls it has not started; one checkpoint covers both batch plans.

- [M87 — admission that reflects reality](milestones/M87-rpc-admission.md): RPC and HTTP
  prompts answer with what actually happened (real rejection, atomic run slot, logged
  in-band refusals).

- [M86 — compaction correctness](milestones/M86-compaction-correctness.md): the durable
  projection replaces the superseded one, and summaries commit only when usable.

- [M85 — follow-up default](milestones/M85-followup-default.md): one-at-a-time
  default restored; explicit All preserved; verification and coverage limits.

- [Fresh audit and port-first plan](../audit/09-revalidation.md): named baseline,
  verified gate evidence, unresolved fidelity gaps and independent review reports.

- [M84 — find directory resource](milestones/M84-find-directory-resource.md):
  narrow scheduling correction; F09 containment remains open.

- [M83 — baseline integration](milestones/M83-baseline-integration.md): isolated
  candidate verification, explicit selection and preserved baseline risks.

- [M81 — observation ownership](milestones/M81-observation-ownership.md): F02 implementation,
  test evidence and remaining verification/alias limits.
- [M82 — plan-mode enforcement](milestones/M82-plan-mode-enforcement.md):
  F10 implementation, red/green evidence and scope limits.

## Conventions

- `qa/` files are numbered sequentially and never renumbered; add the next number.
- `milestones/M<N>-<slug>.md` is written once, when the milestone lands, and is a record — it
  is not edited afterwards except to correct a factual error.
- Current product facts have **one owner**: the guides in `user/`. Milestones, research and
  handoffs are historical records — their status claims describe the moment they were written,
  not today. Current defects and verification evidence live in `../audit/`.

Product reference material lives in `user/`. Existing moves, instruction corrections,
verification limits and the Phase 2 review handoff are recorded in
[the restructure record](../audit/08-restructure-record.md). Those moves predate this
phase; the verification log is not their move map.
