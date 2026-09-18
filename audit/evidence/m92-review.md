# Independent reviews — M92 RPC-02 protocol breadth (deleg_598a57b5, deleg_c2ea49d7)

Read-only reviews, no builds. Round 1 found **two blockers** — both real and both fixed before the
commit — plus nine findings. The focused re-review then confirmed the fixes with **no blockers**,
independently reproduced the four-line `epsilon-contracts/src/lib.rs` footprint, and recorded five
further non-blocking items.

## Round 1 — verdicts and the two blockers

Verdicts on the nine verification items: envelope + backward compatibility PASS (with caveats),
correlation PASS, the extracted dispatch path PASS (single dispatch site; HEAD's inline match
gone, not duplicated), new ops PARTIAL (B1), lagged frame PASS, HTTP routes/wiring PASS but the
drift-test **claim** FAIL (B2), tests real with named weak ones, Limits honest except one false
claim, pi fidelity PASS at envelope level.

**B1 — the "pi's seven levels verbatim" claim was false.** `ThinkingLevel::XHigh` serializes as
`x_high` (snake_case rename; the session-persistence id pinned by the crate's own test) while pi
spells it `xhigh` (`references/pi-0.84.2/packages/agent/src/types.ts:300`). A pi-shaped
`set_thinking_level` therefore failed with `unknown variant 'xhigh'`, both `get_state` surfaces
emitted `x_high`, and the OpenAPI enum declared `xhigh` — a value the server could never emit
(schemas are not covered by the drift test).

*Fixed:* `#[serde(alias = "xhigh")]` on `XHigh` (input-only; serialization unchanged, so stored
sessions keep parsing), a contracts test pinning both spellings + the canonical output, an
end-to-end RPC test driving pi's spelling, and an OpenAPI enum derived from the same serde path
`/state` uses (`ThinkingLevel::ALL` → serialized), pinned against what is emitted and excluding
the alias. The doc's claim is corrected.

**B2 — "spec paths == registered routes" was not enforced.** The drift test compared
`spec()["paths"]` against a hardcoded 9-name list and never read `router()`, so adding a route
without a spec entry failed nothing (pre-existing since M12; M92's docs restated it as enforced).

*Fixed:* `route_table()` in `epsilon-server/src/lib.rs` is now the only registration site
(`router()` folds it; repo-wide grep finds one `.route(` call, no `.nest`/`.fallback`), and the
test compares the spec's path **set** against that table. `/openapi.json` — mounted at HEAD but
absent from both the spec and the old list — is now documented in both, the only spec addition and
a real route. What the guard still does not cover (methods, handlers, params, schemas) is stated
in the doc, the ADR, and the module comment.

## Round 1 findings — dispositions

| # | Finding | Disposition |
|---|---|---|
| 1 | `pending_message_count` narrows pi's meaning (steering only; pi sums steering + follow-ups; core has no followups getter) | **Recorded** in Limits with the pi citation (the child also corrected the pi file path: it is `packages/coding-agent/src/core/agent-session.ts:1528-1531`) |
| 2 | The outbound `{"event":…}` wrapper missing from the interop list — the one divergence a pi client hits on every event | **Recorded** in Limits |
| 3 | `SessionState`'s header implied a subset of pi's fields while adding `estimated_context_tokens` | **Fixed** (header reworded: "not a subset") |
| 4 | Missing tests: both-tags frame, `/abort` 409 with `request_id`, unknown op with extra realistic payload fields | **Fixed** — all three added and judged discriminating in the re-review |
| 5 | Weakly discriminating tests (set_thinking_level echo ambiguity; lagged notice compared to a duplicated literal; a tautological get_state assert) | **Fixed** — the level test now starts at a different level than it sets; the lagged notice is single-sourced through `LaggedNotice` used by both surfaces; the tautology is gone |
| 6 | Abort tests' timing assumption (~400 ms SlowProvider margin) unrecorded | **Recorded** as a residual (a proper fix needs a provider the faux harness cannot hold open) |
| 7 | Scope accounting: the slice's doc footprint includes this round's ADR-011 amendment and F13 sync | **Confirmed** — both are in the commit (one M92-only hunk each, no sibling content) |
| 8 | `id` shares its name with `approve`'s ask id | **Comment added** at both code sites |
| 9 | Weak test: `get_state` mapping-pinning for `message_count`/`estimated_context_tokens` (no stable literal exists) | **Accepted** — mapping-pinning noted, the flagged model tautology is gone |

## Round 2 (focused re-review) — no blockers

Confirmed: B1 closed with discriminating tests (removing the alias fails them; the `x_high` reply
separates read-back from echo); B2 closed (route table is the only registration site; the old
hardcoded list is gone; the test fails for a table-only path, a spec-only path, and — proven by
the fix round and re-proven by this round's probe F — a scratch route); the shared `LaggedNotice`
is types-only and used by both surfaces with serve's bytes unchanged against HEAD (RPC's frame is
byte-identical to the slice's own first cut; HEAD emitted nothing there); all three added tests and
both strengthened tests are discriminating; the doc corrections are accurate with a verified pi
citation; hygiene clean (`epsilon-tui/**` and AGENTS.md untouched, no new dependencies, no
`unsafe`, nothing staged); and the contracts footprint is exactly four lines, reproduced
independently by rebuilding HEAD + those lines and comparing bytes (numstat 10/3 = sibling 6/3 +
slice 4/0, hunks non-overlapping).

| # | Round-2 finding | Disposition |
|---|---|---|
| F1 | The doc's Gotchas bullet said "two lines … 6/3 before, 8/3 after", 37 lines — stale after the fix round | **Fixed** — now four lines, `10/3`, with the reconstruction and the staging-by-hash consequence recorded |
| F2 | The fix round's scratch-route negative control had no persisted log | **Closed** — this round's probe F produces it (`audit/evidence/m92b-probes.log`) |
| F3 | Both the OpenAPI enum and its pin derive from the hand-maintained `ThinkingLevel::ALL`; a variant added only to the enum would be omitted from both | **Recorded** in the milestone's Limits, with `as_str()`'s exhaustive match named as the compiler-side mitigation |
| F4 | A vacuous sibling-token assert in the round's throwaway script (`AfterCallToolCall`) | Noted; the script is scratch, its four other token checks and the byte comparisons were real |
| F5 | In the get_state test, `message_count`/`estimated_context_tokens` are compared against the same getters that build them | Accepted as mapping-pinning (no stable literal available) |
