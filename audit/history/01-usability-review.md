# 01 — Current state

**Audience:** engineers onboarding to the current implementation and the owner evaluating readiness.
**Snapshot:** 2026-09-16, dirty working tree over `e200815`. See [verification](05-verification-log.md) for execution evidence and [findings](02-defect-ledger.md) for defects.

## What this project actually is

Epsilon is a Rust workspace with one CLI binary and twelve crates. It implements a provider/tool loop, terminal and headless frontends, session storage, permissions, subprocess confinement, extensions, MCP, and optional LSP diagnostics. It borrows pi's loop structure but uses different storage, safety, extension and UI implementations. It is not a drop-in pi SDK, RPC server, or session-file replacement.

The initial Git inspection reported 215 short-status entries; the number includes modified/deleted paths and untracked directories, **not an exact file or milestone count**. HEAD was `e200815 fix(m32d): live /login hot-swap + copy fix — flash via /model now goes real`. Later source includes checkpoint work marked M80, beyond the M79 documentation. This audit evaluates those working files; a clean checkout of HEAD is a different product. No implementation changes were committed by this revalidation. Audit-only commits are recorded in 00/05; the earlier report had left its documents uncommitted.

## Repository organization

| Location | Actual role |
|---|---|
| `Cargo.toml:3–15` | Authoritative twelve-crate workspace membership |
| `crates/epsilon-cli/src/main.rs:1581–1603` | Composition of gate, Agent and durable store; mode/argument handling also lives here |
| `crates/epsilon-cli/src/runtime.rs` | Project config, provider/resource composition and adapter attachment |
| `crates/epsilon-contracts/src/` | Shared messages, events, tool and diagnostic traits; also executable helpers/validation, not literally types only |
| `crates/epsilon-llm/src/providers/mod.rs:755–975` | Shared live SSE transport; provider-specific codecs live beside it |
| `crates/epsilon-core/src/lib.rs:1037–1261` | Main loop, queues, tool-result commitment and stop guards |
| `crates/epsilon-core/src/exec.rs:183–256` | Resource-conflict batch planning; later sections execute the plan |
| `crates/epsilon-session/src/store.rs:594–749` | JSONL append, branch traversal, navigation and model-view projection |
| `crates/epsilon-tools/src/` | Native file/search tools, bash, task, webfetch, read observations, mutation locks and checkpoint storage |
| `crates/epsilon-safety/src/` | Rules, approvals, environment scrubbing and sandbox backends |
| `crates/epsilon-ext/src/`, `crates/epsilon-mcp/src/` | Process extension host and MCP adapter |
| `crates/epsilon-lsp/src/` | Language-server discovery, process client, framing and error reports |
| `crates/epsilon-tui/src/app.rs:1446–1614` | Application orchestration and session commands; rendering/editor modules coexist in the crate |
| `crates/epsilon-server/src/lib.rs:109–136` | HTTP router over the Agent |
| `docs/` | Design decisions, historical milestones, plans, QA and research; present structure is already categorized but statuses conflict |
| `audit/` | This assessment and its evidence; not a second implementation history |
| `references/` | Four local source snapshots for comparison; not part of Cargo membership |
| `.agents/` | Development skills; `.epsilon/` is runtime/project state and must not be swept into source reorganizations |

### Dependency shape

The architecture is a crate graph, not a strict twelve-step stack. CLI composes siblings; providers and sessions depend on shared vocabulary; core orchestrates through traits; tools bridge core/safety behavior; TUI/server depend on the runtime-facing crates. The CLI directly names all sibling crates (`crates/epsilon-cli/Cargo.toml:32–43`). LSP communicates through `DiagnosticsProvider` rather than making core depend on an LSP implementation.

Cargo checks dependency validity and prevents cycles. It does **not** prove the README's architectural layering law or the “types only” rule. Root lints declare warnings, not unconditional denial (`Cargo.toml:70–75`); CI and the gate script supply `-D warnings` for Clippy (`.github/workflows/ci.yml:37–38`, `scripts/gate.sh:22`). Keep these distinctions in the Phase 2 architecture document.

## Implemented surfaces and evidence

| Surface | What exists | Fresh execution evidence / limitation |
|---|---|---|
| Loop | Two-level turns, steering/follow-up, per-run rejection, tool hooks, max-token call rejection, bounded scheduling | Core scenario binaries passed before the workspace run stopped. Demo completed. This does not cover persistence failures or live transport chunking. |
| Tools | `read`, `write`, `edit`, `bash`, `grep`, `glob`, `ls`, `find`, `todo`, `task`, `webfetch`; registration at `crates/epsilon-tools/src/lib.rs:109–138` | Tools unit binary: 75 passed, one checkpoint test failed. Later tool integration binaries were not established as passing by this fail-fast run. Large-file edits are unsafe (F01). |
| Providers | Native protocol adapters, catalog routing, retry/fallback, API keys and several OAuth flows | Real feature compiled; codec/login unit and fixture tests passed; live smoke test ignored. Production SSE and refresh orchestration have source-verified gaps. |
| Sessions | Versioned JSONL, branch IDs, lazy indexed reads, repair, leases, compaction entries, search | Session unit and integration binaries passed. Multiple compactions, final unsupported entries and append failures expose missing boundary coverage. |
| TUI | Ratatui editor, completion/picker, Markdown, transcript, approval dialogs, session browser and commands | Source inspected; interactive operation and visual floor tests not exercised. Workspace testing stopped before establishing TUI suite results. A slash name is not proof of pi-equivalent semantics. |
| HTTP | Prompt/steer/followup, SSE, approval routes, OpenAPI; bearer/Host/Origin guard | Eight server tests passed, including socket SSE and approval round trip. No remote multi-user isolation claim; run-admission race remains (F13). |
| RPC | Six input operations with event pump and line-delimited replies | Source is implemented; lacks abort/state/session-control operations and truthful busy admission (F13). No pi protocol compatibility. |
| Extensions/MCP | Subprocess extension handshake/invocation/events; stdio MCP discovery and calls | Extension and MCP fixture suites passed. This is not pi's complete extension SDK or evidence of confinement for arbitrary plugin code. |
| LSP | Auto discovery of installed servers, reused clients, bounded-count error blocks after mutation | 18 unit and five process-fixture tests passed. Repeated document versions and incomplete timeout boundaries remain (F14). |
| Checkpoints | Tool-owned preimage blobs and ledger; capture wired before edits | Already implemented in working source, not a future greenfield feature. Test failure, swallowed ledger errors and unsafe restore distinctions prevent a recovery guarantee. |

## What actually worked in this audit

Executed using `C:\Users\Hp\.cargo\bin\cargo.exe` on Windows:

- `cargo fmt --all -- --check`: exit **0**.
- `cargo clippy --workspace --all-targets -- -D warnings`: exit **101**; two checkpoint diagnostics.
- `cargo test --workspace`: exit **101**, stopped on checkpoint eviction assertion. **Not a full green suite.**
- `cargo run -q -p epsilon-cli -- --demo`: exit **0**, `Completed`, six messages, reported 300/150 input/output tokens.
- `cargo check -p epsilon-cli --features real`: exit **0**, with unused-import warning.

The demo uses scripted echo/failure tools. It establishes an executable loop, **not** successful production file editing, OS confinement, real authentication or a paid-provider workflow. The exact failure and scope are recorded in 05.

## Setup and platform reality

- Manifest: Rust 2024 edition, minimum Rust 1.88 (`Cargo.toml:18–23`). Local execution used Cargo 1.97.1; minimum-version compilation was not rerun.
- Source installation path is `cargo install --path crates/epsilon-cli --force`. `real` is already default (`crates/epsilon-cli/Cargo.toml:13–30`). Installation was not performed in this phase, and should not ship the failing baseline.
- Native Windows uses Job Objects for resource/process containment. **Filesystem and network are not constrained by that backend.** The spawn-to-assignment window remains an accepted limitation, not the only limitation.
- Non-Windows selection looks for `bwrap` (`crates/epsilon-safety/src/sandbox.rs:282–308`). Linux has a profile; macOS has no native confinement backend here and normally falls to refusal unless confinement is explicitly disabled. Do not advertise uniform Unix sandbox support.
- Bubblewrap read-only mounts restrict writes, not host-file visibility. Its `--unshare-all` also restricts network; dependency downloads inside sandboxed builds may fail by policy. Environment scrubbing removes credential-shaped variables, not secrets readable from host files.
- LSP servers, shell executables and extension/MCP programs are external requirements when used. “No runtime dependencies” needs qualification.
- The earlier report's Bash gate did not find Cargo in its environment; the independent revalidation resolved Cargo and completed the wrapper. A Windows executable worked. A shell mismatch is not proof the Rust build is broken; the independent Clippy/test failures are.

## Readiness assessment

**Usable for supervised experimentation on disposable or backed-up workspaces:** substantial features and executed offline paths support this assessment.

**Not ready for unattended modifications of valuable repositories:** F01/F02/F07/F08/F09/F15 concern data loss, stale authorization or misleading durability. A model's quality cannot repair these runtime contracts.

**Not yet a dependable editor backend:** RPC admission/errors/control are incomplete; HTTP admission has a race; no tested replay/resynchronization guarantee for dropped events.

**Not a complete pi port:** see [03](03-port-plan.md). No meaningful percentage is assigned: command counts and lines of Rust do not measure workflow equivalence.
