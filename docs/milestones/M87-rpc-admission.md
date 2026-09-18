# M87 — Run admission reflects what actually happened (RPC + serve)

## What

Both frontends used to answer success *before* the run itself decided anything, so the loser
of an overlap received an acknowledgment for work that was rejected a moment later, with
nothing tying the failure back to that reply.

**RPC (`crates/epsilon-cli/src/rpc_mode.rs`).** Every prompt got
`{"ok":{"started":true}}` as long as a helper — which always returned "not busy" — said
nothing was running; the run was then spawned detached and its result discarded. Now
`admit_prompt` answers with the decision the run actually made. `Agent::run` settles
admission before its first await: a rejected run returns `AlreadyRunning` immediately, an
admitted one emits `AgentStart` immediately after the check. Racing the spawned run against
its own first event therefore answers truthfully without waiting for the work to finish (pi
emits success only through prompt preflight, `rpc-mode.ts:394-415`). The reply is `started`
only when `AgentStart` arrived; otherwise it is an error carrying the real reason. A failure
*after* admission is written to stderr instead of being discarded, and the advisory busy
helper is deleted.

**serve (`crates/epsilon-server/src/lib.rs`).** The handler checked `active_runs() > 0` and
then spawned the run — a check-then-act that two overlapping requests can both pass. The
reservation is now atomic: `fetch_add` on the run counter *is* the decision (previous value
non-zero → 409 `another run is already active`, nothing spawned), and the slot is released
by a `Drop` guard inside the run task, so a panicking run cannot leave the server returning
409 to every later prompt.

## Scope note (inherited change)

`rpc_mode.rs` and `epsilon-server/src/lib.rs` each also carry an inherited working-tree
change from the `@`-reference work: `user_message` delegates to
`epsilon_core::references::user_message`, the same constructor `-p` and the TUI use
(main.rs at HEAD states that rule; these two surfaces were the ones not following it). The
revalidation review inspected both as benign and protocol-neutral. They ship here because
the admission fixes sit in the same functions. Nothing else in either file changed.

## Regressions (executed)

| Test | File | Asserts |
|---|---|---|
| `a_second_prompt_is_rejected_while_a_run_is_admitted` | `epsilon-cli/src/rpc_mode.rs` unit tests | first prompt admitted; a second prompt while the run is active is `Rejected` with a reason naming the active run — not a false success; after the run ends, admission is released and a third prompt is admitted |
| `overlapping_prompts_have_exactly_one_admitted` | `epsilon-server/src/lib.rs` unit tests (through the real router) | first prompt 202; overlapping prompt 409 naming the active run; the reservation is released when the aborted run's task ends |

## Verification

Round-B battery (`gate-m86-88f`, candidate `epsilon-m86-green`, ten files byte-verified):
fmt, clippy, test, demo and real-check all exit 0 — **751 passed, 0 failed, 0 ignored
across 55 summaries** (`audit/evidence/gate-m86-88f.log`, sha256 `1ccc939e…`). Both new
tests ran there: `rpc_mode::tests::a_second_prompt_is_rejected_while_a_run_is_admitted`
and `tests::overlapping_prompts_have_exactly_one_admitted`.

No executed RED for either half, for the reasons in the limits below; the pre-fix
behaviour was captured instead — the serve characterization probe
(`red_probe_overlap_is_accepted_by_the_old_check_then_act`) failed against the pre-fix
handler, recording that a *sequential* overlap was already refused
(`another run is already active`) and that only the check-then-act window was open
(`audit/evidence/red-round.log`).

## Limits

- Correlation IDs, abort/state/model operations and dropped-event reporting remain open
  (RPC-02); this commit fixes the admission *reply contract* only.
- The HTTP test proves the reservation path through the router, not a multi-process load
  test; the counter is per-`ServeState`, which is the intended single-run scope. There is
  no executed RED for the serve half: a sequential overlapping pair is also refused by the
  pre-fix pre-check, so the defect is only the check-then-act window, which a deterministic
  single-thread test cannot exhibit (the probe asserting pre-fix acceptance failed and is
  preserved in `audit/evidence/red-round.log`). The atomicity is structural — `fetch_add`
  is the decision — and the regression pins the corrected contract.
- A different frontend writing to the same `Agent` outside this counter would still get
  `AlreadyRunning` in-band — now logged, not silently discarded.
- Independent review (read-only, no builds) returned no blockers. Its RPC finding — an
  unbiased select could report an admitted-then-failed run as *rejected* — is fixed here
  (`biased` with the event arm first); its serve finding (the reservation guard was built
  a statement too late, leaving a panic window) is fixed by constructing the guard
  immediately after the reservation. Neither fix has a dedicated regression: a
  post-admission run *failure* needs a provider whose stream ends with no frame, which the
  faux provider cannot express. Dispositions: `audit/evidence/m86-88-review.md`.
- The reviewer compared the frozen candidate against a working tree that was moving (fmt
  and review fixes landed during review); equality of the committed files with the tested
  candidate is re-verified by hash at commit time and recorded in the verification log.
