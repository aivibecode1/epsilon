# Independent review — M91 review-hardening slice (deleg_3463cc68)

Read-only review (no builds) of the four-file slice. **All six items CONFIRMED** (A1, A2, B1,
B2, B3, C), one documentation-only blocker, and ten findings. The reviewer also re-verified the
slice's boundary independently: the candidate is exactly HEAD `ff7e8cc` with all HEAD files
byte-identical after CRLF normalisation except the three listed slice files plus the new doc;
`crates/epsilon-tui` is byte-identical to HEAD; the diffs are strictly additive.

## Blocker (documentation-only) — FIXED

The milestone's Limits claimed "Nothing in-tree consumes the new store behaviour: no frontend
calls `branch_entries` on a corrupt file." **False**: `repair_on_open`
(`crates/epsilon-session/src/repair.rs:95`) runs on every live resume (`epsilon-core/src/lib.rs:665`,
`epsilon-cli/src/main.rs:1655`) and active-branch search (`epsilon-session/src/search.rs:149-150`)
both walk through the chain, and the resume path pushes the possibly-shorter view into runtime
history. The corrected limits state the real consequence — a hand-corrupted session resumes with
a silently shorter transcript and search silently misses entries (the file's own entries stay
intact; `read_all` is unaffected) — name the silent-by-construction property (the walk returns
`Ok` with a shorter branch and no caller can tell), and record two follow-ups: let
`repair_on_open` report a chain that stopped early, and surface it in a frontend.

## Findings and dispositions

| # | Finding | Disposition |
|---|---|---|
| 1 | The `seq` guard has a non-hand-corruption trigger: `pop_last` truncates the file but never truncates `migrated_entries`, so after open → pop_last → append, `read_entry(n-1)`/`read_all` serve the *popped* entry and the new id check breaks the walk (an empty branch where pre-M91 returned a stale ghost branch). No in-tree caller pops below the open-time count; `SessionStore::pop_last` has **zero** test call sites anywhere | **Recorded follow-up** (queued): fix the `pop_last`/`migrated_entries` bookkeeping and give the store-level pop a test. The guard is not at fault; the stale bookkeeping is pre-existing |
| 2 | B2's replacement pop is not observable — confirmed independently, matching this round's probe (`audit/evidence/m91-probes.log`): reverting `lib.rs`'s `ctx.pop()` leaves the suite green. The Limits sentence overstated it | **Fixed** in the limits + probe evidence; the suggested strengthening (attach a store in B2 so the *store* pop is observable via `read_all`) is **queued** |
| 3 | B2's transcript limb IS genuinely pinned and the overflow path is real, not a lookalike (overflow-shaped stop reason, per-run latch reset, the three pops + committed compaction, post-retry request from source, `history.len() == 5` fails without `pop_last_message`) | Recorded — no action owed |
| 4 | The chain break is silent by construction — no error kind, notice or repair diagnosis reaches any caller | **Part of the blocker fix** (recorded with its follow-ups) |
| 5 | Citation slip: C's framing attributed the quoted M89 follow-up to `m89-review.md` blocker 2, which actually recorded the clear-instead-of-re-base as its fix | **Fixed** — the quote now points at `docs/milestones/M89-turn-hooks.md`'s limits and the `next-hardening.md` queue entry |
| 6 | Live precision gap: `attach_session_store`/`detach_session_store` bump the epoch but do **not** clear the anchor, so a pre-swap anchor keeps feeding the pressure check (over-counting, defensive) | **Recorded** inside the same open follow-up as C (a correct re-base needs the same provenance/lifetime discipline) |
| 7 | Closeout hygiene: tracking docs still listed the closed items as open | **Done in this closeout** — M89 and M90 limits now cross-reference M91; `next-hardening.md` marks both bullets closed and carries the new queue |
| 8 | Evidence owed: no `m91-*` artifact existed in the candidate and the implementer's byte-identity restore was unverified | **Closed** — `audit/evidence/m91-probes.log` (sha256 `05cc3915…`) records this round's four probes, and the restore hashes were verified here against the live files (`store.rs` `7ff4cd15…`, `session_store.rs` `279cd366…` — exact matches) |
| 9 | Test-strength nits: the cycle test asserts "never rewritten" as a length (a hash would match the M90 happy-path standard); a third tolerant-stop trigger (duplicate ids collapsing the position map) should be named with the other two | **Recorded follow-ups** (trivial; no behaviour at stake) |

Two refinements to the implementer's own caveats, both from this round's probes: the cycle
guard's pre-fix signature here was `STATUS_STACK_BUFFER_OVERRUN` (unbounded growth) rather than
the wall-clock hang the implementer observed with both guards reverted — either way the property
holds; and the "pinned only jointly" wording for B2 is now the sharper "not independently
observable".
