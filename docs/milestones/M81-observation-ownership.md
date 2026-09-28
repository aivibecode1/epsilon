# M81 — Observation ownership (F02)

For contributors maintaining native file-read/write safety. M80 is reserved by the
pre-existing checkpoint implementation; this log does not manufacture its history.

## Change

- ToolContext carries a passive allocation-backed observation token, independent of
  the display session ID. Agent retains it across runs and replaces it when history
  is restored. Task children are new Agents and do not inherit parent observations.
- Read log stores per-token path snapshots. Weak token ownership enables lazy
  cleanup at the next log access after the last owning Agent/context disappears.
- Successful write/edit invalidates sibling observations before refreshing only
  its own snapshot. This catches harness writes even at identical size/mtime.
- Shared mutation locks and lexical path keys remain unchanged. F09 alias handling
  and external file replacement races remain open. A read/stat race is not fixed.
- The existing partial-read policy is unchanged, including the byte-cap exception
  for WriteTool; F01 separately refuses incomplete EditTool source buffers.

## Tests and decisions

The first real-tool regression failed before the fix: B overwrote README using
only A's read. An intermediate session-string key passed distinct-name contexts
but could not distinguish production Agents (all called inline), so it was replaced
with an opaque lifetime token. The final test deliberately gives both contexts the
same display ID. No stateful behavior was added to the contracts crate.

Other tests cover sibling successful-write invalidation, deterministic same-size/
same-mtime changes, reread recovery, partial reads, a failed edit followed by a
targeted edit, weak
scope cleanup, and token stability across calls versus renewal on history restore.
The core test exercises actual preflight/execution with an inert recording tool.

## Verification status

Targeted: all 35 builtin-tools integration tests passed; all 9 read-log unit tests
passed; the core identity/restoration scenario passed. Full battery: fmt, strict
Clippy, demo and real-provider compilation passed. Workspace tests stopped before
wire_fixtures ran: Windows Access is denied (error 5). Two retries failed to link
that same executable with LNK1104. No relevant running process or Defender detection
was found by the checks performed; the cause is now established — see
[the verification log (history)](../../audit/history/05-verification-log.md): Avira Security's
heuristic (`HEUR/APC.Misc`) quarantined the freshly linked wire_fixtures test
executable in this repository's target directory. The full workspace suite
(53 summaries, 786 passed, 0 failed) and all five gates pass under an alternate
`CARGO_TARGET_DIR`; clearing the detection is owner action. No security
configuration was changed.

Scoped `cargo test -p epsilon-tools -p epsilon-core` passed: 294 passed,
0 failed, 0 ignored across 17 summaries. A subsequent compile-only
check for both libraries exited 0. Selected executed output is preserved in
[the verification evidence](../../audit/evidence/f02-verification.txt).
Independent static review approved the implementation without source-level blockers.
The review identified three acceptance-test gaps. The follow-up now covers:

- A failed sibling edit does not upgrade the original reader's partial view:
  another full overwrite is still refused, before the targeted edit succeeds.
- Both single and batch EditTool mutations invalidate a sibling's observations,
  even when length/mtime match; an unrelated path remains writable.
- Real parent/task-child filesystem operations refuse blind child overwrites and
  stale parent writes, recover on reread, and refuse a restored agent's overwrite.
  These composition tests live in epsilon-cli because it already depends on both
  core and tools plus the faux provider; no new dependency edge was introduced.

All 36 builtin-tools tests and both CLI composition tests passed. The final
five-gate round passed formatting, strict Clippy, demo and real-provider check;
workspace tests failed linking epsilon-llm's wire_fixtures executable (LNK1104).
The clean rebuild therefore did not resolve the Windows blocker. Full output is
preserved in [the final gate log](../../audit/evidence/gate-f02-final.log).
The original cross-agent regression also passed from a clean target directory;
see [the isolated run](../../audit/evidence/f02-isolated-clean.log).
No runtime guard changes were needed for the acceptance follow-up. Independent
[follow-up review](../../audit/evidence/f02-acceptance-review.txt) approved with
caveats and confirmed all three gaps closed. The scenarios are sequential, not
a concurrent task scheduling test.

Adding observation_scope is source-breaking for external ToolContext struct-literal
callers; they must supply and retain the token. It adds no serialized wire field.
No full-workspace pass is claimed. Windows event-log checks found no matching
wire_fixtures entry in the queried Defender, CodeIntegrity, AppLocker or Application
logs; that does not establish why Windows denied access.

The earlier round removed 21,059 files (11.2 GiB). After the acceptance follow-up,
cleanup removed another 13,181 files (6.1 GiB); target/ is absent and C: has 63G free
(87% used). No failing baseline was installed. Implementation remains uncommitted
pending resolution of the workspace verification and dirty-baseline commit scope.

## Baseline and commit scope

Several affected files already contain extensive uncommitted implementation changes.
Do not silently include those changes under this fix. The original working files
were captured for a narrow review diff. Runtime behavior changes require ADR-005's
M81 amendment; F02's alias-dependent acceptance remains open with F09.
