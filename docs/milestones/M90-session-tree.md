# M90 — session tree (branch summaries, no-leaf navigation, fork)

## What

The session-layer half of SESSION-01 (`audit/09-revalidation.md`:111; acceptance evidence at
`audit/evidence/revalidation-core-review.md:83-92`, `audit/02-defect-ledger.md:90-104`). The
store could already navigate to any indexed entry, but it could not express three of the four
operations pi's tree needs, and the one vocabulary item that existed had no consumer: the
revalidation record's words are "branch-summary vocabulary has no producer/replay behavior".

The slice, all in `crates/epsilon-session/src/store.rs`:

- **`branch_with_summary(branch_from, summary, usage)`** mirrors pi's `branchWithSummary`
  (`session-manager.ts:1381-1405`) with a one-to-one mapping of pi's two call shapes. `Some(id)`
  is `branchWithSummary(id, …)`: the target is validated BEFORE anything moves — an unknown id
  is refused with the leaf untouched, where pi throws before assigning `leafId` — the leaf moves
  to the target, an `EntryKind::BranchSummary` carrying `from_id = <target>` is appended as that
  target's child, and the leaf advances to the new entry so the next append continues on the new
  branch. `None` is `branchWithSummary(null, …)`: the summary is itself a new ROOT and records
  pi's literal `fromId = "root"`, abandoning whatever leaf existed. A caller that wants the
  summary at the current leaf passes it explicitly
  (`branch_with_summary(store.leaf_id().as_deref(), …)`), which is what pi's callers do with the
  leaf id.
- **No-leaf root navigation.** The private `Leaf` enum replaced the old
  `Option<(String, u64)>` field, so "no leaf" is a state of the store rather than an absent
  value, and `reset_leaf()` mirrors pi's `resetLeaf` (`session-manager.ts:1372-1374`): null the
  leaf, and the next append is a new root (`parent_id = None`). This is the navigation `/tree`
  needs when the chosen entry is the first user message, whose parent is "the beginning of the
  session" — a position, not an id, so `navigate_to(&str)` cannot express it.
- **`BranchSummary` projection on the active branch.** `model_view` no longer drops the entry
  in its catch-all arm. A `ModelViewEntry::BranchSummary { text, from_id, seq }` is produced at
  the entry's place on the branch, and `into_message()` renders it as a user-role context
  message wrapped `<branch_summary>…</branch_summary>` — the same shape as the existing
  `<compaction_summary>` wrapper — so a summary attached at the navigation target appears
  exactly once on the NEW branch and never on the abandoned one. It waits in the same buffer as
  messages, so a later compaction that summarizes it away replaces it exactly as it replaces
  messages (CORE-03's replace rule still holds for compactions alongside it, pi
  `session-manager.ts:418-453`).
- **`fork_branch_at(leaf_id, dest_dir) -> io::Result<PathBuf>`** mirrors pi's
  `createBranchedSession` (`:1412-1491`): a NEW file with a fresh `ses_` id and a
  `CURRENT_SESSION_VERSION` header carrying the source's `cwd` and a `parent_session` link to
  the source file, containing ONLY the entries from root to the chosen leaf — every `id`,
  `parent_id`, timestamp and payload preserved, `seq` renumbered gaplessly 0..n-1 in path
  order (seq is the file index; a gap would read as a missing line). The source file is
  byte-identical when the call returns.

The TUI half of SESSION-01 (all-tree selection, editor-text restore, switching the live
session) is deliberately excluded from this milestone, and `crates/epsilon-tui/` is untouched.

## Key decisions

- **`None` is pi's `null`: the summary attaches at the ROOT.** The receiver is
  `Option<&str>`, and the mapping is one-to-one with pi's `string | null` — `Some(id)` targets
  that entry, `None` starts a new root (pi's `branchFromId ?? "root"`). An earlier framing in
  this slice mapped `None` to "the leaf where it is" and reached pi's null case by composing
  `reset_leaf()`; that made one parameter carry two meanings for no gain, so the mapping was
  made exact and "at the current leaf" is now the explicit
  `branch_with_summary(store.leaf_id().as_deref(), …)`.
- **The reset lives in memory, and reopening is faithful to the file.** pi does not persist
  `leafId` either: its `_buildIndex` sets the leaf to the last entry of the file, and this
  store does the same (`reindex`). A crash between a reset and the new root's append therefore
  reopens at the old tip — the file is the source of truth, and the file was not rewritten
  (ADR-006).
- **Branch summaries are buffered like messages, not pinned like summaries.** In pi's
  entry-level projection a branch summary survives a later compaction if it sits at or after
  `firstKeptEntryId`. Epsilon's retained tail is message-level (the compaction entry stores the
  kept messages themselves), so a branch summary BEFORE a compaction is inside the material
  that compaction replaced, and a branch summary AFTER it is kept and rides in order with the
  messages. Rendering it once, in path position, is what makes the projection deterministic for
  either branch layout.
- **`parent_session` records the source file PATH** (pi stores `previousSessionFile`,
  `session-manager.ts:1441`). The contracts field carries two honest uses, now stated in its
  own doc comment rather than left to the reader: forked headers carry the source file path
  (what pi does, and what a human reading the file needs to find the parent again), while
  M58's delegated children carry a `ses_` id. Contracts are frozen (ADR-017 item 6), so the two
  uses coexist in one string field and the comment was corrected instead of the shape.
- **Fork is a read plus a verified write.** The entries are assembled in memory and written
  through a temporary file + rename (the migration discipline), then the destination is opened
  through the store's own open path (`open_in` reindexes and repairs a torn tail, store.rs
  `open_with_parent`), and its index length must equal the entries written before the path is
  returned. A destination that cannot hold a session is refused untouched (an existing file at
  the computed name is `AlreadyExists`, never truncated); a file that is not on the selected
  path is not visited, let alone pruned.
- **Refusals are io errors in one vocabulary.** `fork_branch_at`'s signature is the brief's
  `std::io::Result<PathBuf>`; an unknown leaf inside it is `NotFound` and a broken ancestry is
  `InvalidData` — the same conditions `navigate_to` reports as `StoreError::Io(NotFound)`, so a
  caller sees one kind of value for "that entry does not exist" on every path.
- **The two wrappers are distinct tags, not one tag with a label.** `<compaction_summary>` and
  `<branch_summary>` keep "history was compacted" apart from "this conversation came back from
  another branch" — pi's distinction (`messages.ts:100-120`), carried in the wrapper because
  Epsilon's provider-bound vocabulary is the contracts `Message`, not a role union.

## Gotchas

- A branch summary is dropped by a LATER compaction on the same path (buffered with the
  messages, so the compaction's `out.clear()` takes it). Nothing is lost that the compaction
  chose to keep: whatever was in its retained window rides in `retained_tail`.
- `fork_branch_at` names the destination `{epoch_millis}_{ses_id}.jsonl` in `dest_dir` and
  creates the directory chain. The pre-check that refuses an existing name (`AlreadyExists`) is
  best-effort, not a guarantee: the name carries a fresh timestamp and a fresh `ses_` id so it
  is practically unreachable, the check is inherently TOCTOU, and the atomic step is the
  rename — which on Windows replaces an existing file rather than refusing. The honest
  guarantee the function does make: an `Err` from it means nothing the call itself created is
  left on disk (temp file removed on a failed write/rename; a just-written destination removed
  when the verification open or index check fails; anything that existed before the call is
  never touched).
- The cleanup branches above are correct by inspection but not exercised by tests: provoking a
  write/rename/verification failure needs a hostile filesystem state (e.g. a directory squatting
  on the temp name after the pre-check), which no portable test can arrange here.
- The fork's `parent_session` is a path (`…\\session.jsonl`), so a picker that prints the
  field verbatim shows a path for forked sessions and a `ses_` id for delegated children —
  see the decision above.
- `branch_to` (the fork's ancestry walk) parses the whole file once, like `branch_entries`:
  correct at navigation/fork frequency, not a hot-loop reader.
- pi's `createBranchedSession` filters `label` entries out of the path and re-chains
  `parentId` so the retained subtree does not orphan. Epsilon has no label entry kind, so the
  path IS the ancestry and the preserved `parent_id`s are the re-chaining. If labels are ever
  added, fork must re-chain that way or it will copy labels and orphan children.
- An unknown id in `branch_with_summary` leaves the leaf exactly where it was; the refusal is
  all-or-nothing by construction (validate, then move, then append), and the test asserts the
  leaf did not move.

## Verification

Round battery `m90b` on the frozen candidate (`epsilon-m90b-candidate` = HEAD `52195d5` + the
five slice files, byte-verified): **all five gates exit 0 — 766 passed, 0 failed, 0 ignored
across 55 summaries** (fmt 1s, clippy 118s, test 816s, demo 96s, real-check 14s;
`audit/evidence/gate-m90b.log`, sha256 `571a4319…`). The test profile was built with
`CARGO_PROFILE_TEST_DEBUG=1` and an external target dir with `CARGO_BUILD_JOBS=1` — this
machine's AV-dodge and one-build rule; neither affects assertions. The seven-test delta over the
previous round reconciles exactly: six in `session_store` (this slice's four plus the fix
round's two) and one in `compaction_scenarios` (the alternate-branch variant below).

The evidence this slice owes, and where it lives:

- `crates/epsilon-session/tests/session_store.rs`:
  `a_branch_summary_lands_once_on_the_new_branch_and_never_on_the_abandoned_one` (allowed-once
  projection, refusal atomicity, reopen);
  `branch_with_summary_maps_pis_target_shapes_exactly` (both of pi's target shapes: `Some(id)`
  at that entry — from a non-tip target, so the move is real — and `None` as a new root with
  `fromId = "root"`);
  `resetting_the_leaf_makes_the_next_append_a_new_root_and_reopens_faithfully` (no-leaf state,
  new root, append-only file, reopen, and the reset-is-in-memory-only property);
  `an_empty_branch_summary_is_appended_but_not_projected` (pi's truthiness rule; the entry lands
  in the file, nothing is projected, a whitespace-only summary still is);
  `fork_refuses_an_ancestry_that_does_not_reach_a_root` (a hand-written dangling chain is
  refused with `InvalidData` before any side effect);
  `fork_branch_at_writes_only_the_selected_ancestry_with_fresh_identity` (fresh id, parent
  link, gapless seq, ids/payloads/timestamps preserved verbatim, the source header's own cwd
  carried over, source byte-identical before/after, destination refusal).
- `crates/epsilon-core/tests/compaction_scenarios.rs`:
  `a_sibling_branch_sees_its_own_summary_and_not_the_other_branchs_compaction` — a real
  faux-provider run compacts on one branch, a branch summary is attached at the root and the
  sibling is continued, the file is reopened, and the FIRST request of a run over the reopened
  store must equal the intended projection exactly (the ledger's missing F04 alternate-branch
  variant).
- Negative control (this slice has no pre-fix RED: the store APIs did not exist, so the tests
  cannot compile against the pre-slice tree): a reverted fix inside a disposable copy of the
  candidate, executed by the author of this milestone. Forcing the projection guard false fails
  `branch_with_summary_maps_pis_target_shapes_exactly`,
  `an_empty_branch_summary_is_appended_but_not_projected` and the loop-shaped sibling test;
  reverting the fork's fresh-id/parent-link/ancestry properties fails
  `fork_branch_at_writes_only_the_selected_ancestry_with_fresh_identity` with "fresh session
  identity" (`audit/evidence/m90-final-probes.log`, sha256 `411d8b31…`; the earlier probe run is
  `m90-negative-control-detail.log`). The implementer's own controls (empty-summary guard;
  `branch_to`'s `InvalidData` refusal) are recorded in `audit/evidence/m90-review.md`, and its
  byte-identical restore was verified here (md5 `aa2b5c92…`).
- Independent review over the frozen bytes: all four claims **CONFIRMED**; two
  documentation-only blockers (both fixed before this commit — the candidate's doc copy
  predated the `None` semantics correction) and ten findings, six fixed in code or tests and
  four documented in the limits above. Records: `audit/evidence/m90-review.md`.
- Attempt history: the first battery (`m90a`) was stopped when the fix round landed, because
  the evidence must cover the shipped bytes.
- No reinstall: nothing in-tree calls the new store APIs yet — SESSION-01b is the consumer — so
  the shipped CLI behaves exactly as before.

## Limits

- Nothing in-tree calls `branch_with_summary`, `reset_leaf` or `fork_branch_at` yet: the
  consumer is the `/tree` and fork UX, which is SESSION-01b. Until it lands, this slice's
  surface is exercised by tests only, and the TUI still shows the active branch alone.
- The all-tree picker (`crates/epsilon-tui`) is untouched by design (the hard rule for this
  work item: the tree carries another work item's uncommitted changes). "Select either sibling
  from the all-tree UI" from the SESSION-01 acceptance is therefore not met by this milestone.
- Fork does not restore editor text and does not switch the running session (pi's runtime-side
  transaction lives in `agent-session-runtime.ts:262-286,317-332`); it returns the new file path
  and writes only that file.
- `seq` renumbering means a forked file's `seq` values do not match the source file's — by
  design (the fork is a new file, `seq` is its own index). Anything that assumed seq stability
  across files would be wrong; nothing does.
- The 90%-budget/nested-summary rules pi applies when *generating* a branch summary
  (`branch-summarization.ts:132-171`) are out of scope: this slice stores and projects a
  summary the caller supplies; producing one (a summarization call over the abandoned path) is
  the runtime half.
- **The one place this projection is not pi-equivalent** (independent review): a branch summary
  that sits BEFORE a later compaction is dropped with the messages the compaction replaced, and
  cannot ride inside the compaction's retained window — that window is a list of *messages*
  (`session_entry.rs`), so a summary can never enter it. pi keeps every entry at or after
  `firstKeptEntryId` and would project the summary it finds there
  (`session-manager.ts:401-403,441-453`). The practical effect: on a branch whose summary is
  followed by a compaction, "this conversation came back from another branch" can disappear
  where pi would keep it. Recorded rather than hidden; changing it would mean a message-level
  retained window that can carry non-message entries.
- Deltas from pi's runtime-side fork (review, for completeness): pi defers writing the branched
  file until the path contains an assistant message and places it in the source session's
  directory (`session-manager.ts:1433,1477-1488`), and switches the running session; Epsilon
  always writes into the caller's `dest_dir` and leaves switching to the caller (SESSION-01b).
- Pre-existing robustness gaps this slice inherits and does not fix (review): the parent-chain
  walks (`branch_entries`, `branch_to`) have no cycle guard — a hand-corrupted file with a
  parent cycle loops forever (pi's own walk is equally unbounded), and `branch_entries` indexes
  by `seq` position, so a hand-edited file whose `seq` values do not match line order can panic.
  The fork preserves the gapless-`seq` invariant by construction; hardening the reader against
  hand-corruption is a separate item (queued in `agents/memory/next-hardening.md`). **Both were
  closed by [M91](M91-review-hardening.md)** — the walks now stop on a repeat and the fork
  refuses a cycle, and a mismatched `seq` is a chain break rather than a panic or a wrong entry.
