# Epsilon architecture — re-derived draft

**Status: draft for review. Not indexed in `docs/README.md`; a human decides whether this
replaces `epsilon-architecture.md` or whether that file is deleted instead.**

Every claim below was read out of the tree on **2026-09-25** (commit `4fbca81`, working tree
otherwise unmodified). Method, and nothing else:

- crate list and dependency edges: `cargo metadata --no-deps --format-version 1` plus each
  crate's `Cargo.toml` and the workspace `[workspace.dependencies]`;
- responsibilities: each crate's `src/lib.rs` module list and public re-exports;
- runtime claims: the named file opened at the named line.

Where a line number is marked **drift risk**, the cited line was located but the surrounding
function body was not re-read; treat the number as a pointer, not a quotation. No build, test,
clippy or fmt run was performed for this document — it asserts nothing about green gates.

**The tree was not clean.** `crates/epsilon-tools/src/read_log.rs` (+197 lines) and
`crates/epsilon-tools/src/files.rs` carry uncommitted work belonging to another item
(`docs/milestones/M135-edit-does-not-license-a-whole-file-write.md`, untracked). Every
`read_log.rs` and `files.rs` line number in §4.3 describes the **working tree**, not `HEAD` —
`verify` sits at `read_log.rs:376` today and at `:323` in `HEAD`. `docs/design/epsilon-architecture.md`
itself is unmodified, so the corrections in §6 are stated against committed code.

---

## 1. The twelve crates

| Crate | What it owns (from its own module list) | Non-obvious bits a reader will get wrong |
|---|---|---|
| `epsilon-contracts` | `agent_event`, `content`, `diagnostics`, `ids`, `lagged_notice`, `message`, `session_entry`, `session_state`, `stream`, `tool`, `turn` | Not types-only. `stream::StreamFolder` is stateful folding (`stream.rs:66`, `apply` at `:75`); `session_entry::SessionLine::parse`/`deserialize_kind` validate; `agent_event::TurnEndReason::merge_sticky` is behaviour (`agent_event.rs:32`). Zero internal deps, as the architecture law requires. |
| `epsilon-llm` | `auth`, `catalog`, `config_value`, `custom`, `fallback`, `faux`, `http`, `oauth`, `oauth_login`, `providers`, `retry`, `retry_stream`, `sanitize`, `stream_fn` | `providers/` and `fallback` are `#[cfg(feature = "real")]` (`lib.rs:15,21`); offline is the default build of the crate. `providers/` holds eight per-family wire adapters; `providers/mod.rs` owns the shared streaming engine. |
| `epsilon-session` | `discover`, `import`, `lease`, `repair`, `search`, `store` | `store.rs` is the only durable writer; the human transcript is the JSONL file, the model view is *derived* (`store::SessionStore::model_view`, `store.rs:1448` — drift risk). |
| `epsilon-core` | `agents`, `cache_stats`, `compact`, `config`, `exec`, `gate`, `guards`, `ignore`, `lib`, `paths`, `prompt`, `references`, `registry`, `skills`, `testing`, `untrusted`, `verify` | Owns the turn loop, the transcript, both queues, the event bus, and the delegated-task host. `gate.rs` defines the `PermissionGate` seam that `epsilon-safety` implements — the seam lives above the implementation. |
| `epsilon-tools` | `checkpoint`, `edit_match`, `files`, `find`, `image`, `lib`, `mutation`, `read_log`, `search`, `shell`, `skill`, `task`, `todo`, `util`, `webfetch` | `read_log` and `mutation` are **private** modules (`lib.rs:14,15`); the read-before-write guard is not part of any public API. `files.rs` is their only consumer. |
| `epsilon-safety` | `channel_ui`, `engine`, `env`, `escalation`, `gate`, `lib`, `sandbox`, `wildcards` | `gate::RulesetGate` is the bridge; it depends on core only for the `PermissionGate` trait (`gate.rs:8`). The only `unsafe` user in the tree is `sandbox::jobobject`. |
| `epsilon-ext` | `bin`, `host`, `manifest`, `tool` | Child processes over line-delimited JSON on stdio; `INVOKE_TIMEOUT_SECS = 10`, `CRASH_BUDGET = 2` (`lib.rs:24,26`). No in-process plugin execution. |
| `epsilon-mcp` | `bin`, `lib`, `stdio` | `lib.rs` owns config + the `Connector` trait + the `bridge`; `stdio.rs` is the rmcp implementation. Also ships a fixture binary gated on rmcp server features. |
| `epsilon-lsp` | `bin`, `client`, `framing`, `report`, `server`, `service` | User-installed servers only. `MAX_REPORTED = 20`, ERROR only (`report.rs:60`). Talks to the rest of the tree through `contracts::diagnostics::DiagnosticsProvider`, implemented at `service.rs:216`. |
| `epsilon-tui` | `app` (+ `app/{event,keys,golden,tests}`), `at_files`, `bench`, `clipboard`, `editor_ops`, `fuzzy`, `git`, `keybindings`, `kill_ring`, `latex`, `lib`, `markdown`, `picker`, `render_cache`, `scroll`, `state`, `trust_store`, `ui`, `undo`, `wrap` | `state::UiState` is the pure reducer; `app::run` owns the terminal; `app::event::drive` owns the loop. `app.rs` is 4238 lines and `ui/` is 12 modules. |
| `epsilon-server` | `lib`, `openapi` | axum router (`lib.rs:193`), SSE for `AgentEvent`, REST for prompt/steer/permissions. Composition stays in the CLI. |
| `epsilon-cli` | `doctor`, `lib`, `main`, `rpc_mode`, `runtime` | The only crate with a `[[bin]]` (`epsilon`) *and* a lib. Parses modes (`main.rs:212`), dispatches (`main.rs:1427-1442`), and composes the runtime (`main.rs:1672-1794`). |

Feature wiring, because it changes what "the product" is: `epsilon-cli`'s `default = ["real"]`
and `real = ["epsilon-llm/real", "epsilon-core/real", "epsilon-tui/real"]`
(`crates/epsilon-cli/Cargo.toml`); `epsilon-core`'s `real = ["epsilon-llm/real"]`; `epsilon-tui`'s
`real = ["epsilon-core/real"]`. Offline is `--no-default-features` on the CLI, which drops
`epsilon-llm`'s optional `reqwest` — but **not** the one `epsilon-tools` declares
unconditionally, so reqwest still links.

---

## 2. The real direct dependency graph

Normal (non-dev) internal edges, exactly as `cargo metadata` reports them. "Used in" is a
grep of non-test source for `epsilon_<name>`.

| Crate | Direct internal dependencies | Used in non-test source |
|---|---|---|
| `epsilon-contracts` | — | — |
| `epsilon-llm` | contracts | yes |
| `epsilon-session` | contracts | yes |
| `epsilon-lsp` | contracts | yes (1 site, `service.rs:216`) |
| `epsilon-ext` | contracts | yes |
| `epsilon-core` | contracts, llm, session | yes |
| `epsilon-safety` | contracts, core | yes (core: 1 site, `gate.rs:8`) |
| `epsilon-mcp` | contracts, core | yes (core: 2 sites, `lib.rs:74`, `stdio.rs:123`) |
| `epsilon-tools` | contracts, core, safety, lsp | yes |
| `epsilon-tui` | contracts, core, llm, safety, session | yes |
| `epsilon-server` | contracts, core, llm, safety, tools | contracts/core/safety yes; **llm and tools only inside `#[cfg(test)] mod tests`** (`lib.rs:461`, `:863`, module opens at `:457`) |
| `epsilon-cli` | contracts, core, ext, llm, lsp, mcp, safety, server, session, tools, tui | all eleven |

Dev-dependency edges that do not appear above:

| Crate | Dev-depends on | Note |
|---|---|---|
| `epsilon-safety` | llm, tools | A dev-only cycle: `epsilon-tools` depends on `epsilon-safety` normally, and safety's tests use tools. Cargo permits it; it means safety's test build compiles tools. |
| `epsilon-cli` | portable-pty, vt100 | Declared in the workspace table as test-only for the whole tree; no shipping crate links them. |

Shape, stated plainly: **it is not a stack.** `core` sits under five siblings at once
(`safety`, `tools`, `mcp`, `tui`, `server`), and `tools` reaches down to `lsp`. A layered
picture of this graph is wrong even when every edge in it is legal.

---

## 3. The allowed-edge policy, checked

### 3.1 What the repo states

From `AGENTS.md` §Architecture law:

> Cargo enforces declared dependencies and rejects cycles; it does not enforce our
> allowed-edge policy. The actual direct dependency graph is documented in
> `docs/design/epsilon-architecture.md`. CLI composes sibling crates; core depends on
> contracts, llm and session; LSP communicates through contracts. No library may depend on
> CLI. New edges require review rather than relying on an illustrative layer diagram.

Four testable clauses:

| # | Clause |
|---|---|
| P1 | CLI composes sibling crates |
| P2 | core depends on contracts, llm and session |
| P3 | LSP communicates through contracts |
| P4 | no library may depend on CLI |

### 3.2 Result: **all four hold on 2026-09-25. No violating edge exists.**

| Clause | Evidence |
|---|---|
| P1 | `epsilon-cli` depends on all eleven other crates, all eleven used in non-test source (§2). |
| P2 | `epsilon-core`'s internal deps are exactly `contracts`, `llm`, `session` — no fourth. |
| P3 | `epsilon-lsp` depends on `contracts` alone. The loop↔LSP contract is `contracts::diagnostics::DiagnosticsProvider`; `epsilon-lsp` implements it (`service.rs:216`) and `epsilon-tools` constructs the service (`lib.rs:119,125`). No `lsp → core/tools` edge. |
| P4 | `cargo metadata` over all 12 packages: zero dependencies on `epsilon-cli` in any kind, including dev. |

Two further points that are policy-adjacent rather than violations:

- **`unsafe_code = "forbid"` is not workspace-wide in effect.** Ten crates use
  `[lints] workspace = true`. `epsilon-lsp` replaces that table with
  `[lints.rust] missing_docs = "warn"` only, and `epsilon-safety` replaces it with
  `missing_docs = "warn"`, `unsafe_code = "allow"`, plus `clippy::all` and
  `undocumented_unsafe_blocks`. A package-level `[lints]` table *replaces* the workspace one
  rather than merging, so neither crate inherits `unsafe_code = "forbid"`. Safety's manifest
  documents why (job-object FFI); `epsilon-lsp`'s manifest says nothing about it.
- **The contracts boundary is the documented drift, not a violation.** P1–P4 do not mention
  it. `AGENTS.md` already records that `epsilon-contracts` carries validation and stream
  folding; that is still true, and `contracts/src/lib.rs:4-5` still claims "zero business
  logic".

---

## 4. Runtime responsibilities

### 4.1 How a prompt becomes a turn

`epsilon-core` owns the loop. Nothing else does.

| Step | Where | Line |
|---|---|---|
| CLI picks a mode and dispatches | `crates/epsilon-cli/src/main.rs` | `1427-1442` |
| CLI composes the runtime: tool registry, MCP attach, gate, `Agent::new`, session store, `attach_session_store` | `crates/epsilon-cli/src/main.rs` | `1672`, `1676`, `1741`, `1742`, `1749`, `1794` |
| TUI submits: spawns the run on a tokio task | `crates/epsilon-tui/src/app.rs` | `2613` |
| `Agent::run` admits one run (atomic swap, `AlreadyRunning` otherwise), installs a `RunGuard` that clears state on every exit, mints the run token, closes dangling tool calls, appends the prompt, emits `AgentStart`/`AgentEnd` | `crates/epsilon-core/src/lib.rs` | `1280-1337` |
| `run_inner` is the loop: `'outer` over the follow-up queue | same | `1401`, `1442`, `1813` |
| inner `while has_more_tools \|\| !pending.is_empty()` | same | `1445` |
| drain point: queued messages pushed before the next model call | same | `1468-1472` |
| compaction pressure check (once per run) | same | `1479-1493` |
| one provider turn | same → `stream_one_turn` | `1510-1517`, impl `2273` |
| overflow recovery: pop the failed attempt, compact, retry | same | `1538-1571` |
| tool batch | same → `exec::execute_batch` | `1597-1607` |
| results committed, then `TurnEnd` | same | `1654-1685` |
| batch execution itself: preflight, gate, bounded pool, model-order commit | `crates/epsilon-core/src/exec.rs` | `260`, gate at `330-366` |
| prompt assembly (stable / context / volatile tiers) | `crates/epsilon-core/src/prompt.rs` | `114`, called at `lib.rs:2333` |
| compaction: cut point, summary construction, durable entry | `crates/epsilon-core/src/compact.rs` | `73`, `124`; commit at `lib.rs:1985`, append at `2150` |

Two claims worth being precise about, because the old guide got one of them wrong:

- `Agent::run` **rejects** a concurrent run rather than queueing it (`lib.rs:1281-1283`).
  Admission *reporting* is per-frontend and pre-reserved: the HTTP surface takes a
  `run_depth` slot before replying 202 and returns 409 to the loser
  (`crates/epsilon-server/src/lib.rs:326-341`); RPC does the same through
  `admit_prompt` (`crates/epsilon-cli/src/rpc_mode.rs:100`, used at `300`).
- `TurnEndReason` is sticky — `max_tokens` survives a later clean turn
  (`contracts/src/agent_event.rs:32-47`).

### 4.2 Where session state lives, and who writes it

| Fact | Where | Line |
|---|---|---|
| One append-only JSONL file per session; each line a `SessionEntry` with monotonic `seq` and a parent link (branch tree) | `crates/epsilon-session/src/lib.rs` | `1-6` |
| The model-visible history is a **derived** view where compaction entries shadow the ranges they summarize | same | `5-6` |
| `SessionStore::open` / `open_in` / `open_child_in` | `crates/epsilon-session/src/store.rs` | `392`, `398`, `411` — drift risk |
| `SessionStore::append` is the single write primitive | same | `926` — drift risk |
| `SessionStore::model_view` builds the projection the provider request reads | same | `1448` — drift risk |
| Leases | `crates/epsilon-session/src/lease.rs` | — |
| The CLI creates the store and hands it to the agent | `crates/epsilon-cli/src/main.rs` | `1749`, `1794` |

**Who writes, in the product path:** `epsilon-core`, and only `epsilon-core`.

| Write | Where | Line |
|---|---|---|
| every transcript message | `Agent::persist` → `store.append(EntryKind::Message)` | `crates/epsilon-core/src/lib.rs:1039-1050` |
| in-memory transcript push, memory and file together | `Agent::push_message` | `1105-1110` |
| discard the last message (overflow retry) | `Agent::pop_last_message` | `1086-1098` |
| compaction entry, appended *before* runtime history is replaced | `compact_now` | `2150-2160` |
| delegated child: a fresh child store plus a `Custom` link | `attach_child_session` | `2701-2704` |

The failure discipline is the point: a refused durable write returns `RunError` and latches
`durable_write_failure` (`1065-1078`), and `try_context_view` (`1239` — drift risk) consults
that latch before building a request, so a later request is never built from a view the loop
disagrees with. `epsilon-session`'s own `repair` (`repair.rs:157`) writes too, but only on the
open-a-crashed-file path, not during a run.

### 4.3 The read-before-write guard for filesystem tools

Lives in **`epsilon-tools`**, in the private module `read_log`. Consulted by exactly one
caller: `epsilon-tools/src/files.rs`.

| Piece | Where | Line |
|---|---|---|
| The module's own statement of what it is: a *known-change* guard, not a lock; an external writer between check and write still wins | `crates/epsilon-tools/src/read_log.rs` | `1-14` |
| Private — no other crate can call it | `crates/epsilon-tools/src/lib.rs` | `15` |
| `verify(owner, path, full_overwrite) -> Result<(), Stale>` | `read_log.rs` | `376-408` |
| Three refusals: `Modified`, `NeverRead`, `PartialRead` | `read_log.rs` | `67-87` |
| Coverage, not a bool: which lines a paged read showed, whether it reached EOF, whether a single over-budget line was the stop | `read_log.rs` | `166-210` |
| `record_read` / `record_write` / `record_edit` | `read_log.rs` | `218`, `276`, `323` |

Consultation sites, all in `crates/epsilon-tools/src/files.rs`:

| Tool | Guard call | Line |
|---|---|---|
| `read` | `record_read(scope, path, coverage)` after the page is assembled | `229` |
| `write` | `verify(scope, path, /*full_overwrite=*/ true)` **inside** the per-path mutation queue, before the checkpoint record and the bytes move | `356` (queue entered at `346`) |
| `edit` | `verify(scope, path, /*full_overwrite=*/ false)` inside the same queue | `531` |
| both, after the write | `record_write` / `record_edit` | `368`, `555`, `689` |

Two supporting mechanics:

- **Serialization**: `mutation::with_file_mutation_queue` (`mutation.rs:31`) keys a process-global
  `HashMap<PathBuf, Arc<Mutex<()>>>` on the lexically normalized absolute path — no `realpath`,
  so path identity is lexical, and two spellings of one file are two locks.
- **Scope**: observations are keyed by an opaque `Arc<()>` lifetime token, not a session name.
  `Agent` owns it (`core/src/lib.rs:485`), hands a clone into every `ToolContext`
  (`core/src/exec.rs:682-688`, field declared at `contracts/src/tool.rs:309`), and
  `restore_history` replaces it with a fresh token, which is what invalidates everything the
  model thought it had seen (`core/src/lib.rs:777-781`). Delegated children share mutation
  locks, not read history.

`edit` deliberately passes `full_overwrite: false`: an edit matches exact text the model had
to have seen, and a targeted edit carries the file's completeness flag forward rather than
promoting it (`read_log.rs:20-22`).

### 4.4 How the TUI gets its events

One broadcast channel from `Agent`; the TUI is a reducer over it.

| Step | Where | Line |
|---|---|---|
| `Agent::subscribe()` — a `tokio::sync::broadcast` receiver | `crates/epsilon-core/src/lib.rs` | `889-890` |
| `Agent::emit` | same | `1193` |
| the event vocabulary, and the doc line that says every UI is a reducer over it | `crates/epsilon-contracts/src/agent_event.rs` | `1` |
| TUI entry: `app::run` initializes the terminal and hands off to `drive` | `crates/epsilon-tui/src/app.rs` | `2697-2710` |
| `drive` subscribes, creates three `mpsc` side channels (notices, OAuth, live-fetch) and a `crossterm::EventStream` | `crates/epsilon-tui/src/app/event.rs` | `43-48`, `94` |
| bounded input drain, then bounded agent-event drain (`try_recv`, `Lagged` → a notice) | same | `123-155` |
| frame ceiling: draw at most once per 33 ms and only when something changed | same | `163-178`; constants at `app.rs:71,76,83` |
| biased `select!` — input first, then approvals, then agent events, then the three side channels, then a frame-due tick | same | `205-249` |
| each event goes through `apply_with_scroll`, which pins the scroll window and restales finished cards | `crates/epsilon-tui/src/app.rs` | `2728-2741` |
| the pure reducer itself | `crates/epsilon-tui/src/state.rs` | `753` — drift risk |

Lagged subscribers are told, not silently truncated: the TUI pushes a notice
(`event.rs:220`), the server emits an SSE event named `lagged` carrying the shared
`contracts::LaggedNotice` (`server/src/lib.rs:448`), and RPC writes a `{"lagged":…}` line
(`rpc_mode.rs:376-381`). The dropped events themselves are **not** recoverable — only the count
survives. Extension forwarding is the CLI's: `spawn_event_forwarder(agent.subscribe(), ext_hosts)`
(`main.rs:1795`).

---

## 5. What this draft does not establish

Things a reader would reasonably infer from a dependency diagram, which the code does not
guarantee:

1. **The policy is not enforced anywhere.** No test, script, lint or CI step reads the
   dependency graph. `.github/workflows/ci.yml` runs fmt, clippy, `cargo test --workspace`, the
   `real` compile and wire-fixture checks, `--demo`, the pty terminal test on two platforms, and
   an MSRV `cargo check`. A new illegal edge compiles and passes every gate; §3.2 is a
   hand-check, and it will rot the same way this file's predecessor did.
2. **`AgentEvent` is not a durable log.** A `Lagged` notice carries a count, not the events.
   Any frontend can miss `MessageEnd` for a card it never saw start. Nothing re-synchronises a
   lagged subscriber against the transcript.
3. **Durable requests are not recorded.** `Agent::persist` appends
   `EntryKind::Message { message, api_request_sha256: None }` (`core/src/lib.rs:1044-1047`).
   Byte-identical replay of a provider request cannot be reconstructed from a session file.
4. **The compaction entry is not a deletion.** Old messages stay in the file and are shadowed in
   the projection, so file size and context size are different numbers and will diverge further
   with every compaction.
5. **The read-before-write guard is best-effort by construction.** It runs inside the mutation
   queue, so no *harness* writer interleaves — but a `task` child, a background script or the
   user's editor can rewrite the file between the check and the write, and the module says so
   (`read_log.rs:11-14`). It is also process-local: nothing about it survives a restart, and the
   lexical path key means two spellings of one path are two entries.
6. **The graph is not a layering.** `core` is a dependency of five crates, and `tools` reaches
   `lsp`. A "core owns the loop, everything sits above it" reading is a description of the loop,
   not of the graph.
7. **`epsilon-server`'s `llm` and `tools` edges are test-only** as written (§2). If someone
   later moves a `use` out of the test module, the graph changes and no gate notices.
8. **Compaction, compaction recovery and the doom-loop brake have real failure paths** whose
   residual limits are tracked in `audit/02-defect-ledger.md`, not here. This draft says where
   the code is; it does not say which defects are open today.
9. **No gate result is asserted anywhere above.** This document was produced without building or
   testing. "Verified on 2026-09-25" means *read on 2026-09-25*.

---

## 6. Corrections to the old guide

`docs/design/epsilon-architecture.md` stays in the tree, so readers will hit both. What
survived, and what did not.

### Survived — re-verified, still true

| Old claim | Re-verified at |
|---|---|
| The full internal dependency table, all twelve rows | identical to `cargo metadata`; §2 above |
| "Libraries must not depend on CLI" | holds; no package depends on `epsilon-cli` in any kind |
| "core depends on contracts, llm and session" | exact |
| The contracts types-only drift, `stream.rs:62–134` | still true, wrong line range: `StreamFolder` is `stream.rs:66`, `apply` runs to `:214`, the DeepSeek/Anthropic index-0 collision is `:93` |
| `epsilon-safety` engine: matching denies take precedence, otherwise the later matching allow/ask wins | `engine.rs:83-98`, deny returns at `:90-93`, last match overwrites at `:94` |
| `epsilon-tools/src/mutation.rs` serializes writers by path | `mutation.rs:31`; the file's own header confirms the lexical (no-`realpath`) key |
| `read_log.rs` scopes observations to an opaque lifetime token | `read_log.rs:24-26`; token minted at `core/src/lib.rs:613`, renewed at `:777-781` |
| "The TUI's `/fork` copies a session file without switching to it" | `app.rs:1568` — `std::fs::copy`, and the notice says "open with `--session`" |
| Landlock and Seatbelt are not backends in this tree | `sandbox.rs:287-299`: JobObject on Windows, else probe `bwrap`, else none |
| "nothing in the product calls `restore`" | `CheckpointStore::restore` (`checkpoint.rs:322`) is called only from that file's own tests |
| M85's follow-up default is pi's one-at-a-time | `LoopLimits::default`: `steering_mode` and `followup_mode` are both `QueueMode::OneAtATime` (`core/src/lib.rs:196,200`) |
| RPC's op set is prompt / steer / followup / approvals / approve / ping | `rpc_mode.rs:296-327` |
| LSP appends bounded-count error reports | `report.rs:60,76` — ERROR only, at most 20 per file |
| "No Rust `BranchSummary` producer was found" | still true, and now nameable: `store::branch_with_summary` (`store.rs:1216`) exists but is called only from `epsilon-session`'s and `epsilon-core`'s tests |
| `AgentEvent` broadcasts feed frontends and extension forwarding | `main.rs:1795` |

### Wrong or drifted — the correction is in this file

| Old claim | What is actually there |
|---|---|
| `core/src/lib.rs:900–934` admits a run | Those lines are `steer_queue_len`, `follow_up` and `attach_session_store`. Admission is `Agent::run` at **`lib.rs:1280-1337`**. |
| `core/src/lib.rs:1037–1261` alternates provider turns and tool batches | That range is `persist`, `durable_write_failed`, `pop_last_message`, `history`, `push_message` and the accessors. The alternation is **`run_inner`, `lib.rs:1401-1825`** (outer loop `1442`, inner `1445`, tool batch `1597-1607`). |
| "Core rejects overlapping runs, but frontend acknowledgments do not yet share atomic admission" | Both frontends now reserve admission **before** replying: `run_depth` fetch-add with a 409 for the loser (`server/src/lib.rs:322-341`, test at `:632`), and `admit_prompt` (`rpc_mode.rs:100`). |
| "RPC currently skips lagged broadcasts" | RPC reports them: a `{"lagged": <notice>}` frame on `Lagged` (`rpc_mode.rs:376-381`, `lag_notice` at `:227`). |
| "`epsilon-server` … Admission and dropped-event recovery remain open" | Both closed on 2026-09-25: atomic admission (above) and the `lagged` SSE event (`server/src/lib.rs:448`, `lagged_data` at `:429`). |
| `epsilon-tui/src/app.rs` "orchestrates editor/picker/markdown/state modules" | Understated rather than wrong: `app.rs` is 4238 lines and now owns submodules `app/{event,keys,golden,composer_viewport_tests,tests_support,tests}` plus a 12-module `ui/` tree. The loop is `app::event::drive` (`event.rs:31`). |
| "`/tree` shows the active branch" | `/tree` is an alias for `/timeline` (`app.rs:1692-1693`); both render `store.branch_entries()` (`store.rs:1062` — drift risk) through the same picker. Neither filters to the active branch. |
| `epsilon-contracts` `stream.rs:62–134` | See above: the claim holds, the range does not. |
| "`epsilon-llm/src/providers/` owns native protocol adapters. Shared transport lives in `providers/mod.rs`" | Correct, with the qualifier the old text omits: `providers/` and `fallback.rs` are behind `#[cfg(feature = "real")]`, so an offline build has neither. |
| The guide's own header ("dated snapshot … everything else has not been re-verified") | Accurate, and this draft is the re-derivation it asked for. The old file is not superseded by this one until a human says so; `docs/README.md:43-48` still routes readers to it, and this draft is deliberately not in the index. |

---

## 7. Re-deriving this yourself

```sh
cd "C:/Users/Hp/Desktop/My Agent Space/Epsilon"

# crates + edges, no build, no artifacts
cargo metadata --no-deps --format-version 1

# the twelve manifests and the workspace table
cat Cargo.toml crates/*/Cargo.toml

# who depends on the CLI (expect: nothing)
cargo metadata --no-deps --format-version 1 \
  | python -c "import json,sys; m=json.load(sys.stdin); print([p['name'] for p in m['packages'] if any(d['name']=='epsilon-cli' for d in p['dependencies'])])"

# no gate enforces the policy
grep -rn 'run:' .github/workflows/ci.yml
```

If §3.2 ever comes back violated, the fix belongs in a test or a CI step that reads
`cargo metadata`, not in this document.
