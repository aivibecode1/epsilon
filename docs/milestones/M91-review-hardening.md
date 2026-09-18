# M91 — review hardening (the M89/M90 follow-ups)

## What

The follow-ups that the independent reviews of M89 (turn hooks) and M90 (session tree)
recorded as owed, closed in one slice. Every item was already itemised in the repo; this is
the record of where each one was written down and what closed it.

**A. Session-reader robustness** (`crates/epsilon-session/src/store.rs` + its integration
test). Both gaps are hand-corruption paths, not reachable from normal writes: no store
append can produce either shape (parents are ids of existing entries; `seq` is the file
index by construction). They were recorded in M90's limits ("the parent-chain walks
(`branch_entries`, `branch_to`) have no cycle guard — a hand-corrupted file with a parent
cycle loops forever (pi's own walk is equally unbounded), and `branch_entries` indexes by
`seq` position, so a hand-edited file whose `seq` values do not match line order can
panic"), as M90 review finding 6, and queued in `agents/memory/next-hardening.md`
("Queued from M90's review").

- **A1 — cycle guards.** `branch_entries` and `branch_to` walked `parent_id` chains
  unbounded. Both now keep a visited set:
  - `branch_entries` (the tolerant active-leaf walk that backs `model_view` and
    navigation) stops at the repeat and returns the entries it legitimately reached —
    each once, oldest first — the same tolerant behaviour it already had for a chain that
    ends on a missing parent. The model view degrades to a shorter branch instead of
    hanging the process.
  - `branch_to` (the fork's exact ancestry walk) refuses with the `InvalidData` kind it
    already gives a dangling chain, and the message names the entry the chain re-entered
    ("its parent chain contains a cycle through {id}") — a cycle can never reach a root,
    and a partial path written out as a session file would be a silent lie.
  Test: `a_parent_cycle_terminates_the_tolerant_walk_and_is_refused_by_the_fork`
  (hand-written 2-cycle — a's parent is `b`, `b`'s is `a` — both behaviours, and the
  refusal happens before the destination directory is created).
- **A2 — `seq`-indexed lookup.** `branch_entries` resolved each id through the entry's
  STORED `seq` (`by_pos`), then indexed the parsed vector with it: an out-of-range panic
  or, worse, a silently wrong branch when the position lands in bounds but on another
  entry. The walk now resolves the position with `all.get(pos)` and requires the entry
  there to be the one the chain named; anything else is treated as the same broken chain
  and stops the walk (the tolerant behaviour above — documented at both the guard and the
  function). Test:
  `a_seq_that_disagrees_with_line_order_is_a_chain_break_not_a_panic` covers both
  shapes: a `seq` past the end of the file (the panic path) and a `seq` that resolves
  in-bounds to a different entry (the wrong-branch path), plus that the fork — which
  resolves by id, not by `seq` — still reaches the real root, so the guard is scoped to
  the seq-indexed walk.

**B. The three M89 test gaps** (`crates/epsilon-core/tests/loop_scenarios.rs`). M89's
limits say: "Recorded test gaps (re-review, `audit/evidence/m89-review.md`): the
drain-point mirror and the overflow-discard pop are correct by inspection but not
exercised *with an active replacement*" and "The epoch is also bumped when the store is
attached or detached mid-run (a source swap must invalidate a run-scoped replacement);
that consistency fix is correct by construction and is not regression-tested, because no
in-tree caller attaches a store mid-run today." Restated in
`agents/memory/next-hardening.md` ("Recorded follow-ups from M89"). All three are offline
`FauxProvider` tests in the existing CORE-02 style (`RecordingTurnHooks`,
`recorded_requests`, the real loop):

- **B1 — the drain-point mirror** (`a_drain_point_message_reaches_an_installed_replacement`):
  a tool queues a steering message during turn one; the replacement is installed at that
  turn's end; the message is injected at the next drain point; the next recorded request
  (built from the replacement) must carry it.
- **B2 — the overflow-discard pop**
  (`an_overflow_retry_discards_the_failed_attempt_from_transcript_and_replacement`): the
  replacement is live when turn two comes back with a context-overflow-shaped
  `StopReason::Error`; the retry's discard and recovery commit; the post-retry request
  must match the transcript and carry nothing of the discarded attempt.
- **B3 — the attach/detach epoch bump** (`attaching_a_store_mid_run_invalidates_the_replacement`,
  `detaching_the_store_mid_run_invalidates_the_replacement`): with a replacement
  installed, attaching a fresh session store (or detaching the attached one) during a
  turn must invalidate the replacement — the next request is rebuilt from the new source
  (the newly attached store's own view; runtime history after a detach), not from the
  stale snapshot.
- The one harness piece that did not exist was a mid-run source swap, so the slice adds
  `SwapStoreDuringTurn` (a late-bound tool that attaches a given store, or detaches).

**C. Anchor precision after a replacement — deliberately NOT implemented.** M89's limits
record: "a request built from a replacement clears the usage anchor instead of re-basing
it to the runtime history length, so the pressure trigger falls back to the whole-history
estimate for the rest of such a run … Re-basing is a recorded follow-up"
(`docs/milestones/M89-turn-hooks.md`'s Limits; the queue entry is
`agents/memory/next-hardening.md`'s M89 follow-ups bullet — the M89 *review* recorded the
clear-instead-of-re-base as its fix, not as the follow-up). The re-base was implemented as far
as its shape allows and then deliberately left in the clear-instead-of-re-base form: the
change is larger than it looks, and the naive version is unsafe. See Key decisions.

## Key decisions

- **The tolerant walk stops; the exact walk refuses.** The two walks have different
  contracts and the guards keep them: `branch_entries` already tolerates a chain that
  ends early (a transcript that lost its beginning is still worth showing), and a cycle
  or a `seq` mismatch is one more way the chain is broken, so it stops and returns what
  it reached. `branch_to` already refuses a dangling chain as `InvalidData` because a
  fork of a partial path would misrepresent the branch, so a cycle refuses the same way —
  in the same vocabulary, with the cycle named in the message. Neither choice adds a new
  error kind or changes any well-formed read.
- **`seq` mismatch is a chain break, not an error.** The alternative (a `StoreError`) was
  available; the tolerant stop was chosen because the walk's caller (`model_view`,
  navigation) is a read path whose existing contract is "degrade, don't fail" and because
  an error would be indistinguishable at the API level from the corruption cases that are
  already handled by stopping. The exact consumer (fork) does not use this walk at all.
- **Nothing about well-formed files changes.** For a file the store wrote, the entry at
  the `seq`-derived position IS the entry the chain names and no chain repeats, so every
  guard is inert: the existing 25 integration tests in `session_store` and the whole
  `epsilon-core` suite are the regression cover for that ("keep behaviour byte-identical"
  — the guards fire only on the two hand-corruption triggers, named in each doc comment).
- **B3's mid-run attach is expressed with the agent's own API** (`attach_session_store` /
  `detach_session_store`, both of which bump the history epoch) rather than a new seam:
  the point of the test is that the *existing* consistency fix works, so it must go
  through the same call a future frontend would.
- **C: the re-base is refused, not fudged.** The anchor's `covered` is an index into
  `RUNTIME HISTORY` (`compact::anchored_tokens` computes `history[covered..]`), while a
  replacement-built request sent the replacement's messages. A re-base would have to set
  `covered = runtime_history.len()` at request time — which is *precise* only while the
  run's replacement is still the source the next request reads from. It is not, in three
  places, and each one needs an invalidation the slice cannot express today:
  1. **Across runs**: the replacement dies with the run, the anchor does not. The next
     run's first request is built from history/the store, so `prompt_tokens` (the
     replacement's measured size) + `history[covered..]` omits the whole prefix the new
     request will actually carry — an under-count that delays the pressure compaction.
     Today's clear behaves correctly there (the whole-history estimate is exactly right
     for a whole-history request).
  2. **Mid-run attach/detach**: the epoch bump drops the replacement (and this slice now
     tests that), but neither `attach_session_store` nor `detach_session_store` clears the
     anchor — a re-based anchor would survive a source swap the same way.
  3. **A later replacement**: a hook that re-installs a (pruned) context makes the earlier
     request's measurement describe a different message list; the estimate then
     over-counts (defensive) but is no longer the measurement it claims to be.
  The common cause is the anchor's SHAPE: it carries a source-agnostic index into runtime
  history and no provenance, so it cannot say "only valid while the replacement is the
  request source". A correct fix therefore needs both a new value (`covered` =
  runtime-history length) and new lifetime discipline — e.g. a run-scoped "this anchor
  came from a replacement" flag, cleared when the run ends or the source changes — or a
  `UsageAnchor` that records what it describes, which lives in `crates/epsilon-core/src/
  compact.rs`, outside this slice's file set. Neither is small, and getting it wrong
  reintroduces the class of mis-sizing M89's review called out, so the behaviour is left
  as it is and the follow-up is recorded as still open (Limits). Everything needed to
  finish it is in this section.

## Gotchas

- The cycle guards mean a corrupt file's model view can be SHORTER than the file's entry
  count with no error surfaced — deliberate (the walk is tolerant), and the fork still
  refuses the same file, so the corruption is not silent on the path that must be exact.
- `branch_entries`'s `by_pos` map is still built from stored `seq` values (that is the
  store's own id → position index); the guard makes the *use* of those positions safe,
  it does not make the map itself meaningful for a corrupt file.
- B2's replacement discards are three pops (runtime history + the store, `produced`, the
  replacement). The test pins the joint contract (nothing of the failed attempt reaches
  the provider; the post-retry request matches the transcript); the replacement pop has
  no separate observable, because the same committed compaction that triggers the retry
  invalidates the replacement through the epoch — stated in the test's doc comment rather
  than papered over.
- B2 needs the overflow to happen on a turn AFTER the replacement is installed, or there
  is nothing in the replacement to pop: the script therefore runs a tool turn first, then
  the error step. The summarizer is its own `FauxProvider` so the main script stays
  exactly three steps.
- B3's attach test deliberately lets the mid-run store start EMPTY: each script step is one
  turn, so the store is attached during the turn whose request was the replacement and
  receives what is pushed after it — starting with that turn's own tool result. The
  post-swap request is then that store view, which is distinguishable from both the
  replacement and the full transcript. That is a real consequence of attaching late (the
  store only receives what is pushed after it is attached), not a test artefact.
- The docs in `store.rs` name each guard's exact trigger (cycle / mismatched `seq`) so a
  reader can tell a guard from a behaviour change.
- ADR-006/ADR-007 are untouched: nothing here writes tags, prunes history or reorders the
  file; the guards stop reads.

## Verification

**GATE PASS** on the frozen candidate `m91a` — `audit/evidence/gate-m91a.log`
(sha256 `711e93f6…`): all five gates exit 0 (fmt 2 s, clippy 1298 s, test 665 s, demo 401 s,
real-check 40 s), **772 passed / 0 failed / 0 ignored, 55 summaries**. Per-binary delta against
the M90 baseline (`gate-m90b.log`, 766): `loop_scenarios` 28 → 32 (**+4** — B1, B2, the mid-run
attach and the mid-run detach) and `session_store` 25 → 27 (**+2** — the cycle test and the `seq`
test); every other binary is unchanged, so the delta is exactly the six new tests.

- Candidate: HEAD `ff7e8cc` + the three code files + this document, byte-verified by hash after
  assembly (`CANDIDATE == WORKING TREE`); external target, `CARGO_BUILD_JOBS=1`.
- Focused run on the candidate before the battery: fmt clean, clippy exit 0 (three crates),
  `epsilon-session` 16 + 27, `loop_scenarios` 32 — all exit 0.
- **Negative controls re-run on this round's own copy** (the implementer's controls were on its
  working tree; the frozen candidate re-earned them) — `audit/evidence/m91-probes.log`
  (sha256 `05cc3915…`), four probes armed in a disposable copy:
  - P1 `seq` guard reverted → `a_seq_that_disagrees_with_line_order_is_a_chain_break_not_a_panic`
    fails with `panicked at store.rs:726:29: index out of bounds: the len is 3 but the index is 7`
    — exactly the panic the M90 review named.
  - P2 cycle guard reverted → the test process dies with `0xc0000409 STATUS_STACK_BUFFER_OVERRUN`
    (unbounded chain growth; the implementer, with both guards reverted under its own bound, saw
    a wall-clock hang instead — either signature proves the pre-fix walk does not terminate
    normally).
  - P3 overflow-discard pop reverted → **the suite stays green**, which is why the Limits below
    say the pop is defensive and *not* independently observable rather than "pinned jointly".
  - P4 attach epoch bump reverted → `attaching_a_store_mid_run_invalidates_the_replacement` fails
    at `loop_scenarios.rs:2259` on the assertion that the stale replacement must not survive the
    swap.
- **Independent review** (read-only, no builds) — `audit/evidence/m91-review.md`: all six items
  (A1, A2, B1, B2, B3, C) confirmed; boundary re-verified (candidate = HEAD + exactly the three
  slice files + this doc; `crates/epsilon-tui` byte-identical to HEAD); the review's one
  documentation-only blocker (the earlier "nothing in-tree consumes this" claim) was false and is
  fixed in the Limits above, and its findings are dispositioned in the record. The implementer's
  claimed byte-identity restore of its negative controls was verified here against the live files
  (`store.rs` sha256 `7ff4cd15…`, `session_store.rs` `279cd366…` — exact matches).
- Implementer's own focused run (its working tree, before the frozen round): fmt clean; clippy
  exit 0; `epsilon-session` 16 + 27; `epsilon-core` 200 passed across 15 summaries; its controls
  (seq guard → the named panic; cycle guard → no termination under a 20 s bound) with byte-exact
  restores. Logs kept under `%LOCALAPPDATA%/Temp/epsilon-m91b-logs/`.
- No reinstall is owed: nothing in-tree calls any changed API with new arguments
  (`branch_entries`/`branch_to` keep their signatures; `crates/epsilon-core` and
  `crates/epsilon-tui` call them exactly as before), so the shipped CLI behaves as before.

## Limits

- **C (anchor re-basing) is not implemented** — see the decision above for the exact
  reason and the sketch of a correct fix. The M89 limits line stays true: a request built
  from a replacement clears the anchor, and the pressure trigger falls back to the
  whole-history estimate for the rest of such a run. The follow-up is still open and must
  not be silently dropped; it needs `crates/epsilon-core/src/compact.rs` (the anchor's
  shape) as well as `lib.rs`. One present precision gap belongs in the same follow-up
  (independent review): `attach_session_store`/`detach_session_store` bump the history
  epoch but do **not** clear the anchor, so an anchor measured against the pre-swap source
  keeps feeding the pressure check after the swap — over-counting at worst (defensive), but
  it is the same provenance/lifetime problem a correct re-base must solve.
- The cycle guard is Epsilon hardening, not pi parity: pi's own parent walk is equally
  unbounded (`session-manager.ts`). The parity statement lives in the M90 limits and is
  unchanged by this slice.
- The guards cover hand-corruption only. A file the store writes preserves the
  gapless-`seq`/existing-parent invariants by construction, so no normal path is affected
  — which is also why the existing tests are the right regression cover for
  well-formed reads.
- The other item queued from M90's review — whether a branch summary should survive a
  later compaction the way pi's entry-level window keeps it — is NOT part of this slice
  and remains queued in `agents/memory/next-hardening.md`.
- B2's replacement pop is **not independently observable** in the current shape, and the round's
  probe proved it: reverting the pop (`crates/epsilon-core/src/lib.rs`'s discard block) leaves
  the whole suite green, because the same committed compaction that triggers the retry drops the
  replacement wholesale through the history epoch before any request can read it. The pop stays
  as defensive consistency (if the invalidation ever moves, the two views would otherwise
  disagree by one message), and what B2's test genuinely pins is the *discard* — the transcript,
  the store and the post-retry request. Probe evidence: `audit/evidence/m91-probes.log`.
- The reader hardening is **not** frontend-idle, and the consequence is recorded rather than
  glossed (independent review, this round): `repair_on_open`
  (`crates/epsilon-session/src/repair.rs:95`) runs on every live resume
  (`crates/epsilon-core/src/lib.rs:665`, `crates/epsilon-cli/src/main.rs:1655`) and
  active-branch search (`crates/epsilon-session/src/search.rs:149-150`) — both walk through
  `branch_entries`, and the resume path then pushes the (possibly shorter) view into runtime
  history. So on a hand-corrupted file the new behaviour is: resume with a silently shorter
  transcript, search silently missing entries — where before it was a hang, a panic, or (in one
  stale-state case) a wrong-but-longer branch. That is the intended trade — a suspect file stays
  usable and the file's own entries are intact (`read_all` is unaffected; the data is not lost)
  — but the degradation is **silent by construction**: the walk returns `Ok` with a shorter
  branch and no caller can tell it stopped early. Two recorded follow-ups: make `repair_on_open`
  report a chain that stopped early (its diagnosis vocabulary already exists for this shape), and
  surface it in a frontend. Until then the fork is the only path that refuses loudly — and the
  fork has no in-tree caller yet (SESSION-01b).
- `crates/epsilon-tui/` is untouched, per this work item's hard rule (the tree carries
  another work item's uncommitted changes there); SESSION-01b remains the consumer that
  will surface any of this in a frontend.
