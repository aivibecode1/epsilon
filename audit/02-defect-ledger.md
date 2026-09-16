# 02 — Gap analysis and defect ledger

**Audience:** engineers fixing reliability and safety, and the owner selecting scope.
**Snapshot:** 2026-09-16 working tree over `e200815`. **Update 2026-09-17:** F00 was corrected and verified in Phase 3 batch A — see [05](05-verification-log.md#phase-3-batch-a--f00-verification-2026-09-17); the row below describes the pre-fix snapshot and stays for traceability.

**Severity:** P0 = potential destructive loss or fundamental execution-boundary failure; P1 = blocking reliability/correctness; P2 = workflow or maintainability. Priority is not an exploitability rating.
**Evidence:** E = executed here; S = source-verified; I = conditional consequence/inference. Unless marked E, a proposed regression below was **not run**. Estimates include tests for one experienced Rust engineer; see 04 for dependency order.

## Ranked inventory

| ID | Priority | Finding | Evidence | Effort / change risk |
|---|---|---|---|---|
| F00 | P1, prerequisite | Current verification baseline fails — **fixed 2026-09-17, gates green** (05) | E | 0.5–1 day / low |
| F01 | P0 | Editing a capped buffer overwrites the complete file | S, high confidence | 0.5–1 day / low |
| F02 | P0 | Read observations are shared between agents | S, high | 2–4 days / medium |
| F03 | P1 | SSE buffering depends on HTTP chunk boundaries; EOF can claim success | S, high | 3–5 days / medium |
| F04 | P1 | Repeated durable compaction accumulates old summaries/tails | S, high | 2–4 days / medium |
| F05 | P1 | Failed shell checks are recorded as passing verification | S, high | 1–3 days / medium |
| F06 | P1 | Failed or cancelled summarization can replace valid context | S, high | 2–3 days / medium |
| F07 | P0 | Persistence failures are discarded while model/store views diverge | S, high | 4–7 days / high |
| F08 | P0 | Complete unsupported final entries are treated as torn writes | S, high | 1–3 days / medium |
| F09 | P0 | Native file tools have lexical, not resolved, containment | S; exploitability filesystem-dependent | 4–8 days plus race-resistant design / high |
| F10 | P1 | Plan-mode shell denials do not match execution subjects | S, high | 1–3 days / medium |
| F11 | P1 | Process deadlines/cleanup do not cover the whole operation | S; descendant survival/hangs inferred | 3–6 days / medium |
| F12 | P1 | Output/input caps apply after unlimited allocation | S, high | 3–5 days / medium |
| F13 | P1 | RPC/HTTP can acknowledge work that is rejected | S, high | 2–4 days / medium |
| F14 | P1 | LSP writes/framing/versioning undermine bounded fresh diagnostics | S, high | 3–5 days / medium |
| F15 | P0 before exposing restore | Checkpoint restoration conflates skipped capture with creation | S, high; frontend reach not established | 3–6 days / high |
| F16 | P1 | OAuth refresh orchestration differs from login/fallback paths | S, high for control flow | 3–6 days / high |
| F17 | P1 | Locally aborted partial replies are not durably committed | S, high | 1–2 days / low |
| F18 | P1 | Resumed child task is not marked running before dispatch | S, high | 1–2 days / low |
| F19 | P2 | Product/docs/parity claims exceed actual workflows | S plus E baseline | 3–5 days docs; workflow work separate / low |

## F00 — Restore the baseline without weakening its assertions

**Executed:** fmt=0, strict Clippy=101, workspace tests=101, demo=0, real-check=0. Full evidence in [05](05-verification-log.md).

Clippy reports unused `Hash as _` at `crates/epsilon-tools/src/checkpoint.rs:340` and collapsible `if` at `:312`. The failing test is `checkpoint::tests::the_store_evicts_old_blobs_when_it_grows_past_its_ceiling`, assertion at `:471`.

**Source diagnosis:** `record()` already invokes eviction (`:211–212`). The test repeatedly records, then expects another eviction call to remove something (`:462–471`). That expectation is not evidence that eviction itself fails. Keep the invariant—bounded eligible storage and newest-preimage preservation—and test it at the correct operation boundary; do not simply delete the assertion or disable Clippy.

**Acceptance:** all five gates pass; regression checks storage after each capture and keeps the newest recoverable blob. If protected latest blobs alone exceed the configured ceiling, document/test whether the cap is soft or capture must refuse.

## F01 — Never write a display-limited source buffer back as a whole file

**Evidence:** `crates/epsilon-tools/src/util.rs:171–194` drops the capped flag and reads at most `MAX_READ_BYTES`; `crates/epsilon-tools/src/files.rs:482–501` edits that prefix and writes it over the original. The batch path does the same at `:620–635`.

**Trace:** file exceeds the 256 KiB text cap → replacement matches in prefix → complete-file write contains only the modified prefix → unread suffix is lost. Lossy UTF-8 conversion can also rewrite unrelated bytes. Existing fuzzy/CRLF tests do not prove preservation beyond the reader cap.

**Minimum fix:** refuse capped or invalid-UTF-8 edit input before mutation. A separate bounded full-file edit reader may be added later if required. Do not remove the model-visible read cap to fix write correctness.

**Acceptance:** single and batch edits against a file with a unique suffix beyond the cap either preserve every unrelated byte or refuse with the original byte-identical. Cover boundary size, multi-byte split and invalid UTF-8. Checkpoint existence is not an excuse for destructive writeback.

## F02 — Track what each agent actually observed

**Evidence:** `crates/epsilon-tools/src/read_log.rs:90–92` is one process-global `PathBuf → Snapshot`; `:120–139` updates it after every read/write; `:153–179` verifies against whichever agent refreshed it last. Real mutations call it at `crates/epsilon-tools/src/files.rs:477–501`. Core supplies a non-distinguishing `"inline"` context ID at `crates/epsilon-core/src/exec.rs:661–669`.

**Trace:** A reads → B writes through Epsilon → B refreshes global stamp → A's stale full overwrite compares against B's stamp and passes. A read by B can also satisfy A's never-read rule. This is a semantic error, not merely missing writer attribution.

Hermes retains per-task observations (`references/hermes-agent-2026.8.19/tools/file_state.py:93–139,157–189`). The older audit's “fixed M65” conclusion is reopened. M65's global-observation decision must be amended; the path-global mutation lock should remain.

**Acceptance:** two actual tool contexts, not an external `fs::write` substitute: B's successful write invalidates A until A rereads. Include partial reads, failed writes, independent sessions and observation cleanup. Resolve file identity consistently with F09; metadata-only stamps still have same-size/same-timestamp limitations.

## F03 — Process already-received SSE lines before awaiting the network

**Evidence:** `crates/epsilon-llm/src/providers/mod.rs:776–811` returns after the first event-producing frame, leaving complete lines buffered. `:820–847` polls another HTTP chunk before draining them. `:865–870` feeds the entire EOF remainder as one line. `:947–958` synthesizes completion from content/tools even without terminal evidence.

**Trace:** one HTTP body chunk contains several SSE frames → first frame returns → terminal frame remains buffered → a quiet server causes a false idle timeout, or EOF processing loses frame boundaries. A missing terminal reason can also be mapped to ordinary completion/tool use, rather than an interrupted response.

**Impact:** truncated prose/usage, stalled runs and potentially executing calls from an incomplete generation. The execution consequence depends on the particular wire/EOF shape; no real provider reproduction was performed.

**Coverage gap:** wire fixtures test codec events and line helpers, not every production `WireStream` network-chunk partition.

**Acceptance:** local HTTP fixture sends the same valid stream as one chunk, byte fragments, several frames/chunk, and final-frame-with-connection-held-open; results and usage must agree. Missing terminal evidence must yield failed salvage and no executable partial call. Valid provider-specific final framing remains supported.

## F04 — Make durable compaction replace superseded model context

**Evidence:** `crates/epsilon-session/src/store.rs:717–748` appends every summary/tail to `out` and clears only `pending`; `crates/epsilon-core/src/lib.rs:1484–1512` replaces runtime history with the new summary/tail. Provider calls prefer the store view (`:1592–1604`).

**Trace:** C1 gives S1/T1; C2 replaces memory with S2/T2 but persisted projection gives S1/T1/S2/T2. Retained material can duplicate: older summaries and tails remain even though newly pending messages are still shadowed. This is a projection error, not physical duplication of the historical log. Resume restores the divergent projection.

**Acceptance:** attach a real store, compact twice, compare exact provider context against intended current summary/tail, then reopen and compare again. Include alternate branches. Preserve every human-log entry; fix the projection, not history by deletion.

## F05 — Distinguish process exit from successful tool transport

**Evidence:** nonzero shell exits intentionally return `Ok(ToolOutput)` with `details.exit_code` (`crates/epsilon-tools/src/shell.rs:301–329,416–438`). Verification derives success only from `!result.is_error` (`crates/epsilon-core/src/lib.rs:1173–1179`). `crates/epsilon-core/src/verify.rs:151–175` records that boolean and treats any successful check in the same or later turn as covering an edit.

**Trace:** tests exit nonzero → bash successfully reports the result → verification marks pass. Additionally, a check before an edit within the same serial batch gets the same turn number and wrongly covers that later edit.

**Acceptance:** real-shaped `Ok` result with nonzero/null exit remains unverified; zero exit after the mutation may count. Check-before-edit must not count. Unknown outcomes remain unknown. Carry execution order and conservative check scope; do not redefine every ordinary nonzero shell exit as a transport failure.

## F06 — Commit summaries only on success, with cancellation

**Evidence:** `crates/epsilon-core/src/lib.rs:1468–1478` accepts any summarizer `Done` and does not select on cancellation. `crates/epsilon-core/src/compact.rs:241–248` substitutes a template for empty text. The replacement commits at `lib.rs:1497–1512`.

**Trace:** a failed or empty `Done` message is accepted as summary content; empty text produces a placeholder that shadows useful context. Raw early EOF is already rejected before replacement (`lib.rs:1473–1474`). Cancellation does not itself create a placeholder, but it does not interrupt summary creation/polling and can allow a later result to commit.

**Acceptance:** provider error, empty answer, early EOF, cancelled connect and cancelled stream leave history/store view unchanged and emit a truthful failure/cancellation outcome. A successful summary still uses legal cuts. Coordinate commit semantics with F04/F07.

## F07 — Surface persistence failures and retain one authoritative transcript

**Evidence:** `crates/epsilon-core/src/lib.rs:743–777` discards append/pop failures after changing memory; compaction similarly ignores append failure (`:1497–1512`). Next requests use store projection (`:1592–1604`).

**Trace:** disk fills during prompt/result append → UI/in-memory history advances → store index does not → model sees older context or resume loses the turn. A partial physical append followed by continued writing can damage the log. This is especially material on the disk-constrained development machine.

**Minimum decision:** stop further model/tool execution after a failed durable commit, or explicitly switch to an announced in-memory-only mode. Never silently alternate authoritative sources. Specify append, projection and UI event commit order.

**Acceptance:** injected short-write/flush/append errors at prompt, assistant, result and compaction boundaries; no successful durable claim, no subsequent provider request using an older view, prior valid log recoverable. Do not depend on physically filling the operator's disk to test this.

## F08 — Refuse schema/version errors; repair only genuinely incomplete tails

**Evidence:** `crates/epsilon-session/src/store.rs:484–500` labels any unusable final entry torn without diagnosis. Opening truncates this case (`:273–275`). `truncate_to_index` defaults to byte zero if no entry indexed (`:550–558`).

**Trace:** a complete final entry rejected by both typed and lenient parsing is treated as crash debris and truncated; a final second header also follows this path (`store.rs:457–463`). Recognized legacy shapes are separately migrated or refused, so not every unsupported shape is deleted. Header plus a torn first entry leaves no indexed entry, so repair also deletes the header. A clean header-only file is not itself subject to this repair.

**Acceptance:** unknown complete final entry, second complete header and incompatible schema are refused with the entire file unchanged. Incomplete syntax tail may be repaired; a header-only surviving prefix stays on disk and reopens correctly. Test both terminated and unterminated final lines. Amend tests that codify “any bad final line is safe to truncate.”

## F09 — State and enforce the real filesystem boundary

**Evidence:** lexical normalization at `crates/epsilon-core/src/exec.rs:105–139`; native edit opens/writes host paths at `crates/epsilon-tools/src/files.rs:461–499`; walker follows directories at `crates/epsilon-tools/src/util.rs:144–166`. `crates/epsilon-core/src/paths.rs:9–34` delegates symlink escape to a lower layer that native tools do not use.

**Impact:** a workspace symlink/reparse point can name an outside object while the gate sees an inside path. Alias spellings can defeat resource identity. Shell sandboxing does not surround native tool filesystem calls. This was historically deferred, but the stated product confinement guarantee exceeds the implementation.

**Acceptance:** read/write/edit/search/attachments/checkpoints share an explicit resolved-path policy. Test existing aliases and new-file parent aliases on native Windows/Linux. A canonicalize-then-open check alone does not close replacement races; either use handle-relative/no-follow access or explicitly refuse unsupported traversal and document residual races. Supporting legitimate linked workspaces requires an explicit permission policy, not silent access.

Windows Job Objects restrict resources, not file or network access; bubblewrap's read-only root is not read confidentiality (`crates/epsilon-safety/src/sandbox.rs:282–348`). Do not describe either as universal protection against secret-file reads.

## F10 — Plan restrictions must survive actual subject extraction and layering

**Evidence:** plan rules match `git commit*`/`git push*` (`crates/epsilon-safety/src/engine.rs:137–145`), but core authorizes only the first shell word (`crates/epsilon-core/src/exec.rs:141–147`). CLI chooses plan rules **instead of** project rules (`crates/epsilon-cli/src/main.rs:1581–1593`).

**Impact:** subcommand denies cannot match; a broad shell approval can allow actions advertised as forbidden. Project-specific deny rules disappear in this mode. Disabling native write/edit alone is not a read-only shell boundary.

**Acceptance:** preserve all project denies when adding plan restrictions. Test through core preflight with an inert recording tool. Choose conservative shell refusal or genuinely enforced read-only execution; do not promise arbitrary shell classification via string prefixes. Windows partial enforcement must be surfaced/refused where stronger guarantees are required.

## F11 — Own child exit, descendants and pipe draining under one deadline

**Evidence:** bwrap ignores `_policy`, awaits child exit and drains without timeout/kill-on-drop (`crates/epsilon-safety/src/sandbox.rs:389–438`). Shell cancellation only drops that future (`crates/epsilon-tools/src/shell.rs:280–298`). Unconfined shell's timeout covers direct-child wait, not later drains (`:385–407`). Windows similarly drains after timed wait while holding the job (`crates/epsilon-safety/src/sandbox/jobobject.rs:271–294`).

**Impact:** Linux timeout is not implemented; cancellation may leave work running. If a descendant retains a pipe after the direct child exits, final drain can hang past the promised timeout. These lifecycle consequences require native process tests, not sleeps-and-hope.

**Acceptance:** successful completion, timeout, abort, descendant-held stdout, runner startup failure and runtime cancellation all settle within a total deadline, with owned children reaped. Results distinguish “did not start,” “started then cancelled” and normal nonzero exit. `--die-with-parent` is not cancellation while Epsilon stays alive. Require Linux CI evidence before closing.

## F12 — Bound memory at ingestion, not only model-visible output

**Evidence:** `drain_pipe` grows an unlimited vector (`crates/epsilon-safety/src/sandbox.rs:259–279`), shell truncates afterward (`crates/epsilon-tools/src/shell.rs:303–305`). File read loads full bytes for image detection (`crates/epsilon-tools/src/files.rs:113–144`); webfetch buffers body before final size check (`crates/epsilon-tools/src/webfetch.rs:155–175`).

**Impact:** command/file/HTTP volume can exhaust the harness's memory despite bounded displayed output. Child memory limits do not constrain the parent collector.

**Acceptance:** bounded signature read for images, explicit attachment limit, streamed HTTP byte budget, bounded head/tail capture plus capped spill. Test oversized individual chunks, missing Content-Length, spill failure and multibyte clipping. Tell the model when full output cannot be retained; never advertise a nonexistent complete spill.

## F13 — Acknowledge runs only after atomic admission

**Evidence:** RPC busy helper always returns false, starts a detached task and discards `run()` result (`crates/epsilon-cli/src/rpc_mode.rs:77–85,117–120`). HTTP checks then increments separately (`crates/epsilon-server/src/lib.rs:238–253`). Core rejecting concurrent runs does not retroactively correct a successful admission response.

**Impact:** callers can receive `started`/202 for rejected work and wait for events that never identify their failure. RPC also silently skips lagged broadcasts (`rpc_mode.rs:65`), has no correlation ID and no abort operation (`:16–35`).

**Acceptance:** competing requests have exactly one accepted run, the other gets an attributable busy error; reserve/release admission even on failure. Add cancellation and explicit dropped-event/recovery semantics before treating RPC as a dependable editor API. A mere racy `is_running()` replacement is insufficient.

## F14 — Make LSP feedback bounded and tied to the current document

**Evidence:** `crates/epsilon-lsp/src/client.rs:234–262` repeats version 2 after every open and sends before the timeout. `:268–300` accepts the first matching URI and may return cached diagnostics after timeout. Framing holds partial read state inside a cancellable future (`crates/epsilon-lsp/src/framing.rs:45–86`). Tool mutation awaits diagnostics inside its queue (`crates/epsilon-tools/src/files.rs:467–511`).

**Impact:** stalled stdin can hang a successful edit response; a timeout mid-frame can desynchronize reuse; delayed diagnostics can describe old bytes. Actual server rejection of repeated versions is server-dependent.

**Acceptance:** monotonic per-document versions and correct language IDs; delayed/stale publications distinguished from fresh results; total timeout includes writes. Persistent reader state or retire client after partial-frame timeout. Fixture stops reading input and splits frames across deadlines. Optional LSP failure must not change successful file mutation into a false failed edit.

## F15 — Checkpoint capture is not yet safe user undo

**Evidence:** skipped oversized captures have `blob: None, existed: true, skipped: Some(...)` (`crates/epsilon-tools/src/checkpoint.rs:168–184`). `restore()` treats every `None` blob as deletion (`:261–277`), without checking skip/existence. Ledger append errors are swallowed (`:223–235`) while capture can report success (`:211–216`). Restore writes without checking later user changes (`:265–270`).

**Impact:** restoring a skipped capture through this API deletes an existing file; restoring a real preimage can overwrite later work. No normal UI route to this API was established, so this is a **blocker before exposure**, not a claim that users already trigger it with a shipped slash command.

**Acceptance:** typed captured/created/skipped states; skipped is never restorable. Persist post-write identity and refuse conflicts unless a human explicitly overrides after preview. Surface capture failures, validate ledger/blob paths, serialize ledger append across different-file writers, and test interrupted capture/eviction. Reuse the current store rather than adding shadow Git first.

## F16 — Unify call-time OAuth preparation

**Evidence:** `crates/epsilon-llm/src/providers/mod.rs:1161–1181` requires an environment client ID before calling the refresh exchange. Copilot's endpoint has no such requirement (`crates/epsilon-llm/src/oauth.rs:79–87,223–226`). Login supports config client IDs (`crates/epsilon-llm/src/oauth_login.rs:780–789`). Fallback retains credentials and invokes direct transport without the same refresh preparation (`crates/epsilon-llm/src/fallback.rs:33–40,89–115`).

**Impact:** successful login does not imply refresh works; long-running fallback sessions can reuse expired tokens. These are source integration findings, not live account tests.

**Source-confirmed threshold mismatch:** `crates/epsilon-llm/src/auth.rs:623–627,653–662` can start proactive refresh while the old token remains live beyond one minute, then reject the newly exchanged token merely because the old stored token is live and different. No concurrent writer is required by those branches. The code needs to compare with the originally exchanged snapshot. Whether discarding a rotation invalidates subsequent refresh depends on provider semantics; this was not tested with a live account.

**Acceptance:** injected clock/store/exchange tests for Copilot without client ID, config-only IDs, expired fallback credentials, missing rotation and true concurrent refresh. Compare against the originally exchanged credential snapshot. Required client IDs remain required for flows that need them; never make all IDs optional to fix Copilot.

## F17 — Persist cancellation salvage like ordinary completion

**Evidence:** cancellation returns locally synthesized assistant messages directly (`crates/epsilon-core/src/lib.rs:1680–1703`); provider `Done` uses `push_message` and `MessageEnd` (`:1729–1733`).

**Impact:** partial prose can survive in `RunOutcome` while absent from both in-memory history and the durable transcript. Ordinary raw-EOF salvage does push history and emit `MessageEnd` (`lib.rs:1772–1782`); the finding is specific to locally synthesized cancellation. The inspected abort regression checks returned output, not reopened history.

**Acceptance:** attached-store abort during connect and after text; partial/error state retained exactly once, normal terminal lifecycle emitted, incomplete calls never executed. Reopen and compare. Coordinate error handling with F07.

## F18 — Reset task state when resuming

**Evidence:** resumed child lookup checks prior state but does not set `Running` (`crates/epsilon-core/src/lib.rs:2320–2356`); background dispatch writes final state later (`:2495–2530`). Abort selects running tasks (`:980–995`); eviction can select non-running entries (`:2219–2229`).

**Impact:** a resumed child appears settled while running, can evade cancellation tracking or become eligible for eviction. Core may still reject concurrent execution, but the registry misreports it.

**Acceptance:** controlled slow-provider resumed child appears running before dispatch acknowledgment, refuses second resume, receives parent abort and remains unevictable until settled. Failed launch restores a truthful terminal state.

## F19 — Reconcile product workflows and documentation

- **Rejected stale allegations:** the current README already lists twelve crates and warns about the failed baseline (`README.md:17–21,155–168,185–193`). Its sandbox wording now applies to commands, not every tool (`:41–44`). Do not repeat those earlier allegations as current facts.
- **Still incorrect:** no runtime dependencies (`README.md:8–9`), read-only plan-mode guarantees (`:107`), compiler-enforced architecture and types-only contracts (`:148–157`). External executables, F10, and executable `StreamFolder` logic contradict these claims.
- **Already reconciled:** Plan 018 is PART DONE in `docs/plans/README.md:38`, matching the Copilot/Codex split. The old TODO allegation is stale.
- **Broken provenance link:** `docs/README.md:62–63` claims a restructure move map lives in 05; no such map exists there. Existing document moves predate this audit run.
- `/tree` inspects only active-branch entries; `/fork` copies a file and does not switch (`crates/epsilon-tui/src/app.rs:1446–1477,1594–1608`). The `Command::New` arm only clears UI (`:1487–1492`); check parser/browser routing before attributing that behavior to every `/new` invocation.
- App orchestration is concentrated in a 6,404-line source file; split only stable ownership seams after characterization, not arbitrary line counts.

**Acceptance:** one current status source; explicit compatibility/platform matrix; README → setup → first run → permission handling → resume/recovery walkthrough checked on a fresh profile. Preserve history as history, not current guarantees.

## Deliberate differences, not implementation bugs

Keep process-isolated extensions, ratatui, TOML configuration, bounded resource-conflict scheduling, advisory verification nudges and fail-closed unavailable confinement. Each can differ from pi intentionally. None justifies incorrect data preservation or unbounded execution.

Do not add fuzzy edit stages, a plugin framework replacement, distributed teams, a GUI, automatic LSP downloads, or memory curation before these existing contracts work. Missing pi capabilities are tracked separately in [03](03-port-plan.md), rather than inflated into security defects.

## Unclosed audit areas

Dependency advisories/license provenance, hostile multi-process lease races, complete OAuth/provider wire conformance, cold-start/install artifacts, TUI visual accessibility, long-session performance and cross-platform execution need separate evidence. Absence of a finding here is not a clean bill of health. No severity is assigned to an unverified CVE or guessed upstream behavior.
