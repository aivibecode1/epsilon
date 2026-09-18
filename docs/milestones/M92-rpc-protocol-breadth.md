# M92 — RPC-02 protocol breadth (correlated replies, abort/state, dropped events)

## What

The RPC-02 half of `audit/09-revalidation.md` row 4 (finding `<RPC-02>`, acceptance list in
`audit/evidence/revalidation-core-review.md:73-81`), implemented from the verified brief in
`agents/memory/rpc02-protocol.md`. The finding's headline was that pi's RPC client resolves only
frames tagged `type:'response'` whose `id` matches a pending request
(`pi/packages/coding-agent/src/modes/rpc/rpc-client.ts:508-522`) and reads the payload from
`data` (`:540-590`), while epsilon's wire had no id, no command tag and correlated by arrival
order — so a pi client could not use this transport unchanged, and an editor could not abort or
query state at all even though core backs both.

**A. Correlated replies** (`crates/epsilon-cli/src/rpc_mode.rs`). Inbound frames gain an
optional `id`, and pi's `type` tag is accepted as an alias for `op` (`rpc-types.ts:20-73` —
pi tags the same frame `type`). Replies now carry pi's envelope — `type:"response"`,
`command`, `id` (present exactly when the request carried one), `success`, and `data`|`error` —
while keeping epsilon's original `ok`/`error` keys with the same shapes, both on success
carrying the same payload, so clients written against the first wire keep working.

**B. A testable dispatch path.** The per-line match that lived inside `run()` (which owns
stdin/stdout, the reason the old code could only be tested through a pipe) is now
`async fn dispatch_line(&str, &Arc<Agent>, &Arc<ChannelApprovalUi>) -> String`; the loop is a
read/write shell around it.

**C. Three new ops.** `abort` (reports core's `Agent::abort_current_run` answer in-band), `get_state`
(the shared `SessionState` payload), `set_thinking_level` (sets and reads the level back; pi's
`xhigh` is accepted as an input alias for the canonical `x_high` — see Limits). An op
this build does not implement — the whole deferred family — fails explicitly with
`unknown op: <name>` and the echoed id, rather than a serde parse error.

**D. The dropped-event gap.** The event pump's silent `RecvError::Lagged(_) => continue` is
replaced by a frame matching serve mode's: HTTP emits an SSE event named `lagged` whose data is
`{"dropped":n}` (the `events` handler, `crates/epsilon-server/src/lib.rs:434-456`); the line
protocol's keyed frames make that `{"lagged":{"dropped":n}}`, the same data object. Both
surfaces build the notice from one type — `epsilon_contracts::LaggedNotice` — so the two cannot
drift apart, and each side's test pins the bytes.

**E. One getter in core** (`crates/epsilon-core/src/lib.rs`): `pub fn model_id(&self) -> String` —
the read side of `set_model_id`, needed by both `get_state` payloads.

**F. Serve-mode breadth** (`crates/epsilon-server/src/lib.rs`): `POST /abort` (202 when a run was
cancelled, 409 when none was; core's answer reported in the body) and `GET /state` (the same
payload as the stdio `get_state`, one shared struct). `POST /prompt`'s 202/409 bodies became JSON:
they keep the old sentence under `detail` and echo the new optional `request_id`.

**G. The shared payload** (`crates/epsilon-contracts/src/session_state.rs`): `SessionState`, a
plain serde struct with docs and no behaviour — the crate stays types-only by rule, and putting
it there avoids a `cli -> server` edge. Both surfaces build it field by field from the live
`Agent`.

**H. OpenAPI** (`crates/epsilon-server/src/openapi.rs`): `/abort`, `/state` and `/openapi.json`
registered, the `SessionState` schema added, prompt's `request_id` documented. The drift test
became real: `router()` now mounts exactly one `route_table()`
(`crates/epsilon-server/src/lib.rs:173-189`) and `spec_paths_match_router` compares the
document's path *set* against that same table — a path registered without a spec entry (or
described without a route) fails the test. What it still does not compare (path *sets* only,
not methods/handlers/schemas) is recorded in Limits.

**I. Tests**, all offline (`FauxProvider`, `SlowProvider`; no network): id echo per request in
arrival order; pi's `type` tag dispatching; a frame carrying *both* tags (no duplicate-field
error, `op` wins); legacy no-id requests unchanged; unknown op failures with echoed ids on both
wire dialects, including a pi-shaped frame carrying realistic payload fields; malformed lines
keeping the legacy `error` key without an invented correlation; abort on an idle agent returning
core's `false`; abort during an admitted run returning `true` and ending the run (proven by its
terminal event); `get_state` values equal to the live agent's; `set_thinking_level` round-trip
from a *different* starting level (read-back, not echo) plus pi's `xhigh` spelling end-to-end;
the lagged frame's exact bytes on both surfaces; the OpenAPI `thinking_level` enum pin (emitted
set, alias excluded); and over the real router — abort ending a `SlowProvider` run, abort with
no run reporting conflict (body-less and with a `request_id`), the 409 prompt echoing its own
`request_id`, and the `GET /state` shape.

## Key decisions

- **The envelope is pi's, extended rather than replaced.** `data` (pi) and `ok` (epsilon) carry
  the same value on success rather than one being chosen: the finding rated backward
  compatibility for current clients MED, the brief makes it a hard requirement, and a translation
  layer on either side would have to know which wire it is talking to. `error` is the same key in
  both dialects, so failures needed no duplication.
- **The `id` is echoed only when it was sent.** A first-wire client's reply is the legacy object
  with new keys added — never a re-keyed one — and an unparseable line carries no id, because
  guessing a correlation is worse than the honest refusal.
- **Unknown ops are a named variant, not a parse failure.** `#[serde(other)]` on an internally
  tagged enum gives the dispatch path the op's name, so the failure is
  `unknown op: set_model` with the id echoed, instead of serde's variant list. This is also the
  explicit-failure half of the finding's acceptance ("unsupported operations must fail
  explicitly") without half-wiring any deferred op.
- **The `type` alias is a tag normalization, not a second enum.** serde has no alias for an
  internally tagged enum's *tag*, so the dispatcher points `type` at `op` before parsing when
  `op` is absent (a frame carrying both keeps `op`). One enum reads both dialects; the existing
  deserialization test still covers the `op` path.
- **Abort reports core's answer, and the two surfaces keep their own vocabularies.** Both call
  the same `Agent::abort_current_run`; RPC's reply carries `{"aborted":bool}` (an idle agent
  answers `false`, not a fabricated success), while HTTP maps the same bool to 202/409 — HTTP
  keeps statuses rather than importing RPC's in-band channel. Both bodies also name the outcome
  in `detail`, so "no run is active" is readable without parsing.
- **The 202/409 prompt bodies keep their sentences.** They became JSON only because they must
  carry `request_id`; the previous text rides along as `detail`, so a client matching the body
  as prose still finds it. 400/422 stayed plain strings — this slice does not restyle routes it
  was not asked to change.
- **The lagged frame mirrors HTTP's (event, data) pair.** HTTP's notice is an event *named*
  `lagged` carrying data `{"dropped":n}`; in the line protocol the frame kind is the top-level
  key, so it becomes `{"lagged":{"dropped":n}}` — distinguishable from both event frames
  (`event`) and replies (`type:"response"`), and carrying byte-identical data. Both sides build
  that data object from `epsilon_contracts::LaggedNotice` (types-only), so "the same object" is
  structural rather than a convention.
- **`SessionState` lives in `epsilon-contracts`, with no constructor.** The brief prescribes it
  (no `cli -> server` edge) and the crate's rule is types-only, so both surfaces build the
  struct from the agent field by field. That is deliberate duplication of seven field reads,
  protected against shape drift by the shared struct — not by a second copy of it.
- **`set_thinking_level` reads the level back** instead of echoing the request: the reply is the
  state now in effect, so it cannot acknowledge a change core did not make.
- **The deferred families stay deferred**, listed in Limits with the brief's reasons — most
  importantly `set_model`/`cycle_model`/`get_available_models`, which need the model getter plus
  the TUI's hot-swap/persist sequence as one seam; half-wiring recreates the
  false-acknowledgement class M87 just closed.

## Gotchas

- **Test-caught: serde aliases and a second tag are mutually exclusive.** The first cut read
  pi's `type` with `#[serde(alias = "type")]` *and* pointed `type` at `op` before parsing — so
  a pi-shaped frame arrived at serde carrying both names and deserialization failed with
  `duplicate field op` (found by `pi_type_tag_is_read_as_op` and
  `unknown_op_reports_failure_with_the_echoed_id` on the first run). The fix keeps one
  mechanism: `normalize_tag` *moves* `type` to `op` and removes it, and the header struct has
  no alias. A frame carrying both tags now keeps its `op` and drops pi's value — which is the
  documented precedence, instead of an error.
- **Test-caught: the 202 is a reservation, not a started run.** The first version of the
  HTTP abort test posted `/abort` immediately after the prompt's 202 and received
  `409 no run is active` — the run task had not been polled yet, so core had no token
  registered (the server reserves its slot before spawning; core sets `running`/the token as
  the task's first actions). The test now waits for the run's own `AgentStart` event before
  aborting, the same contract `admit_prompt` uses on the stdio side. The window is real and
  recorded in Limits: an abort landing inside it answers per core (409/`false`) while core's
  M54 arming means the run still stops.
- **Another work item's uncommitted changes live in `crates/epsilon-contracts/src/lib.rs`.**
  The slice's shared-struct requirement could not avoid that file (a module must be declared in
  `lib.rs`), and the file carried a pre-existing working-tree delta (the `SessionHeader`/
  `ToolHooks` re-exports — not this slice's). The slice's entire footprint there is **four**
  lines — `pub mod lagged_notice;`, `pub mod session_state;`,
  `pub use lagged_notice::LaggedNotice;`, `pub use session_state::SessionState;` — inserted on
  untouched anchors; numstat is `10/3` total against HEAD (= the sibling's `6/3` plus the
  slice's `4/0`), and a reconstruction of HEAD + exactly those four lines is byte-identical to
  the candidate that the `m92b` battery measured (independently reproduced during the re-review,
  which also confirmed the sibling's two hunks and the slice's lines do not overlap). The
  commit therefore stages that reconstructed file by object hash rather than the working-tree
  file (AGENTS.md's line-level rule for shared files), so the sibling's re-exports stay
  uncommitted. Nothing else in that file changed, and no other dirty file was touched
  (`stream.rs`, `config.rs`, `registry.rs`, `agent-contract.md` are that other item's).
- `#[serde(other)]` only exists for internally (or adjacently) tagged enums — the mechanism
  depends on the `tag = "op"` shape the enum already had.
- The `Lagged` arm of the stdio event pump is not exercised end-to-end: forcing a real broadcast
  lag needs 1024+ backlogged events while a subscriber is stalled, which an in-process test
  cannot arrange (the channel is created inside `Agent::new` and only the agent publishes on
  it). `lag_notice` pins the frame's bytes and is what the pump calls; the wiring itself is
  covered only by inspection. Recorded in Limits rather than papered over.
- On an idle agent, core's `abort_current_run` also *arms* the abort flag so a run starting just
  after the request still stops (M54); both surfaces therefore inherit arming from the 409/false
  answer. That is core's contract, not a new invention here, and the tests pin the observable
  part (the reported bool).
- The RPC `data`/`ok` duplication means a success payload appears twice on the wire. Accepted for
  a local stdio protocol: each key serves one client generation, and deduplicating would require
  one of them to translate.
- `test_agent_delayed` (server tests) reuses the F13 overlap pattern — a `SlowProvider` pauses
  before each stream, so the first run is genuinely active when the second request arrives; no
  sleeps-and-hope.

## Verification

**GATE PASS** on the frozen candidate `m92b` — `audit/evidence/gate-m92b.log`
(sha256 `8f25c46d…`): all five gates exit 0 (fmt 1 s, clippy 117 s, test 670 s, demo 17 s,
real-check 14 s), **791 passed / 0 failed / 0 ignored, 55 summaries**. Per-binary delta against
the M91 baseline (`gate-m91a.log`, 772): `epsilon_cli` 26 → 38 (**+12**, the envelope,
correlation, dispatch and op tests), `epsilon_contracts` 24 → 25 (**+1**, the `xhigh` alias pin),
`epsilon_server` 9 → 15 (**+6**, the router tests, the drift test and the level-enum pin);
`epsilon_core` unchanged (the new `model_id` is exercised through the CLI and server tests).
19 total, every new test accounted for.

- Candidate: HEAD `10422ab` + exactly this slice's eight code files (the contracts `lib.rs` is a
  reconstruction of HEAD + the slice's four lines, byte-identical to what the commit stages) +
  this document; every copied file hash-verified against the working tree, and the sibling's
  four dirty files (`stream.rs`, `config.rs`, `registry.rs`, `agent-contract.md`) plus its two
  `lib.rs` hunks are provably absent from the candidate.
- Focused run on the candidate before the battery: fmt clean, clippy exit 0 on the four touched
  crates with `--all-targets -- -D warnings`, and the four lib test binaries green
  (contracts 25, core 107, server 15, cli 38 — the slice-only totals; the 108/37 seen in the
  implementer's tree include another work item's `config.rs` test).
- **Six negative-control probes** on a disposable copy — `audit/evidence/m92b-probes.log`
  (sha256 `c05e857b…`), all armed in one copy, each read against its own test:
  - P-A id echo removed (both reply builders) → `ids_echo_per_request_in_arrival_order`,
    `unknown_op_reports_failure_with_the_echoed_id` and `abort_during_an_admitted_run…` fail.
  - P-B the legacy `ok` key removed → `replies_are_single_lines_and_keep_the_legacy_keys`,
    `legacy_requests_still_work_without_ids` and the `ok`-reading assertions fail.
  - P-C a `get_state` field falsified → `get_state_values_equal_the_live_agents` fails (the field
    is compared against the live agent, not restated).
  - P-D the pre-M92 silent `Lagged` continue restored → **the suite stays green**: the pump
    *wiring* is unexercised, which is exactly the limit this document records; the frame bytes
    are pinned by `lag_notice`'s own test.
  - P-E pi's `xhigh` alias removed → both alias tests fail (`thinking_level_accepts_pis_xhigh_spelling_on_input_only`
    panics in contracts, `set_thinking_level_accepts_pis_xhigh_spelling` fails end-to-end) — B1 is
    genuinely pinned.
  - P-F a scratch route added to `route_table()` → `spec_paths_match_router` fails with the exact
    path-set difference (`…/scratch-probe` present on one side only) — B2's guard is real, not
    restated.
- **Two independent read-only reviews** — `audit/evidence/m92-review.md`: round 1 found two
  blockers (the false "pi's seven levels verbatim" claim behind `x_high`/`xhigh`; the fake
  "spec == routes" guarantee) plus nine findings; both blockers were fixed and the focused
  re-review confirmed closure with no blockers, independently reproduced the four-line contracts
  footprint, and recorded five further items (all dispositioned in the record — F1 and F3 are
  fixed in this document, F2 is closed by probe F, F4 is a scratch-script nit, F5 is accepted
  mapping-pinning).
- No reinstall is owed: the CLI's own behaviour is unchanged (the new ops are additive on the
  stdio surface, whose in-tree caller does not parse replies); the wire additions are what a
  client would consume, and the installed binary predates them only in that respect.

## Limits

- **Deferred ops, deliberately not wired** (the brief's binding list, each for its recorded
  reason): `set_model`, `cycle_model`, `get_available_models` (need the model getter plus the
  TUI's id+hot-swap+persist sequence as one seam), `set_steering_mode`/`set_follow_up_mode`
  (construction-time only in core), compaction over the wire (`compact_now` is the async
  summarizer path), session/fork/tree/export/messages ops (SESSION-01 territory), `extension_ui`
  frames, `bash`/`abort_bash`. They now fail with `unknown op: <name>` — explicit, correlated,
  and stable — rather than being silently unsupported.
- **pi interoperability is envelope-level, not payload-level.** pi's `prompt`/`steer`/`follow_up`
  frames carry `message`, ours carry `task`/`text`; pi spells the follow-up op `follow_up`, ours
  `followup`; `SessionState` keys are snake_case where pi's are camelCase, and pi's `model` is a
  `Model` object where ours is the id string. Every outbound event is additionally wrapped as
  `{"event":{…}}` (`rpc_mode.rs:367-371`) where pi emits bare event objects — the one divergence
  a pi client meets on *every* event, unlike the static items above. The tag alias and id echo are
  what this slice promised; field-level parity for the remaining commands is not implemented and
  is not claimed. `set_thinking_level`'s *ladder* is pi's seven levels
  (`pi/packages/agent/src/types.ts:300` — `epsilon_contracts::ThinkingLevel` was ported from it),
  but the wire spelling of one level diverges in both directions: pi writes `xhigh`; we accept it
  on input through a serde alias and emit the canonical `x_high` (the id session files already
  persist), so the supported loop is send `xhigh`, read `x_high`. The other six spellings match
  verbatim.
- `SessionState` carries only fields this harness can answer from live state; pi's
  `isCompacting`, queue modes, `sessionName`, `autoCompactionEnabled` and `sessionId` have no
  accessor here (the brief could not find one for the last two either) and are absent rather
  than reported as fiction.
- **`pending_message_count` counts steering only.** Ours is `Agent::steer_queue_len`
  (`epsilon-core/src/lib.rs:738-740`); core keeps follow-ups in a separate queue
  (`:365`, `follow_up` at `:743-747`) with **no length accessor at all**, while pi's field of the
  same name sums steering *and* follow-ups (`pi/packages/coding-agent/src/core/agent-session.ts:1529-1531`).
  So the number under-reports whenever follow-ups are queued — a semantic gap, recorded rather
  than papered over: closing it needs a new core getter (or one merged queue), and the key keeps
  pi's name because this payload is deliberately pi-shaped.
- The request_id echo covers `POST /prompt` and `POST /abort` 202/409 bodies; 400/422 remain
  plain strings with no echo.
- **The reservation → started window on `/abort`** (found by the server abort test): the prompt
  route replies 202 when the *reservation* is taken, a moment before the spawned run task is
  first polled and registers core's token. An `/abort` landing inside that window receives core's
  `false` (`409 no run is active`) even though a reserved run is about to start — core's M54
  arming does stop that run, so the failure is in the *reply*, not in the effect. A real client
  cannot practically hit this (the task is polled as soon as the handler yields); the in-process
  router test could, which is how it was found. Fixing it properly means deciding that a
  reservation counts as "a run" for abort's status, which is a behaviour change to M87's reply
  contract and was deliberately left out of this slice.
- **The two abort tests are timing-assuming, not synchronized.** Both wait for the run's own
  `AgentStart` event and then issue the abort (`rpc_mode.rs:700-735`;
  `epsilon-server/src/lib.rs:681-736, 740-760`), and stay correct only while the next request
  lands inside the run's ~400 ms `SlowProvider` pause (`test_agent_delayed`, `:485-505`). That
  margin is wide but finite: on an arbitrarily loaded machine the run could finish first and the
  abort would honestly answer "no run active". Recorded as a residual rather than hidden behind a
  longer sleep — fixing it properly needs a provider the test can hold open, which the faux
  harness does not offer.
- **What the OpenAPI drift test guarantees, and what it does not.** `spec_paths_match_router`
  compares the document's path *set* against the router's single `route_table()`
  (`epsilon-server/src/lib.rs:173-189`, the only place routes are registered) — a scratch-route
  negative control during the fix round proved it fails when the two diverge. It does **not**
  compare methods, handlers, parameters, bodies, or schemas below the path level (the
  `SessionState.thinking_level` enum is pinned separately, against the serialized
  `ThinkingLevel::ALL`), and everything else in the document remains hand-maintained. Before
  this, "spec == routes" was membership in a hardcoded 9-name list inside the test — the router
  was never read, so a route added without a spec entry failed nothing. (The `Lagged` arm of the stdio pump is also not exercised end-to-end — see below.)
- **The stdio loop itself** (`run`) has no end-to-end test: it owns the process's stdin/stdout.
  `dispatch_line` (every decision worth asserting) and `lag_notice` are covered directly; the
  pump's `Lagged` wiring is inspection-only, as recorded above.
- **The level enum's one residual:** both the OpenAPI enum and its pin derive from the
  hand-maintained `ThinkingLevel::ALL` list, so spellings cannot drift (both go through the same
  serde path `/state` uses) but a variant added to the enum *without* being added to `ALL` would
  be silently omitted from the document and from the pin together. `ThinkingLevel::as_str()`'s
  exhaustive match is the compiler-side mitigation.
- Whether any out-of-tree client depended on the exact previous `{"ok":…}` bytes is unknown (the
  brief could not determine it; the only in-tree caller is `main.rs:1433`, which does not parse
  replies). The wire kept those keys and shapes precisely because that could not be established.
- `crates/epsilon-tui/` is untouched by this slice, per its coordination boundary (the tree
  carries another work item's uncommitted changes there).
- Runtime behaviour of the two surfaces was exercised only through focused offline tests in the
  implementer's round; the frozen-candidate battery and the focused re-review of the fix diff are
  still owed (see Verification).
