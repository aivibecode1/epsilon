# ponytail dead-code sweep — Epsilon, 2026-10-04

Re-derived from source by an independent read-only pass, because **the 38-finding audit's list
was never committed** — `docs/STATUS.md` claimed "full list retained with this section's
summary below", and the numbered findings existed nowhere in the tree. Nobody can dispose of
"finding #14" by number when the list is a paragraph. This file is that list, so the next pass
starts from evidence.

**Nothing here was deleted.** Every row carries a disposition; the ones marked *delete* are
authorized and ready, and they are the first thing the next round should do.

## Method (so a reader can disagree with it)

Every `fn`/`struct`/`enum`/`const`/`static`/`type`/`trait` definition outside `#[cfg(test)]`
across all 228 `.rs` files in 14 crates was indexed — 3,078 of them — and every mention of each
name classified production or test by walking back to the nearest `#[cfg(...)]` or column-0
`}`. Every verdict below was then re-checked against a `grep` hit list read by hand.

**Not reached**, so do not read this as clean: `macro_rules!`, `mod` declarations, enum variants,
trait-declaration methods, and the `include_str!` description assets. `Span::rich`
(`crates/epsilon-desktop/src/markup.rs:76`) is classifier-only with no independent grep —
verify before acting. No build was run.

Lines are the measured span from the definition back through its doc comment and forward through
the matching brace. They **exclude the test bodies that would also have to go**, so the real
saving is larger than the total.

## Class A — zero callers anywhere. 23 items, 175 lines. Delete.

| Item | file:line | Lines |
|---|---|---|
| `read_line_from_stdin` | `epsilon-llm/src/oauth_login.rs:1114` | 8 |
| `GOAL_DEFAULT_ROUNDS` | `epsilon-tools/src/goal.rs:478` | 2 |
| `LiveOutput::current_tail` — doc claims "used by tests and by a final flush"; there is neither | `epsilon-tools/src/util.rs:737` | 4 |
| `TranscriptLine::code_line` | `epsilon-tui/src/state.rs:500` | 4 |
| `snapshot_rows_in` — doc claims "the shape a test needs"; no test uses it | `epsilon-tui/src/render_cache.rs:586` | 14 |
| `framing::frame_bytes` — doc claims "for tests and fixtures"; even this file's tests build frames by hand | `epsilon-lsp/src/framing.rs:168` | 6 |
| `read_text_capped` — the sibling's doc describes *this* as what grep uses; grep calls `read_text_capped_ex` | `epsilon-tools/src/util.rs:328` | 4 |
| `Agent::skill_names` | `epsilon-core/src/lib.rs:861` | 4 |
| `Theme::border_active` — byte-identical body to `border_focus:1055` | `epsilon-tui/src/ui/theme.rs:1074` | 4 |
| `Theme::muted` — identical body to `system:1070` | `epsilon-tui/src/ui/theme.rs:1079` | 4 |
| `Theme::selected_muted` | `epsilon-tui/src/ui/theme.rs:1084` | 11 |
| `accept_project_config_hash` — superseded by `epsilon-core/src/config.rs:224`, which has production callers | `epsilon-runtime/src/lib.rs:351` | 10 |
| `BashTool::with_policy` | `epsilon-tools/src/shell.rs:84` | 5 |
| `register_builtins_with_skills` — runtime registers skills directly at `runtime/src/lib.rs:4749` | `epsilon-tools/src/lib.rs:91` | 17 |
| `builtin_registry_with` — doc says "This is what the product path uses"; it is not | `epsilon-tools/src/lib.rs:206` | 10 |
| `SessionStore::open_with_progress` — `open_in_with_progress:485` is the live twin | `epsilon-session/src/store.rs:493` | 22 |
| `clear_write_fault_for_tests` | `epsilon-session/src/store.rs:697` | 5 |
| `config::skill_is_disabled` — the list is read through `load_disabled_skills` | `epsilon-core/src/config.rs:357` | 4 |
| `Agent::restored_len` | `epsilon-core/src/lib.rs:886` | 4 |
| `Usage::without_cache` | `epsilon-contracts/src/message.rs:188` | 10 |
| `GoalSnapshot::rounds_remaining` — `exhausted` is the live sibling | `epsilon-contracts/src/goal.rs:213` | 4 |
| `OAUTH_SYSTEM_IDENTITY` | `epsilon-llm/src/providers/anthropic.rs:381` | 4 |

## Class B — no production caller, only test callers. 50 items, 652 lines. Still dead by this
repository's own rule: *a definition plus a test is not a feature.*

**B1 — delete, with the test sites.**

| Item | file:line | Lines |
|---|---|---|
| `Hints::keys_text` — doc claim still false today | `epsilon-tui/src/ui/footer.rs:290` | 9 |
| `HostHandle::probe_event_forward_latency` — **14 lines, not the ~40 the summary claimed** | `epsilon-ext/src/host.rs:316` | 14 |
| `FallbackChain::usable_count` | `epsilon-llm/src/fallback.rs:68` | 4 |
| `FileWatcher::is_watching` | `epsilon-desktop/src/diff.rs:156` | 4 |
| `is_custom_model_id` | `epsilon-llm/src/custom.rs:106` | 6 |
| `EntryKind::is_unknown` | `epsilon-contracts/src/session_entry.rs:388` | 4 |
| `Gate::session_allows` | `epsilon-safety/src/gate.rs:72` | 4 |
| `KeybindingsManager::from_map` | `epsilon-tui/src/keybindings.rs:341` | 7 |
| `key_text_round_trips` | `epsilon-tui/src/keybindings.rs:384` | 14 |
| `Skill::load_body` | `epsilon-core/src/skills.rs:28` | 6 |
| `preview_target` | `epsilon-core/src/references.rs:384` | 6 |
| `followup_queue_len` | `epsilon-core/src/lib.rs:2514` | 4 |
| `CheckpointStore::with_max_bytes` | `epsilon-tools/src/checkpoint.rs:151` | 7 |
| `SessionLease::acquire_now` | `epsilon-session/src/lease.rs:217` | 7 |
| `Transcript::note_lag` | `epsilon-desktop/src/session.rs:615` | 10 |
| `MAX_BREAKPOINTS` | `epsilon-llm/src/cache_policy.rs:35` | 7 |
| `SKIP_DIRS` — and delete `tools/tests/builtin_tools.rs:2186`, a test whose stated purpose is to keep this constant alive | `epsilon-tools/src/util.rs:32` | 2 |
| `commands::refused_heads` — a `pub` fn whose own doc says it exists "for the test that the refusal table and the parser have not drifted apart" | `epsilon-desktop/src/commands.rs:321` | 22 |
| `has_trust_requiring_resources` | `epsilon-runtime/src/lib.rs:674` | 5 |
| `discover_agents` — production uses `discover_agents_with_notices` | `epsilon-core/src/agents.rs:72` | 19 |
| `describe_task` | `epsilon-core/src/agents.rs:300` | 14 |
| `skill_roots` — production uses `skill_roots_trusted:4726` | `epsilon-runtime/src/lib.rs:702` | 9 |
| `extension_roots` — production uses `extension_roots_trusted:4762` | `epsilon-runtime/src/lib.rs:777` | 7 |
| `latest_session_file` — production uses `latest_session_file_in:1678` | `epsilon-runtime/src/lib.rs:1384` | 10 |
| `ExtManifest::wants_event` | `epsilon-ext/src/manifest.rs:40` | 4 |
| `UiState::push_external_text` | `epsilon-tui/src/state.rs:980` | 10 |
| `Glyphs::SINGLE_GLYPH_FIELDS` | `epsilon-tui/src/ui/theme.rs:484` | 39 |
| `snapshot_rows` | `epsilon-tui/src/render_cache.rs:549` | 17 |
| `Transcript::open_approvals` | `epsilon-desktop/src/session.rs:387` | 20 |

**B2 — file as a defect, do not delete.** These have docs asserting a caller that does not
exist. Deleting them silently would delete the *record* of a missing capability, which is the
one thing worth keeping.

| Item | file:line | Lines | Why it is filed rather than deleted |
|---|---|---|---|
| `bedrock_event_payloads` | `epsilon-llm/src/providers/bedrock.rs:454` | 39 | Two docs say the transport layer unwraps the AWS binary event-stream envelope through it. **Grep finds one definition and two uses, both in this file's tests.** Nothing in production unwraps it. Whether Bedrock is broken at runtime was **not** measured — no live provider, no build. The unreachable named path is the finding. |
| `GoogleVertexWire::bearer_token` | `epsilon-llm/src/providers/vertex.rs:53` | 11 | `headers():147` says "resolution happens above this layer (see `bearer_token`)". Zero production callers. |
| `adc_jwt_input` | `epsilon-llm/src/providers/vertex.rs:75` | 28 | Same shape as the row above, on the function that produced it. |
| `Agent::search_history` | `epsilon-core/src/lib.rs:1543` | 15 | 15 lines of doc promising pi `search/scanning.ts` parity. The TUI `/search` searches the transcript. **A missing feature, not dead code.** |
| `ServeState::active_runs` | `epsilon-server/src/lib.rs:80` | 4 | Nine test sites for a counter no route reports. Wire a `/health`, or delete. |
| `Agent::set_summarizer` | `epsilon-core/src/lib.rs:1029` | 6 | Nothing in production installs a summarizer, which is worth a look on its own. |

**B3 — keep and justify. Test capability shipped in release libraries.**

| Item | file:line | Lines | Justification |
|---|---|---|---|
| `test_support` (whole module) | `epsilon-core/src/test_support.rs` | 106 | Test-only, and shipped: `pub mod test_support;` at `core/src/lib.rs:28` carries no `cfg`. Its doc argues it must be `pub` so `epsilon-tools` shares it — true — and never argues why it is in the **production** library. `mklink`'s comment at `:93` shows the authors knew it ships. **Moving it behind `#[cfg(test)]` would need it shared across crates, so the honest fix is a workspace dev-dependency, not a deletion.** |
| `LineReassembler` + two usage helpers | `epsilon-llm/src/providers/mod.rs:1544` | 65 | `pub mod test_support`, reached only from `llm/tests/`. |
| `from_script_json`, `recorded_requests`, `recorded_requests_handle` | `epsilon-llm/src/faux.rs:94` | 20 | The faux provider is the offline test substrate the whole suite runs on. |
| `load_or_build_test_manifest` | `epsilon-ext/src/manifest.rs:72` | 12 | `#[doc(hidden)]`; extension host tests. |
| `HttpEventStream::empty_for_tests` | `epsilon-llm/src/providers/mod.rs:246` | 4 | Same. |

**B4 — keep and justify, individually.** Desktop test seams with many call sites:
`IdeApp::without_agent` (`app.rs:1449`, 54 lines, 14 sites), `set_write_fault_for_tests`
(`epsilon-session/src/store.rs:646`, 50 lines — the fault-injection seam the F07 durability
tests depend on), `FrameSummary::drew` (42 sites) and `::hotspot` (25 sites) — frame-assertion
helpers that are the whole point of the frame tests — `seed_transcript` (27 sites), `set_draft`
(6), `set_focus` (4), `run_palette_command` (5), `sidebar_view` (1), `store_handle_for_tests`
(7, its name and doc admit it), `start_goal_round_for_test` (1, cited at `app.rs:1388` as the
reason two bugs were invisible), `write_fault_pending_for_tests` (1), `mark_dead` (1, doc says
"Test seam"), `test_support::dir_link` / `::file_link` (8).

## False alarms — checked and cleared. Do not "fix" these.

| Item | Why it is reachable |
|---|---|
| `FixtureServer::get_info` | impl of rmcp's `ServerHandler`, reached through the trait object |
| `EchoTool::new`, `FailTool::new` | called from the **production** `--demo` path, `epsilon-cli/src/main.rs:1814` |
| `bwrap_argv` | production caller at `sandbox.rs:440`; `#[cfg_attr(windows, allow(dead_code))]` is a platform allow |
| `ChildIo.child` | `#[allow(dead_code)]` on a field held for `kill_on_drop`, justified in place |
| `SessionStore::branch_with_summary` | **still dead** — the repository's own named example, 51 lines, the largest single item, 8 test sites. Delete, or wire `/fork` to it. Not in the false-alarm table: it is in B1's cousins and belongs in the next round's decision. |

An identifier-frequency scan is fooled by same-named locals in other crates
(`core/src/walk.rs:303` has its own private `file_link`; `bedrock.rs:652` has a local
`frame_bytes`), which is why every verdict came from a hand-read `grep` list.

**The private-item class is empty**, as expected — `rustc`'s `dead_code` lint catches it. Every
surviving item is `pub`, and `clippy -D warnings` is structurally blind to those.

## One corruption found on the way

`crates/epsilon-llm/src/providers/vertex.rs:80` read `/// Syncлой surface:` — three Cyrillic
characters (U+043B U+043E U+0439) inside an English sentence. **Fixed.** The sweep counted 78
Cyrillic/Greek codepoints across all 228 files; the Greek ones in `tui/src/editor_ops.rs` and
`tui/src/latex.rs` are legitimate maths. `scripts/check-control-bytes.py` does not catch this
class — it looks for C0 control bytes, and a homoglyph is not one.

## Totals

| | items | lines |
|---|---|---|
| Class A — zero callers | 23 | 175 |
| Class B1 — test-only callers | 30 | 400 |
| Class B2 — file as a defect | 6 | 103 |
| Class B3/B4 — keep and justify | ~21 | ~324 |
| **Verified dead and awaiting deletion** | **59** | **575** |
| False alarms cleared | 5 | — |

**The audit is neither clean nor fully disposed, and now that is visible rather than implied.**
The next round's first job is Class A and B1: 53 items, 575 lines, every row already
authorized above. Each batch needs its own battery, because a mass deletion that breaks a call
site the sweep missed is exactly the failure this file exists to prevent.