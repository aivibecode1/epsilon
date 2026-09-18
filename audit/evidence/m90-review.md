# Independent review — M90 session-tree slice (deleg_71537fdb)

Read-only review (static; no builds) of the four code files + the milestone doc. Verdicts: all
four claims **CONFIRMED**, with two documentation blockers and ten findings. No code blocker.
The reviewer also independently re-verified the slice's boundaries: 387 HEAD-tracked files
hashed against the candidate (exactly the four slice files differ), `crates/epsilon-tui`
byte-identical to HEAD (the TUI half really is untouched), the contracts change is a `///`
comment only (ADR-017 item 6 held), `lease.rs` untouched, the v1/v2 migration block unchanged,
and ADR-006 holds for all three store operations.

## Doc blockers (both documentation-only; code and tests verified correct)

1. The frozen **candidate's** milestone-doc copy predates the semantics correction: it still
   said `None` means "the leaf where it is" and that pi's null case is a composition. The code
   (`store.rs`: `None → Leaf::None`, `from_id = "root"`, appended as a new root) and the tests
   pin pi's `branchWithSummary(null, …)` exactly, matching the pi source lines the reviewer
   re-read (`session-manager.ts:1391,1395,1397`). **Disposition:** the corrected revision —
   already in the working tree when the review ran — is the one committed; the candidate's doc
   copy was stale, and docs are not part of the compiled artifact.
2. The doc named a test that exists nowhere (`…_without_a_target_attaches_at_the_current_leaf`,
   the pre-rename name). **Fixed** — the doc now names
   `branch_with_summary_maps_pis_target_shapes_exactly`.

## Findings and dispositions

| # | Finding | Disposition |
|---|---|---|
| 1 | Empty branch summary: Epsilon projected `<branch_summary>\n\n</branch_summary>` where pi skips an empty summary (`session-manager.ts:401` truthiness) | **Fixed** — `!summary.is_empty()` guard (pi-faithful: only `""` is falsy) + a test asserting the entry is appended but not projected |
| 2 | A branch summary before a later compaction is dropped, where pi's entry-level window keeps it — the one non-pi-equivalent projection | **Documented** in the milestone limits (with why: Epsilon's retained window is a message list, so a summary can never ride in it) and queued in `agents/memory/next-hardening.md` |
| 3 | The `AlreadyExists` pre-check is practically unreachable (fresh timestamp + `ses_` id) and TOCTOU, while `rename` replaces on Windows | **Documented as best-effort** in the code comment (no contrived test) |
| 4 | Failure debris: a failed rename leaves `<dest>.jsonl.forking`; a failed verification leaves the freshly written file (and possibly its lock) behind | **Fixed** — the temp file is removed on a failed rename, and a verification failure removes the file the call itself created; the doc states the resulting guarantee (nothing that existed before the call is ever touched) |
| 5 | `branch_to`'s `InvalidData` refusal (ancestry not reaching a root) had no test | **Fixed** — a hand-written file with a dangling parent chain must make `fork_branch_at` refuse |
| 6 | Pre-existing robustness: no cycle guard in the parent-chain walks (parity with pi's unbounded walk), and `branch_entries` indexes by `seq` position so a hand-edited file can panic | **Documented** in the limits and queued as reader-hardening (not this slice's job) |
| 7 | Test-quality gaps: degenerate cwd assertion; payloads/timestamps not asserted verbatim; the "reset is in-memory only" property unpinned; the `Some(id)` move was a no-op in the test; the summary's `seq` unasserted | **Fixed** — all five hardened (source header built by hand with a distinct cwd; verbatim payload/timestamp comparison; reset-then-reopen-without-append; a non-tip target so the move is real; `seq` asserted) |
| 8 | `discover.rs`'s comment still described `parent_session` as an id | **Fixed** — one-line wording update |
| 9 | pi's fork also defers the write until an assistant message exists and places the file in the source session dir | **Documented** in the limits (Epsilon always writes into the caller's `dest_dir`) |
| 10 | Boundary re-verification (no issue) | Recorded above |

Reviewer's own summary of what it could not do: no builds or tests were run (read-only
mandate), so its verdicts are static.

## Fix round (deleg_5afe91bd) — all findings applied

Every fix in the table above landed, `store.rs` was restored byte-identically after the
implementer's own controls (md5 `aa2b5c9263878be65bb0be475800ec0f`, re-verified here), and the
focused suite is green on that exact state (fmt, clippy, `epsilon-session` 16 lib + 25
integration, `compaction_scenarios` 10). Two negative controls were executed in the fix round
and reverted: reverting the `!summary.is_empty()` guard makes
`an_empty_branch_summary_is_appended_but_not_projected` fail with the drop signature, and
reverting `branch_to`'s `InvalidData` check makes
`fork_refuses_an_ancestry_that_does_not_reach_a_root` return `Ok` instead of refusing.

Two honesty notes carried into the milestone limits: the `fork_branch_at` cleanup branches are
correct by inspection but not exercised (they need a hostile filesystem state no portable test
can arrange), and the emptily-named `AlreadyExists` pre-check is documented as best-effort
rather than a guarantee.
