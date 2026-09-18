# RPC-02 — protocol breadth (correlated replies, abort/state, dropped events)

## State
- Brief received 2026-09-18 (deleg_6648c3e8, read-only, 18 api calls). Implementation **not yet
  dispatched** — waiting for the M91 child's build to finish (one build owner on this machine).
- Plan binding: audit/09-revalidation.md row 4 (RPC-01/RPC-02 + SESSION-01); the finding's own
  acceptance list is audit/evidence/revalidation-core-review.md:73-81. M87 already closed the
  admission reply contract on both surfaces — do not re-propose it.

## The headline fidelity fact
pi's RPC client resolves only frames tagged `type:'response'` with a matching `id` and hands
everything else to event listeners (rpc-client.ts:508-522,540-590). Epsilon's RPC has **no id
field anywhere** and tags frames `op` (rpc_mode.rs:17-35, 204-213; its own comment says replies
correlate by arrival order). So current frames are unusable by a pi client unchanged.
pi's command set is 30+ id-bearing commands dispatched in rpc-mode.ts:394-680
(rpc-types.ts:20-73); responses are `{id?, type:'response', command, success, data|error}`.

pi has **no HTTP server**: the only one is the experimental `@earendil-works/pi-server`
(CBOR over a unix socket). So serve mode's target is the repo's own convention — ADR-011 fixes
serve (axum + SSE + OpenAPI) with a hand-maintained OpenAPI document and a route-drift test
(auto-amended M12, epsilon-decisions.md:406-436).

## Self-divergences between our two surfaces
- Same `AgentEvent`, two wire shapes: RPC wraps `{"event":...}` (rpc_mode.rs:148-151); HTTP SSE
  sends the bare event JSON (server/lib.rs:326-328).
- Dropped events: RPC silently continues on `Lagged` (rpc_mode.rs:156); HTTP emits an SSE
  `lagged` frame with the count (server/lib.rs:330-335).
- Error vocabularies: HTTP has statuses (400/401/403/404/409/422); RPC has one string channel.
- Neither surface exposes abort or state, though core backs both
  (`abort_current_run` core/lib.rs:1093, `is_running` :693, `history` :874, `session_path` :827,
  `steer_queue_len` :732, `thinking_level` :555, `estimated_context_tokens` :1655).
- Two getters are missing for a faithful get_state/set_model: no public `model_id()` (private
  field :419, setter :510) and no steering/follow-up mode setter (construction-time only :459-460).
- Testability asymmetry: RPC dispatch lives inside `run()`, which owns stdin/stdout
  (rpc_mode.rs:135-200; sole caller main.rs:1433) so only `admit_prompt` + two helpers are
  tested; HTTP is exercised through the real router with tower oneshot (server/lib.rs:342-539).
- Ledger drift: F13's "still open" clause lists HTTP/serve admission although M87 closed it
  (audit/02-defect-ledger.md, F13 section) — the doc needs the sync.

## Slice plan (smallest faithful)
1. `rpc_mode.rs`: optional `id` on incoming frames + accept pi's `type` tag as an alias for
   `op`; replies `{type:'response',command,id,success,data|error}` **keeping the existing
   ok/error keys** so current clients keep working (backward-compat risk rated MED in the
   finding); extract `async fn dispatch_line(&str, &Arc<Agent>, &Arc<ChannelApprovalUi>) -> String`
   out of `run()` so the loop is testable in-process; add ops **abort, get_state,
   set_thinking_level**; replace the silent `Lagged` continue with a lagged frame matching
   HTTP's.
2. `epsilon-core/src/lib.rs`: one read-only `pub fn model_id(&self) -> String`.
3. `epsilon-server/src/lib.rs`: `POST /abort`, `GET /state` returning the same payload as RPC
   (one shared struct — put it in `epsilon-contracts` so no cli→server edge is added), optional
   `request_id` echoed in the 202/409 bodies; HTTP keeps statuses rather than importing RPC's
   string vocabulary.
4. `epsilon-server/src/openapi.rs`: register both paths (the drift test at :100-106 and
   ADR-011's spec==routes rule make this mandatory).
5. Docs (mine): ADR-011 amendment (envelope, id echo, serve-has-no-pi-counterpart) and the F13
   ledger sync.
6. Tests, offline: RPC unit tests over `dispatch_line` (existing Fake/Slow provider patterns):
   two requests with distinct ids echo their own id in arrival order; unknown op → failure
   envelope with the echoed id and a stable error string; abort on an idle agent returns core's
   false rather than a fabricated success; abort during an admitted run succeeds; get_state
   values equal the live Agent's; the lagged frame's shape. HTTP via the router: POST /abort
   during a SlowProvider run ends it; POST /abort with no run says so; GET /state shape; 202/409
   bodies echo request_id; openapi drift test extended.

## Deferred with reasons (do not half-wire)
set_model/cycle_model/get_available_models (needs the model getter plus the TUI's
id+hot-swap+persist sequence as one seam; half-wiring recreates F13's false-acknowledgement
class), queue-mode ops (no core setter), compaction over the wire (`compact_now` is the async
summarizer path), session/fork/tree/export/messages ops (SESSION-01 territory), extension_ui
frames, bash/abort_bash.

## Implementation status (2026-09-18, child dispatched)
- Slice implemented in the working tree, uncommitted. Files: `crates/epsilon-cli/src/rpc_mode.rs`
  (envelope + id echo + `type` alias + `dispatch_line` + ops abort/get_state/set_thinking_level +
  `lag_notice` + 13 unit tests), `crates/epsilon-contracts/src/{session_state.rs,lib.rs}`
  (`SessionState`, types-only), `crates/epsilon-core/src/lib.rs` (`model_id()`),
  `crates/epsilon-server/src/{lib.rs,openapi.rs}` (`POST /abort`, `GET /state`, `request_id` echo,
  4 router tests + drift list), `docs/milestones/M92-rpc-protocol-breadth.md` (verification
  PENDING), `docs/README.md` (M92 row).
- Focused round green on the working tree: `cargo fmt --all -- --check` clean; clippy on the four
  touched crates `--all-targets -- -D warnings` exit 0; tests — contracts 24, core lib 108,
  server lib 13, cli lib 36 (rpc_mode 13) — all 0 failed; re-runs stable. Raw logs:
  `%LOCALAPPDATA%/Temp/rpc02-logs/`. Frozen-candidate battery still owed.
- `epsilon-contracts/src/lib.rs` carried another work item's delta; the edit there is exactly two
  additive lines (numstat 6/3 → 8/3), their hunks re-verified byte-identical.
- Two test-caught fixes recorded in M92's Gotchas: the `type`-alias/`normalize_tag` duplicate-field
  bug, and the reservation→started window for `/abort` (now a Limits entry, M87 reply contract left
  untouched). `epsilon-tui/**` untouched.

## Could not determine (from the brief)

Whether any out-of-tree client depends on the current `{"ok":...}` bytes (only main.rs:1433
in-tree); whether pi's stdout backpressure can drop/stall (helper unread); whether the full pi
get_state field set is representable (no session-name or auto-compaction toggle found);
runtime behaviour of either surface (no builds allowed for the brief); whether row 4's
fork/branch acceptance is RPC-02's alone or joint with SESSION-01.

## Round state (parent, 2026-09-18)

- Implementation (deleg_4e360c23) and the independent review (deleg_598a57b5) are done.
  Review verdicts: envelope/correlation/dispatch-extraction/lagged/HTTP-routes PASS;
  **two blockers**: (B1) `ThinkingLevel::XHigh` serializes as `x_high` (`snake_case` rename,
  persisted format) while pi's vocabulary is `xhigh`, and the OpenAPI enum declared `xhigh` —
  a value the server can never emit; (B2) `spec paths == registered routes` is NOT enforced —
  the drift test compares the spec against a hardcoded 9-entry list and never reads `router()`,
  so a route added without a spec entry fails nothing (pre-existing since M12, but M92's docs
  restated it as enforced).
- Fix round dispatched as deleg_8b801c43 (serde `alias = "xhigh"` on XHigh + OpenAPI enum
  corrected + a real single-source route table compared against the spec + a shared
  `LaggedNotice` type + three missing tests + two strengthened tests + the Limits corrections).
- The `m92a` battery was **stopped mid-flight on purpose** — the bytes it was measuring are
  about to change and a log for non-shipped bytes proves nothing. Machine verified clean
  (tasklist: zero cargo/rustc). Lesson recorded in next-hardening.md: PowerShell sweeps must
  not match their own command line (split the pattern), and MSYS `sha256sum` escapes
  backslash paths with a leading `\` (false MISMATCH).
- Owed after the fixes land: re-assemble the candidate (contracts/lib.rs built as HEAD + only
  M92's two lines — the file carries a sibling's hunks, so the commit needs the AGENTS.md
  index surgery), focused verify, probes, battery `m92b`, a FOCUSED re-review of the fix diff,
  then: history row, Verification section, commit, cleanup, retire this note.

## Fix round result (deleg_8b801c43, 2026-09-18) — landed uncommitted

- B1: `#[serde(alias = "xhigh")]` on `ThinkingLevel::XHigh` (serialization still `x_high`; both
  spellings + persistence pin are tested in `message.rs`, and an RPC `set_thinking_level` with
  pi's `xhigh` is exercised end-to-end). OpenAPI `SessionState.thinking_level` enum now derives
  from the serialized `ThinkingLevel::ALL` (response direction: emitted spellings only) with a
  drift pin; no request-side schema carries a level today. Milestone doc: "seven levels verbatim"
  claim corrected; outbound `x_high` divergence recorded in Limits.
- B2: `epsilon-server/src/lib.rs` now has one `fn route_table()` that `router()` folds (and the
  `/openapi.json` route joined both the table and the spec); `spec_paths_match_router` compares
  the spec's path *set* against that table. Negative control: a scratch route added to the table
  alone → `spec_paths_match_router` FAILED with the set diff, then removed (green).
  `openapi.rs` module comment and the M92 doc state the exact guarantee (path sets only).
- Finding 1: `epsilon_contracts::LaggedNotice { dropped: u64 }` (types-only, new module) built
  by both surfaces; wire bytes pinned on both sides.
- Findings 2–5: both-tags test, `/abort` 409 with `request_id`, unknown op with a realistic
  payload, `set_thinking_level` starts from a different level, get_state tautology removed,
  Limits (steering-only count, `{"event":…}` wrapper, abort timing residual), `SessionState`
  header reworded, approve-id comment. `epsilon-contracts/src/lib.rs` numstat 10/3 (sibling 6/3
  + M92's 2 + this round's 2). No commit.
- Focused battery green on the working tree (target dir `%LOCALAPPDATA%/Temp/epsilon-rpc02-target`):
  `cargo fmt --all -- --check` exit 0; clippy on the four crates `--all-targets -- -D warnings`
  exit 0; lib tests cli 38 / contracts 25 / core 108 / server 15, 0 failed. Still owed: probes,
  `m92b`, focused re-review, Verification section.

## File inventory for the M92 commit (verified 2026-09-18)

SLICE-OWNED (stage):
- `crates/epsilon-cli/src/rpc_mode.rs`, `crates/epsilon-server/src/lib.rs`,
  `crates/epsilon-server/src/openapi.rs`, `crates/epsilon-core/src/lib.rs` (model_id only),
  `crates/epsilon-contracts/src/message.rs` (the `xhigh` alias + doc + test — verify the full
  diff is only that before staging),
- `crates/epsilon-contracts/src/lib.rs` — SLICE FOOTPRINT IS 4 LINES ONLY
  (`pub mod lagged_notice;`, `pub mod session_state;`, `pub use lagged_notice::LaggedNotice;`,
  `pub use session_state::SessionState;`) → AGENTS.md index surgery: `git hash-object -w` a
  file built as HEAD + exactly those 4 lines (saved at
  `$LOCALAPPDATA/Temp/m92-contracts-commit.rs`) then `git update-index --cacheinfo`, and prove
  `git diff --cached HEAD -- <file>` shows 4 insertions.
- new: `crates/epsilon-contracts/src/{session_state,lagged_notice}.rs`,
  `docs/milestones/M92-rpc-protocol-breadth.md`;
  docs: `docs/README.md`, `docs/design/epsilon-decisions.md` (ADR-011 amendment), `audit/02-defect-ledger.md` (F13 sync), `docs/history.md` (row, after the battery).

NOT OURS (do not stage — another work item's): `crates/epsilon-contracts/src/stream.rs`,
`crates/epsilon-core/src/config.rs`, `crates/epsilon-core/src/registry.rs`,
`crates/epsilon-core/assets/agent-contract.md`, and the session_entry/tool re-export expansions
in contracts/lib.rs.

Count reconciliation (why the child's numbers differ from a slice-only tree): with the slice
alone, `-p epsilon-contracts --lib` = 25 (24 was the run with message.rs missing from my
candidate), `-p epsilon-core --lib` = 107 (the child saw 108 because the sibling's config.rs
adds a test), `-p epsilon-server --lib` = 15, `-p epsilon-cli --lib` = 38. My first m92b run
omitted message.rs and failed 2 cli tests — its own probe assert caught the omission.
