# M84 — Find declares its search directory

## Change

`FindTool::resource_key` now delegates to the existing search-directory helper,
shared with grep, glob and ls. The helper becomes crate-visible; its behavior is
unchanged. Default and explicit search roots now participate in the planner's
existing directory-prefix conflict checks against writes. Reader concurrency is
unchanged. Declaring the key also feeds the existing touched-directory bookkeeping.

The F09 seam review found this omission: a parallel tool with no key did not
receive the conservative barrier used for sequential tools with unknown reach.
This is a bounded scheduling correction, not resolved-path containment. F09,
link/reparse traversal, alias identity and replacement races remain open.

## Regression and review

The new test covers the default root, normalized explicit spellings, descendant
prefixes and a distinct sibling directory. Before the fix it failed with `None`
instead of the expected directory key; afterward the regression and all 78 tools
library tests passed. Existing core tests cover planner prefix conflicts and
reader concurrency. The new test is not an end-to-end find/write execution test.

Independent static review approved the narrow two-file change, conditional on
recording the full gate result. Review scope and caveats are preserved in
`audit/evidence/f09-find-review.json`.

## Verification

Verified isolated HEAD `80b32eb` plus only `find.rs` and `search.rs` changes:

- Initial battery: formatting, strict Clippy, demo and real check passed; workspace
  tests failed on `auth::tests::concurrent_saves_keep_every_provider`, timing out
  on its temporary lock. Auth source was unchanged from HEAD; cause unproven.
- One unchanged repeat: **all five gates passed; 741 tests passed, 0 failed**
  across 54 summaries. Auth concurrency passed on this run. No ignored tests,
  timeout adjustments or authentication code changes were used.
- Repeat cleanup removed 14,923 files / 8.8 GiB. Candidate target absent.
- No CLI/TUI source changed; no global reinstall is required for this library slice.
  No new containment guarantee is claimed.

## Evidence

`audit/evidence/f09-find-red.log`, `f09-find-green.log`, `gate-f09-find.log`,
`gate-f09-find-repeat.log`, `f09-find-cleanup.log`,
`f09-find-repeat-cleanup.log`, `f09-find-review.json` and
`f09-path-policy-map.md`. Both failed and successful outcomes are retained.
