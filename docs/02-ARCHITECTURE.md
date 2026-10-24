# Epsilon architecture

For contributors locating runtime responsibilities in the tree. The
[ADR log](design/epsilon-decisions.md) records design intent; the code and its tests are
the authority. Accepted intent is not proof of implemented behaviour. The pi port remains
incomplete; [`04-STATUS.md`](04-STATUS.md) names the current limits.

## Workspace and dependency graph

The existing Cargo workspace already provides conventional `crates/<name>/src/`
and per-crate `tests/` directories. Unit tests also live beside their modules.
There is no need for another workspace layer or empty benchmark/example directories.

Direct internal production dependencies, as declared in crate manifests and verified with
`cargo tree -p <crate> --edges normal --depth 1` on 2026-10-14:

| Crate | Internal dependencies | Responsibility |
|---|---|---|
| `epsilon-contracts` | none | Shared types/traits; currently also validation and stream folding |
| `epsilon-llm` | contracts | Provider protocols, routing, faux provider, catalog, credentials, retry/fallback |
| `epsilon-session` | contracts | JSONL storage, branch projection, indexing, repair and leases |
| `epsilon-lsp` | contracts | Installed language-server discovery, clients and diagnostics |
| `epsilon-ext` | contracts | Process extension protocol and host |
| `epsilon-core` | contracts, llm, session | Agent state/loop, scheduling, context, compaction, task host, skill discovery and the opt-in built-ins |
| `epsilon-safety` | contracts, core | Permission rules, approvals, environment filtering and sandbox backends |
| `epsilon-tools` | contracts, core, safety | Tool implementations, file observations, mutation exclusion and checkpoints |
| `epsilon-mcp` | contracts, core | MCP connections and tool adaptation |
| `epsilon-tui` | contracts, core, llm, safety, session, **runtime** | Terminal application, editor, dialogs and transcript |
| `epsilon-server` | contracts, core, llm, safety, tools | HTTP/SSE frontend |
| `epsilon-runtime` | contracts, llm, core, tools, lsp, safety, session, ext, mcp | **The one composition point**: a workspace plus a configuration becomes a running agent (`build_runtime`, `resolve_launch`) |
| `epsilon-cli` | every crate above except `epsilon-ext` | Arguments, configuration and dispatch |

> `epsilon-desktop` was a row here until ADR-039 removed it (2026-10-12) with its crate.
> The `epsilon-cli` → `epsilon-ext` manifest edge that this table once carried has since
> been removed; `epsilon-cli` does not depend on `epsilon-ext`.

> **Two rows were missing until 2026-10-04, and the omission inverted the architecture.**
> This table listed neither `epsilon-runtime` nor `epsilon-desktop`, and gave
> `epsilon-cli` the responsibility for "runtime composition". Composition happens in exactly
> one place and it is not the CLI: it is `epsilon_runtime::build_runtime` /
> `resolve_launch` (`crates/epsilon-runtime/src/lib.rs`). That is `AGENTS.md` §2.2, an
> architecture **invariant** — *"A second copy of composition is a defect, not an
> optimisation."*
>
> It mattered because this file is **step 4 of the reading order** `docs/01-OVERVIEW.md`
> prescribes, so a newcomer met an architecture that contradicted the invariant document
> they had just been told to read. The two rows above are derived from the crate manifests,
> not from prose. The staleness warning at the top of this file is real and still applies to
> the rest of it; these rows are the part that has been re-derived.

Cargo checks declared dependencies and rejects cycles; it does not enforce a
separate allowed-edge policy. Libraries must not depend on CLI. The intended
vocabulary-only contracts boundary has existing drift: `epsilon-contracts` also
implements stateful stream folding (`StreamFolder`, `crates/epsilon-contracts/src/stream.rs`)
and provider collision handling. Do not treat that as an approved policy expansion.

## Composition and execution

1. `crates/epsilon-cli/src/main.rs` parses modes/options and delegates every mode to the
   one composition point: `epsilon_runtime::resolve_launch` (`main.rs:1181`) resolves the
   launch, `build_runtime` (`main.rs:1269`, `:1459`, and the serve path) builds the agent.
   `crates/epsilon-cli/src/lib.rs` is a re-export — `pub use epsilon_runtime as runtime` —
   not a second composition. The TUI calls a handful of runtime helpers directly
   (`restore_checkpoint`, `skill_roots`, `goal_command`, `summarize_request`,
   `describe_checkpoint`, `checkpoint_save`, `resumed_model_to_use`) rather than
   reimplementing them; that is the same rule, applied from the other side.
2. `epsilon-core` admits a run and alternates provider turns and tool batches with
   steering and follow-up queues. The loop lives in the crate's own modules — `run_.rs`
   (`run`, `run_inner`, `stream_one_turn`), `session.rs`, `queue.rs`, `tasks.rs` — after
   the 2026-10-14 split of the former 4,700-line `lib.rs`. Core rejects overlapping runs;
   frontend acknowledgments do not yet share atomic admission.
3. `crates/epsilon-core/src/exec.rs` validates and authorizes invocations, plans
   bounded resource-conflict groups and executes tools. Results are committed in
   model order; sequential tools force serial batch execution.
4. `crates/epsilon-llm/src/providers/` owns native protocol adapters. Shared transport
   lives in `providers/mod.rs`; `retry.rs` and `fallback.rs` compose recovery.
5. `AgentEvent` broadcasts feed frontends and extension forwarding. This is not a
   durable replay protocol: TUI, HTTP/SSE and stdio-RPC report lagged-event counts,
    but do not replay the dropped event history.

Pi's two-level loop is the reference; bounded conflict-aware scheduling is an
intentional difference. M85 aligns the follow-up default with pi's one-at-a-time
mode; explicit all-at-once remains available. Async `TurnHooks` implement
`prepare_next_turn` and `should_stop_after_turn` after `TurnEnd` and before queue
polling. ADR-002 (M89) records their run-scoped context and differences from Pi.
The port matrix distinguishes mechanisms from compatibility.

## Tools and execution boundaries

Use the actual API declarations in `crates/epsilon-contracts/src/tool.rs`, not an
approximate signature copied into documentation. `epsilon-tools/src/lib.rs`
registers built-ins and optional services. File/search tools use native host calls;
BashTool receives the subprocess sandbox. MCP and process extensions are separate
adapters, not evidence that arbitrary plugin execution is confined by BashTool.

- `epsilon-safety/src/engine.rs`: matching denies take precedence — **any deny wins
  immediately**, regardless of position (the rule is stated at `engine.rs:60` and
  implemented at `:96-98`); otherwise the later matching allow/ask rule determines the
  decision.
- Path traversal is normalised and symlink-contained (F09, closed by M199: both walkers
  refuse links out of the workspace, and the Windows branch tests the reparse-point
  attribute because a junction needs no privilege and is the form `is_symlink()` does not
  report). Core authorizes the original capability and any derived external-path subject.
  Plan mode retains project rules and denies native write/edit/bash/shell; it does not
  classify arbitrary plugin side effects. [Permissions](user/permissions.md) owns the
  current policy description.
- `epsilon-safety/src/sandbox.rs`: Windows selects Job Objects; other platforms
  probe `bwrap` or report no available backend. Landlock and Seatbelt are not
  implemented backends in this tree.
- Windows Job Objects constrain resources/processes, not filesystem or network
  access. Bubblewrap mounts host root read-only and unshares namespaces; read-only
  does not mean unreadable. Default shell execution refuses without a backend.
- Process waits, cancellation and final pipe drains now share one total deadline
  (F11, closed in three parts: M147 for bwrap, M148 for the unconfined caller's drains,
  M150 for the Windows job-object path). Display caps still apply after unlimited
  parent-output collection (F12, partial: a bounded head is read, the body is still
  buffered whole before its size check).

`epsilon-tools/src/mutation.rs` serializes writers by path. `read_log.rs` scopes
observations to each agent's opaque lifetime token and invalidates sibling knowledge
after writes. Restore renews the token; retired observers are reclaimed. F02 repaired
shared knowledge; resolved-object identity and race-resistant containment followed
in M199.

## Persistence, context and recovery

`epsilon-session/src/store.rs` owns append/read/index/navigation and active-branch
model projection. Compaction appends summaries rather than deleting old messages.
Three defects in this area were found in the audit era and have since been closed,
each with a regression — see the defect ledger for
the evidence rather than taking this list's word for it:

- Earlier summaries accumulating in later compacted projections (F04): closed in M86,
  which clears the projection before installing the latest summary + retained tail, with
  the alternate-branch variant executed in M90.
- Core updating memory independently of a durable mutation that failed
  (`epsilon-core/src/lib.rs`, F07): closed in M122/M124/M133 — the turn loop reports the
  failure, no later request is built from a view that disagrees with the loop, and `serve`
  logs a refused run instead of discarding it. Residual: M122's own sweep of other
  discarded-write sites is not proven exhaustive.
- Complete unsupported final entries entering torn-tail truncation (F08): closed in M108
  (`is_whole_json` is the discriminator; the file stays byte-identical on refusal), and
  M113/M116 replaced the whole-session refusal with keeping a foreign entry and saying so.

Leases live in `epsilon-session/src/lease.rs`, with per-acquisition ownership tokens so a
victim's `Drop` cannot delete a thief's lock (M294) and an `append` that refuses on a
stolen lease (M312, ADR-028). The TUI's `/fork` branches through the store
(`fork_branch_at`) and, since the 2026-10-13 picker slice, opens a message picker rather
than copying a file. `SessionStore::branch_with_summary` (`store.rs:1749`) is the
branch-summary producer — **correct, tested, and still without a production caller**,
which is a capability gap rather than a missing primitive.

`epsilon-core/src/prompt.rs` assembles stable/context/volatile sections;
`compact.rs` owns context selection and summary construction. Deterministic assembly
supports cache reuse but cannot guarantee provider cache hits. Summary acceptance and
cancellation (F06) and persistence of local cancellation salvage (F17) were closed in M86:
a pass commits only on a non-failure stop reason, usable text and a successful durable
append, and the abort arms push their terminal message and emit `MessageEnd` like a
completed turn. Byte-identical replay of every provider request is not established.

`epsilon-tools/src/checkpoint.rs` records preimages before edits, and the user-facing
undo arrived with M277: `/restore <path>` previews, then applies on a trailing `yes`
(ADR-023), with the content-verification guard from M104 and the agent-created-file
protection from M298 (ADR-026). The ceiling, stated rather than implied: `/restore`
reverts one file at a time and draws no diff, so a turn's whole footprint is not one
undo away. Ledger-append serialization and blob-path validation remain on the finding's
list. Do not replace the store with a second snapshot subsystem before its existing
contracts work.

## Frontends and integrations

| Surface | Implementation and limits |
|---|---|
| TUI | `epsilon-tui/src/app.rs` orchestrates editor/picker/markdown/state modules. Ownership is concentrated; split stable responsibilities only after characterization and baseline repair. |
| HTTP | `epsilon-server/src/lib.rs` owns routing, SSE and request guards. Admission and dropped-event recovery remain open. |
| RPC | `epsilon-cli/src/rpc_mode.rs`: prompt, steer, followup, approvals, approve, ping. Not pi's envelope or full cancellation/state/session surface. |
| Extensions | `epsilon-ext/src/manifest.rs` and `host.rs` define the implemented process protocol, not pi's complete extension SDK. |
| MCP | `epsilon-mcp` adapts stdio and StreamableHTTP servers, registers tools and enforces a per-server `timeout_secs` (default 120). |
| Integrations | `epsilon-runtime/src/integrations.rs` holds the curated optional components. Today that is REA: pinned package, platform launch wrapper, 900-second deadline. It is still an MCP server, not a second extension system. |
| Skills | `epsilon-core/src/skills.rs` discovers every harness convention; `builtin_skills.rs` holds the twelve opt-in bundled procedures and installs one as an ordinary `SKILL.md` only on request. |
| LSP | `epsilon-lsp` discovers user-installed servers and appends bounded-count error reports. Monotonic versions, total deadlines, cancellation-safe framing and freshness remain open (F14). |

Provider fixtures and compilation do not establish every catalog model or OAuth flow.
The SSE framing defect is closed (F03, M137: one HTTP chunk holding several frames no
longer loses all but the first); OAuth refresh orchestration across login/fallback paths
remains open (F16, last re-checked at HEAD by M134 — the 2026-10-14 `needs_oauth_refresh`
guard removed one unconditional-refresh site, not the orchestration question).
User-facing configuration and commands belong in the
[user guides](05-USER-GUIDE.md), not duplicate API examples here.

## Maintenance and correction record

[AGENTS.md](../AGENTS.md) owns contributor rules and verification commands.
Internal inconsistencies records competing
ownership models; the roadmap orders their repair.

The previous guide mixed proposals with implementation: Landlock, Seatbelt,
tree-sitter shell analysis, complete callback parity, branch summaries, exact replay,
broad extension features and universal cleanup were stated as existing. Those
claims and approximate Rust API snippets are removed. Design rationale remains in
the ADR log and Git history; removing an unsupported claim is not deleting a feature
or silently changing the port target. This guide makes no new upstream version,
performance, platform-certification or license-provenance claims.
