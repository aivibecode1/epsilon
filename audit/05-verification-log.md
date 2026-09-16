# 05 — Verification record

**Audience:** reviewers checking executed evidence separately from source analysis.
**Date:** 2026-09-16. **Platform:** Windows (win32); Windows edition/version was not verified.
**Baseline:** dirty working tree over `e200815`, including checkpoint source marked M80.
**Cargo:** `C:\Users\Hp\.cargo\bin\cargo.exe`, reported `cargo 1.97.1 (c980f4866 2026-06-30)`.

This document replaces the previous verification log. It does not reproduce or certify the earlier audit's command transcripts. The prior log's successful-gate claims described an older tree and were not accepted as evidence for this audit.

## Commands actually executed

### Wrapper attempt

`bash scripts/gate.sh audit-1` produced no visible console output because the script redirects to its log. Reading `target/gate-audit-1.log` showed:

```
===== fmt --check =====
scripts/gate.sh: line 14: cargo: command not found
[exit=127 0s]
```

The other four steps also reported exit 127; the log ended `GATE FAIL`. Subsequent Bash Cargo checks still failed. This is an environment/PATH problem, not evidence of a Rust compile failure. No shell configuration was changed persistently.

### Direct Windows gate battery

The five commands were then executed sequentially with the absolute Cargo executable. Each exit was retained before the next command; the final captured summary was:

```
GATE_EXITS fmt=0 clippy=101 test=101 demo=0 real-check=0
```

| Command | Exit | Observation |
|---|---|---|
| `cargo fmt --all -- --check` | 0 | No formatting differences reported |
| `cargo clippy --workspace --all-targets -- -D warnings` | 101 | Two diagnostics in checkpoint module |
| `cargo test --workspace` | 101 | Stopped at checkpoint eviction test failure |
| `cargo run -q -p epsilon-cli -- --demo` | 0 | Scripted faux run completed |
| `cargo check -p epsilon-cli --features real` | 0 | Compiled, with unused-import warning |

Complete captured output is local to this audit session:
`C:\Users\Hp\.local\share\opencode\tool-output\tool_0ab5cfc8b001VL7P9727QIvUWt`.
It is not a portable repository artifact. The excerpts and exit codes below preserve the material results without copying credential metadata.

## Strict Clippy failure

The captured output reports:

```
error: unused import: `Hash as _`
   --> crates\epsilon-tools\src\checkpoint.rs:340:21
error: this `if` statement can be collapsed
   --> crates\epsilon-tools\src\checkpoint.rs:312
error: could not compile `epsilon-tools` (lib) due to 2 previous errors
```

The ordinary compile/check paths also report the unused import as a warning. No lint was suppressed and no source fix applied. F00 in [02](02-defect-ledger.md) plans the minimal correction after approval.

## Test failure and coverage limits

```
test checkpoint::tests::the_store_evicts_old_blobs_when_it_grows_past_its_ceiling ... FAILED

panicked at crates\epsilon-tools\src\checkpoint.rs:471:9:
the ceiling must bite
```

The tools unit binary reported **75 passed, 1 failed**. The workspace run stopped; later test binaries and doctests were not established as passing. Do not publish a full-suite total from this partial run.

Representative earlier completed binaries:

| Test binary/group | Observed result |
|---|---|
| CLI library / main / doctor | 25 / 9 / 7 passed |
| Contracts | 22 passed |
| Core unit | 108 passed |
| Core batch planning / compaction / loop / task / verification scenarios | 3 / 6 / 19 / 14 / 5 passed |
| Extension event forwarding / host | 2 / 3 passed |
| LLM unit / OAuth login / wire fixtures | 106 / 27 / 18 passed |
| Live provider smoke | 1 ignored; not executed |
| LSP unit / process end-to-end | 18 / 5 passed |
| MCP unit / fixture binary / connector | 3 / 1 / 2 passed |
| Safety unit / loop rules | 44 / 4 passed |
| Linux bwrap test binary on this Windows host | 0 tests; not Linux validation |
| Server | 8 passed |
| Session unit / store | 16 / 18 passed |

Other core scenario binaries also passed in the captured output. This table is not an exhaustive suite total. The TUI suite was not established as passing before the stop.

**Source-level explanation to test, not a verified fix:** checkpoint `record()` already calls eviction (`crates/epsilon-tools/src/checkpoint.rs:211–212`), while the failing test expects another eviction to remove something (`:462–471`). Preserve bounded storage/newest-preimage invariants when correcting the test; do not delete it merely to get green.

## Offline demo

The demo includes scripted echo calls and an intentional failing tool, followed by a final answer:

```
final reason: Completed
messages produced: 6
tokens in/out: 300/150
```

This establishes that the executable's faux loop completed. It does **not** establish real provider authentication, production file editing or confinement. Normal startup inspected existing configuration and printed provider/credential-availability guidance. The auditor did not open credential-store files, and no credential values are reproduced here.

## Real-provider compilation

The real-feature check finished successfully, with the same unused-import warning. No live API call was requested. Codec tests and compilation do not cover the production chunk-buffering and refresh orchestration failures documented in F03/F16.

## Artifact cleanup

The build directory existed before the audit. The verification round created normal Cargo artifacts and a wrapper log. No CLI/TUI code changed, so no global executable reinstall is required. Cleanup status is recorded below after the round; this document preserves the failures independently of the disposable `target/` log.

Cleanup completed after recording the failures: `cargo clean` reported **16,211 files removed, 9.2 GiB total**. The remaining `target/` directory was removed; `Test-Path target` returned **False**. C: reported **67,100,258,304 bytes free** (about 62.5 GiB). No session, checkpoint, credential or reference data was removed. The failed gates remain unresolved; cleanup is disk hygiene, not a claim of successful verification.

## Checks deliberately not performed

- No new defect reproduction tests were written: Phase 1 is documentation-only.
- No source fix, formatter in write mode, install, commit, push or release.
- No Linux/macOS runtime tests, MSRV 1.88 run, interactive TUI drive, dependency advisory audit, memory benchmark or fresh-profile install.
- No reference-harness builds or tests; comparisons are source-level.
- No claim that every defect in 02 was executed. Each labels source evidence versus inferred consequence.

## Independent Hermes revalidation

A separate run executed `bash scripts/gate.sh phase1-verify` against the same
implementation baseline. Unlike the historical wrapper attempt above, Cargo resolved
on PATH. Versions: Cargo 1.97.1, rustc 1.97.1. The wrapper finished **GATE FAIL**.

| Gate | Exit | Elapsed |
|---|---|---|
| Formatting check | 0 | 3 s |
| Workspace strict Clippy | 101 | 316 s |
| Workspace tests | 101 | 748 s |
| Offline demo | 0 | 197 s |
| Real-provider compile check | 0 | 53 s |

The same two checkpoint lint diagnostics and eviction assertion reproduced. Across
38 test-binary summaries emitted before fail-fast termination, Python aggregation
counted **594 passed, 1 failed, 1 ignored**. This is a partial run total, not a full
workspace suite count. The demo reported Completed, six messages, 300/150 tokens.

Portable selected verbatim evidence is in
[`evidence/phase1-verify.txt`](evidence/phase1-verify.txt). It excludes build chatter
and demo configuration metadata; it is not a replacement raw transcript invented
from the old report. No source fixes, new tests, live calls or second build were run.
The unrun tools integration/TUI/doctest binaries remain unverified in this round.

After saving evidence, `cargo clean` removed **14,628 files, 8.5 GiB**. Removal of
`target/` and `test ! -e target` succeeded; `df -h /c` reported **63G available,
87% used**. No installed binary refresh was necessary: no CLI/TUI behavior changed.
A 166-file source/assets/manifest hash snapshot showed no changes during the checked
interval. This does not retrospectively certify earlier uncommitted source changes.

## Phase 2 follow-up: intermittent auth-lock failure

A subsequent `cargo test --workspace` exited **101**, stopping earlier than the
checkpoint failure: `auth::tests::concurrent_saves_keep_every_provider` failed at
`auth.rs:1096` while acquiring `auth.lock` with Windows error 5 (Access is denied).
The LLM binary reported **105 passed, 1 failed**. Aggregating the 22 completed
summaries yielded **354 passed, 1 failed, 0 ignored**, a partial run only.

Ten exact isolated reruns of the original workspace-built LLM executable all
exited **0**. The failure remains unresolved; neither a Windows deletion race nor
any other root cause has been established. The earlier package-only reruns used
a different feature selection (71 tests versus 106); their shell-loop failure
counter also lacked `pipefail` and is not reliable exit-code evidence.

Selected original failure output and all ten directly captured rerun results are
preserved in [the follow-up evidence](evidence/phase2-auth-followup.txt). No runtime
fix or new regression test was applied. Existing checkpoint gate failures are not
superseded by this earlier fail-fast stop. No full five-gate rerun is claimed.

After the process exited and evidence was saved, `cargo clean` removed **10,271
files, 6.7 GiB**. `test ! -e target` succeeded; C: had **63G available, 87% used**.
No installed binary refresh was required.

## Next verification round

After Phase 3 is authorized, correct F00 and run the complete gates once. The targeted failing command is:

```sh
cargo test -p epsilon-tools --lib checkpoint::tests::the_store_evicts_old_blobs_when_it_grows_past_its_ceiling
```

Record the new baseline, changed files and actual results separately; do not rewrite the failed results above as if this audit started green.
