# 07 — Internal inconsistencies and ownership decisions

**Audience:** engineers preparing the reviewed restructure and hardening batches.
**Evidence:** source inspection of the working tree over `e200815`, 2026-09-16.
These recommendations are not permission to change source in Phase 1. Findings
reference the numbered defect ledger; runtime consequences remain source-derived
unless the verification log records execution.

| Concern | Competing implementations / contradiction | Approach to retain and why |
|---|---|---|
| Transcript authority | Core updates history and ignores store errors (`crates/epsilon-core/src/lib.rs:743–777`), but model calls prefer store projection (`:1592–1604`). Compaction replaces memory while projection accumulates old boundaries (`crates/epsilon-session/src/store.rs:717–748`). | Durable commit must define the authoritative view; report failures before execution advances. Keep append-only historical entries, correct projection rather than delete history. F04/F07. |
| Run admission | RPC always reports not busy and discards run failure (`crates/epsilon-cli/src/rpc_mode.rs:77–85,117–120`); HTTP checks/increments separately (`crates/epsilon-server/src/lib.rs:240–254`); core independently rejects overlap (`crates/epsilon-core/src/lib.rs:900–903`). | Core owns atomic admission and run identity. Frontends encode the same admitted/rejected result, not independent promises of acceptance. F13. |
| Filesystem identity | Lexical containment in `crates/epsilon-core/src/exec.rs:105–139` and `paths.rs:13–34`; independent keys in `crates/epsilon-tools/src/mutation.rs:15–26` and `read_log.rs:95–104`. | Shared authorized resource identity, with distinct scheduler, global mutation lock, and per-observer knowledge. Do not mistake canonicalize-before-open for race-resistant access. F02/F09. |
| Observation versus exclusion | `read_log.rs:16–18,90–92` applies process-global state to knowledge just as mutation locking does to exclusion. A sibling writer refreshes the reader's stamp (`:135–139`). | Keep global exclusion; scope observations to the actual agent/session. Shared files do not imply shared model knowledge. F02. |
| Error contracts | Shell reports nonzero process exit in successful tool output (`crates/epsilon-tools/src/shell.rs:301–330,416–438`); verification counts transport success (`crates/epsilon-core/src/lib.rs:1173–1179`). Store append returns errors but core ignores them; checkpoints swallow ledger errors internally (`checkpoint.rs:223–235`). | Keep process exit distinct from transport failure. Require typed process outcomes and fallible persistence at their consumers. One generic boolean cannot represent both. F05/F07/F15. |
| Process lifetime | Shell, bubblewrap and Job Object adapters separately wait and drain (`crates/epsilon-tools/src/shell.rs:385–407`; `crates/epsilon-safety/src/sandbox.rs:389–438`; `sandbox/jobobject.rs:271–294`). | Shared supervision contract for total deadline/cancellation/drain; retain platform-specific termination adapters. Windows already owns a job: do not erase working cleanup when fixing Linux gaps. F11. |
| OAuth preparation | Login accepts configured client IDs (`crates/epsilon-llm/src/oauth_login.rs:780–789`); direct preparation requires an environment ID (`providers/mod.rs:1161–1181`); fallback bypasses preparation (`fallback.rs:89–115`). | One call-time credential preparation path shared by direct and fallback calls, preserving provider-specific requirements. Compare refresh state with the original credential snapshot. F16. |
| Contracts versus behavior | README's types-only claim (`README.md:157`) conflicts with mutable `StreamFolder::apply` and provider collision handling (`crates/epsilon-contracts/src/stream.rs:62–134`). | Move stream assembly to an implementation layer or explicitly narrow the rule to permit defined helpers. Do not create a crate only to satisfy a slogan. Decide before module moves. |
| Claimed architecture enforcement | `README.md:148–152` claims a compiler-enforced policy; manifests declare a DAG but no allowed-edge checker was established. Root warnings (`Cargo.toml:70–75`) differ from gate denial (`scripts/gate.sh:22`). | Document actual edges, not an artificial linear stack. Add a policy check only if enforcement is required. Keep strict gate flags explicit. |
| Pi commitment versus implementation | ADR-002 promises next-turn and stop callbacks (`docs/design/epsilon-decisions.md:36–40`); inspected core exposes tool hooks, not equivalent configurable callbacks (`crates/epsilon-core/src/lib.rs:684–697,887–895`). | Reaffirm and implement the contract or amend the ADR after review. Do not relabel missing functionality as intentional divergence. |
| Recovery representation | Checkpoint skipped and created-file states both use no blob (`crates/epsilon-tools/src/checkpoint.rs:168–184,261–277`). Restore treats either as deletion. | Typed captured/created/skipped outcomes; skipped is never restorable. Keep the existing preimage store, not competing shadow-Git storage. F15. |

## Functional, incomplete and not established as dead

- Production tool registration wires capture before edit (`crates/epsilon-tools/src/files.rs:479–482`): checkpoint code is not dead. The destructive restore API was located in unit tests, not a production frontend; harden it before exposure.
- `BranchSummary` is declared (`crates/epsilon-contracts/src/session_entry.rs:132`), but targeted searches found no Rust producer and `store.rs:724–745` ignores it in model projection. Treat it as incomplete vocabulary, not approved deletion: it corresponds to the port goal.
- The RPC busy helper is an actual stub (`rpc_mode.rs:117–120`), not an unused helper. Replace its ownership model in F13, rather than deleting the endpoint.
- The largest source file is generated catalog data, not orchestration. Distinguish `catalog_data.rs` (6,701 physical lines) from TUI `app.rs` (6,404), core `lib.rs` (2,559), and CLI `main.rs` (2,194). Split stable responsibilities only after characterization; line count alone does not justify restructuring.
- No dead-code deletion is approved by this audit. Public/exported code and feature-gated paths need usage and compatibility analysis beyond an unsuccessful text search.

## Scope limits

All twelve crates were inventoried; critical assembled paths and selected reference
implementations were traced. This is not a line-by-line certification of the whole
repository. Dependency advisories/provenance, all provider protocols, interactive
TUI accessibility, native Linux/macOS behavior, MSRV, release installation, and
long-running performance remain unverified. The release-qualification roadmap
owns that evidence; a passing helper test cannot close these areas.
