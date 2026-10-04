# Plan 024 §7 — Class B re-derived from the tree, 2026-10-04

Read-only pass. **Nothing was deleted.** No `cargo` was run. Every line number below was read
from the tree today; where it differs from the ledger, both are given.

Source ledger: [`ponytail-deadcode-2026-10-04.md`](ponytail-deadcode-2026-10-04.md).
Plan item: [`../../docs/plans/024-finishing-the-objective.md`](../../docs/plans/024-finishing-the-objective.md) §7.

## Method

Every identifier below was matched with word boundaries (`\b<name>\b`, so `skill_roots` does not
match `skill_roots_trusted`) across `crates/**/*.rs`, `scripts/**/*.{py,ps1,sh}`, `agents/`, then
separately across `docs/` and `references/` (26,282 files). Each hit was then classified by
walking back to the enclosing `#[cfg(test)] mod tests { … }` region of its file — the region
detector reads the `mod` line's indentation and runs to the matching column-n `}`, so a hit at
`footer.rs:467` is *inside* `mod tests` (430–1023) even though the file is production source.
Comments and doc-comments were read and excluded: a name in a `//` line is not a caller.

---

## 1. The recount — the header is wrong again

| Group | Ledger header | Rows actually in the table | Sum of the `Lines` column |
|---|---|---|---|
| **Class B1** | "30 items, 400 lines" (Totals table) | **29** | **291** |
| **Class B2** | "6 items, 103 lines" | 6 | 103 ✅ |
| **Class B3** | ~21 items, ~324 lines (with B4) | 5 rows | 207 (+ B4) |
| **Class B4** | (in the same ~21 / ~324) | **15 items, in prose, not a table** | not stated for 13 of 15 |
| **Class B total** | **"50 items, 652 lines"** | **40 tabular rows + 15 prose items = 55 items** | **601 for B1+B2+B3; ~715 if B4's two stated line counts are included** |

**Nothing reproduces 50 items or 652 lines.** 40 tabular rows is not 50; 55 items counting the
B4 prose list is not 50. B1's Totals row says 30 items / 400 lines against a 29-row table summing
to 291 — a 1-item, 109-line overstatement on its own. B2 is the only group whose header matches
its table.

This is the same failure the plan already records for Class A: the header says **23 items, 175
lines** (`ponytail-deadcode-2026-10-04.md:28`, repeated at `:154`) against **22 rows summing to
160** — I added them by hand: 8+2+4+4+14+6+4+4+4+4+11+10+5+17+10+22+5+4+4+10+4+4 = 160.
**The error is in the same direction both times and it is always in the Totals table, not in the
rows.** The rows are trustworthy; the arithmetic over them is not. Do not spend a round reconciling
the header — spend it re-deriving from the rows, which is what this file is.

**Line drift since the sweep.** 20 of 29 B1 rows have moved. The largest: `followup_queue_len`
`lib.rs:2514 → 2639`, `SINGLE_GLYPH_FIELDS` `theme.rs:484 → 516`, `MAX_BREAKPOINTS`
`cache_policy.rs:35 → 41`, `acquire_now` `lease.rs:217 → 238`, `search_history`
`core/lib.rs:1543 → 1673`. One moved the *wrong* way: `has_trust_requiring_resources`
`runtime/lib.rs:674 → 665`.

**Two names in the ledger are wrong, not just stale.**
- Row 4 is `FileWatcher::is_watching`. The type is **`ChangeWatcher`** (`diff.rs:157`).
- Row 7 is `Gate::session_allows`. The type is **`RulesetGate`** (`gate.rs:73`).

Neither matters for a grep, both matter to anyone reading the table.

---

## 2. B1 — the 29 rows, re-derived

| # | Item | file:line today | Callers (all of them) | Class | Verdict |
|---|---|---|---|---|---|
| 1 | `Hints::keys_text` | `epsilon-tui/src/ui/footer.rs:292` | `footer.rs:467,552,554,560` | test-only | **(a)** |
| 2 | `HostHandle::probe_event_forward_latency` | `epsilon-ext/src/host.rs:322` | `ext/tests/host_protocol.rs:864` | test-only | **(a)** |
| 3 | `FallbackChain::usable_count` | `epsilon-llm/src/fallback.rs:69` | `fallback.rs:209,211` | test-only | **(a)** |
| 4 | `ChangeWatcher::is_watching` | `epsilon-desktop/src/diff.rs:157` | `diff.rs:438,455,482` | test-only | **(a)** |
| 5 | `is_custom_model_id` | `epsilon-llm/src/custom.rs:109` | `custom.rs:173,175,178` | test-only | **(a)** |
| 6 | `EntryKind::is_unknown` | `epsilon-contracts/src/session_entry.rs:389` | `session_entry.rs:551`; `session/tests/session_store.rs:2089,2192` | test-only | **(a)** |
| 7 | `RulesetGate::session_allows` | `epsilon-safety/src/gate.rs:73` | `gate.rs:225,273`; `safety/tests/additive_authorization.rs:224` | test-only | **(a)** |
| 8 | `KeybindingsManager::from_map` | `epsilon-tui/src/keybindings.rs:342` | 14 sites, all test | test-only | **(c)** |
| 9 | `key_text_round_trips` | `epsilon-tui/src/keybindings.rs:390` | `footer.rs:496,961` | test-only | **(b) keep** |
| 10 | `Skill::load_body` | `epsilon-core/src/skills.rs:29` | `skills.rs:385` | test-only | **(a)** + cascade |
| 11 | `preview_target` | `epsilon-core/src/references.rs:385` | `references.rs:666,669` | test-only | **(c)** |
| 12 | `followup_queue_len` | `epsilon-core/src/lib.rs:2639` | `cli/src/rpc_mode.rs:2814,2842` | test-only | **(c)** |
| 13 | `CheckpointStore::with_max_bytes` | `epsilon-tools/src/checkpoint.rs:152` | `checkpoint.rs:982` | test-only | **(a)** |
| 14 | `SessionLease::acquire_now` | `epsilon-session/src/lease.rs:238` | `lease.rs:676,855`; `session/tests/session_store.rs:374` (+ comment `:369`) | test-only | **(c)** |
| 15 | `Transcript::note_lag` | `epsilon-desktop/src/session.rs:616` | `session.rs:1382` | test-only | **(c) — live defect** |
| 16 | `MAX_BREAKPOINTS` | `epsilon-llm/src/cache_policy.rs:41` | `cache_policy.rs:100` | test-only | **(c)** |
| 17 | `SKIP_DIRS` | `epsilon-tools/src/util.rs:33` | `tools/tests/builtin_tools.rs:2186` | test-only | **(a)** |
| 18 | `commands::refused_heads` | `epsilon-desktop/src/commands.rs:334` | `commands.rs:385,465` (both tests); `desktop/tests/window.rs:1668` | test-only | **(b) keep** |
| 19 | `has_trust_requiring_resources` | `epsilon-runtime/src/lib.rs:665` | `lib.rs:1858` | test-only | **(a)** |
| 20 | `discover_agents` | `epsilon-core/src/agents.rs:84` | `agents.rs:346,381,389,417,497,541` | test-only | **(a)** |
| 21 | `describe_task` | `epsilon-core/src/agents.rs:304` | `agents.rs:405,418` | test-only | **(b) keep** |
| 22 | `skill_roots` | `epsilon-runtime/src/lib.rs:697` | `lib.rs:1987` | test-only | **(a)** |
| 23 | `extension_roots` | `epsilon-runtime/src/lib.rs:770` | `lib.rs:1988` | test-only | **(a)** |
| 24 | `latest_session_file` | `epsilon-runtime/src/lib.rs:1380` | `lib.rs:1778,1796` | test-only | **(a)** |
| 25 | `ExtManifest::wants_event` | `epsilon-ext/src/manifest.rs:41` | `ext/tests/event_forwarding.rs:105,106,109,110` | test-only | **(c)** |
| 26 | `UiState::push_external_text` | `epsilon-tui/src/state.rs:982` | `tui/src/app/tests.rs:5316,5343,5462` | test-only | **(b) keep** |
| 27 | `Glyphs::SINGLE_GLYPH_FIELDS` | `epsilon-tui/src/ui/theme.rs:516` | `theme.rs:1195` | test-only | **(b) keep** |
| 28 | `snapshot_rows` | `epsilon-tui/src/render_cache.rs:553` | `render_cache.rs:883,943` | test-only | **(b) keep** |
| 29 | `Transcript::open_approvals` | `epsilon-desktop/src/session.rs:392` | `session.rs:1342,1349` | test-only | **(c)** |

**15 (a) · 6 (b) · 8 (c).**

---

## 3. The five traps, checked on every row

**Same-named method on a different type.** `wants_event` (row 25) is the live one. `is_watching`
(row 4) sits on `ChangeWatcher`; `session_allows` (row 7) on `RulesetGate`. `usable_count`,
`from_map`, `drew`, `hotspot` all appear once each in the tree. No collisions found.

**Same-named *test* function.** `key_text_round_trips` is a **production method whose name reads
like a test** (`keybindings.rs:390`) — the single most dangerous row in the table, because a
sweep that greps for test names would take the method for the test. It is `pub fn` on
`KeybindingsManager`, and the two callers at `footer.rs:496,961` are inside `mod tests`. Keeping
it (b) is the conservative reading; deleting it would delete a check.

**Same-named local variable.** The ledger already cleared `walk.rs`'s private `file_link`
(`:344`, `:349`, platform-gated) and `bedrock.rs`'s local `frame_bytes`. Both still hold: the
only live `file_link` is `epsilon-core/src/test_support.rs:54`. `SKIP_DIRS` has a **local**
collider in `epsilon-tools/tests/builtin_tools.rs:2186` (`let _ = util::SKIP_DIRS;`) — the row
the ledger already flags.

**Name in `references/` or `docs/` but not in code.** Every B1 name also appears in prose, and
none of it is a caller:
- `references/` hits are **all third-party projects**, not this codebase: `is_unknown` in
  `codewhale-ratatui`, `SKIP_DIRS` / `skill_roots` / `snapshot_rows` in `hermes-agent`,
  `bearer_token` in `hermes-agent` and `pi`. A substring scan over the workspace would have
  reported dozens of false callers here.
- `docs/` hits are milestone prose: `describe_task` at `docs/milestones/M50-task-agent-types.md:21`,
  `search_history` at `M18-alignment-pass.md:53`, `bedrock_event_payloads` at
  `M49-bedrock-converse-wire.md:27`, `active_runs` at `M87-rpc-admission.md:21`,
  `set_summarizer` at `M7-compaction-and-prompts.md:37`. **Doc mentions are not callers** — and
  `AGENTS.md` §9 says every command in the docs must have been run; these prose mentions are not
  commands, but a future round should not read them as evidence of wiring.

**Re-exported through a prelude / `pub use`.** I listed every `pub use` in all 14 `lib.rs` files.
None of the 29 B1 names is named in one. Five types carrying B1 methods **are** re-exported, so
deleting the method removes a method from a re-exported type:
`SessionLease` (`session/src/lib.rs:29`), `ExtManifest` (`ext/src/lib.rs:20`), `IdeApp`
(`desktop/src/lib.rs:64`), `EntryKind` (`contracts/src/lib.rs:38`), `SessionStore`
(`session/src/lib.rs:31`). Nothing in the workspace calls the B1 methods, so no re-export breaks
— but these are the rows where "delete a public API item" is literally true. No crate declares
`publish = false`; workspace version is `0.1.0` and nothing is published, so I treated "public
API" as *in-workspace reachability*, and measured it rather than assuming it.

---

## 4. Verdicts that differ from the ledger's implicit one

These are the rows where a mechanical "B1 = delete" sweep would have destroyed a record or a
control.

### `describe_task` — `epsilon-core/src/agents.rs:304` — **(b) keep**

The ledger files it "delete". Its doc says it is the "Task-tool description suffix listing
routable agent types (opencode `describeTask` parity)". I read the description it is supposed to
decorate: `epsilon-tools/src/task.rs:71` —

```rust
pub const TASK_DESC: &str = include_str!("../descriptions/task.txt");
```

It is a **static `include_str!`**. Nothing composes it with `describe_task`. And
`crates/epsilon-tools/descriptions/task.txt` — I read it in full — never names an agent type,
while `task.rs:210` emits `<subagent_type>{}</subagent_type>` to the model. **The task tool
asks the model for a subagent type and never tells it what the valid ones are.** That is a
missing feature, not dead code, and `describe_task` is the only record of it. The two tests at
`agents.rs:405,418` test the renderer, not the wiring.

### `Note_lag` — `epsilon-desktop/src/session.rs:616` — **(c) — this one is a live defect**

`Transcript::dropped()` is read in **production**: `desktop/src/app.rs:3813-3814`, inside
`status_line()`, renders `"{N} event(s) dropped"` into the window's status bar. `dropped` has
exactly one writer — `note_lag`, whose only caller is the test at `session.rs:1382`. The real lag
path is `session.rs:1012-1027`: on `RecvError::Lagged(n)` it sends an `AgentEvent::Notice`,
which lands at `session.rs:425` → `push(RowKind::Notice, text)` and **never touches the
counter**. So the status-bar segment is unreachable in production. This is `AGENTS.md` §2.7 —
a UI that advertises something it does not do — and it was filed as a plain delete. **Decision
needed:** wire the production lag branch to `note_lag` (one line) rather than delete it.

### `ExtManifest::wants_event` — `epsilon-ext/src/manifest.rs:41` — **(c)**

`host.rs:89` says the filter is "Evaluated by `wants_event`". It is not. Production applies the
same predicate **inline, twice**: `host.rs:301` and `host.rs:520` — both
`self.events.iter().any(|e| e == "*" || e == tag)`, which is `wants_event`'s body verbatim.
Deleting `wants_event` leaves a false doc; keeping it leaves a triplicate predicate.
**Decision:** have `host.rs:301` and `:520` call `wants_event`, which deletes two copies and the
false doc at the same time.

### `search_history` — `epsilon-core/src/lib.rs:1673` — **(c) — it has left Class B entirely**

The ledger filed it B2 with "the TUI `/search` searches the transcript". Today it has **zero
callers anywhere** — not production, not a test. Its 6 lines of doc promise pi
`search/scanning.ts` parity. Its substrate is alive (`epsilon-session::search_session`,
`search.rs:139`, 7 test sites), so the search machinery exists and nothing reaches it from the
agent. **It is now Class A-shaped, not Class B.** Delete it, or wire `/search` to it — but note it
is the *only* `Agent` entry point into a working search primitive.

### `KeybindingsManager::from_map` — `epsilon-tui/src/keybindings.rs:342` — **(c), not delete**

14 call sites, **all** in `epsilon-tui`, its own doc says "Test helper — inject a map directly".
Every caller is in `epsilon-tui` (`keybindings.rs`, `footer.rs`, `shell.rs`, `app/tests.rs`), so
it can go behind `#[cfg(test)]` with nothing else moving. That is a different change from a
deletion and a smaller one. **Decision:** add the attribute; do not sweep.

### `refused_heads`, `key_text_round_trips`, `push_external_text`, `SINGLE_GLYPH_FIELDS`,
`snapshot_rows` — **(b) keep**, all five

Each is the engine of a control, and in three cases the control has already caught something:
- `refused_heads` (`commands.rs:334`) powers `desktop/tests/window.rs:1668`, which asks the
  window to refuse every head the registry advertises — the `AGENTS.md` §2.7 check. Its own doc
  (:321) says it exists for that test.
- `key_text_round_trips` (`keybindings.rs:384-397`) is the assertion that a footer hint came
  from a real binding: "a hint that fails it is a key that does nothing".
- `SINGLE_GLYPH_FIELDS` (`theme.rs:516`) is the independent second list behind
  `the_glyph_name_list_and_the_table_agree`. Its doc (`theme.rs:471-473`) records that this check
  **missed `warn` for months** — it has already paid for itself. 39 lines, the largest B1 row,
  and the most clearly earned.
- `push_external_text` (`state.rs:975-984`) is documented as "the seam the sanitiser is tested
  through" — the only public door for injecting U+202E. Its three test sites
  (`app/tests.rs:5316,5343,5462`) prove bidi stripping. Deleting it deletes the only way to test
  a security control.
- `snapshot_rows` / `snapshot_rows_with_kinds` (`render_cache.rs:553,568`) are the only
  observation point on the TUI's render cache; both are test-only, and so is the cache's own
  output.

### Also **(c)**, with the decision named

- **`preview_target`** (`references.rs:385`) — doc says "for frontends that want to show it
  before submitting". No frontend does. **Decision:** wire a live `@`-reference preview, or drop
  the doc and the function together. Do not leave a doc describing a UI that does not exist.
- **`followup_queue_len`** (`core/lib.rs:2639`) — doc says "for a queue-depth reply"; its only
  callers are two `#[cfg(test)]` sites in `cli/src/rpc_mode.rs`. The RPC surface never sends a
  queue-depth reply. **Decision:** send it, or delete both.
- **`SessionLease::acquire_now`** (`lease.rs:238`) — doc says it is "the right answer for a
  caller that has its own timeout". No production caller; production always uses `acquire`.
  3 test sites. **Decision:** this is the refuse-immediately policy path for lease takeover
  (plan item 2). Delete it and the takeover work loses its seam; keep it and say why.
- **`MAX_BREAKPOINTS`** (`cache_policy.rs:41`) — a documented provider ceiling ("Anthropic
  accepts at most four") that **nothing enforces**. Its one use is
  `DEFAULT.count() <= MAX_BREAKPOINTS` in a test. **Decision:** keep as the named ceiling and
  cite it where a policy is built, or delete it and inline `3` into the test, which loses the
  sentence that says why three and not four.
- **`Transcript::open_approvals`** (`session.rs:392`) — 20 lines answering "can the user still
  act on this?", as against `rows()`'s "is it on screen?". The window renders approval rows
  (`app.rs:4573`) and tracks `answered` (`session.rs:597`), but never asks the second question:
  **there is no visual signal for an unanswered approval once its row has scrolled by.**
  **Decision:** surface it, or accept that answered and unanswered look identical.

---

## 5. B2 — file as a defect (re-checked, not re-argued)

| Item | file:line today | Callers | Verdict |
|---|---|---|---|
| `bedrock_event_payloads` | `epsilon-llm/src/providers/bedrock.rs:463` | 2, both in this file's tests (`bedrock.rs:659,663`) | **(b) keep** |
| `GoogleVertexWire::bearer_token` | `epsilon-llm/src/providers/vertex.rs:57` | 2, tests (`vertex.rs:199,214`) | **(b) keep** |
| `adc_jwt_input` | `epsilon-llm/src/providers/vertex.rs:85` | 3, tests (`vertex.rs:225,230,231`) | **(b) keep** |
| `Agent::search_history` | `epsilon-core/src/lib.rs:1673` | **zero** | **(c) — no longer Class B** |
| `ServeState::active_runs` | `epsilon-server/src/lib.rs:81` | 9, all tests (`lib.rs:608,665,668,720,724,739,792,795,963`) | **(b) keep** |
| `Agent::set_summarizer` | `epsilon-core/src/lib.rs:1154` | 4, all in `core/tests/` | **(b) keep** |

Two notes the ledger did not carry, both verified:

- **Bedrock.** `bedrock_event_payloads`'s own doc (`:458`) says "The transport layer calls this
  to recover the JSON payloads". It does not — the only callers are the tests. The transport
  feeds `SseEvent` straight to `bedrock_on_event` (`bedrock.rs:146`), and that function's doc
  (`:316-319`) says frames arrive "already unwrapped from the binary event-stream envelope" —
  which they cannot be, because nothing unwraps them. AWS `ConverseStream` is binary
  event-stream, not SSE. **Whether Bedrock actually fails at runtime was not measured here**
  (no build, no live provider). The unreachable named path is the finding, exactly as filed.
- **`set_summarizer`.** The slot is read in production (`lib.rs:1160-1171`, used at `:2920`), and
  with the slot empty it silently falls back to the **main** provider. So compaction works, it
  just always runs on the session's own model and its own bill. Not a defect, but the "dedicated
  auxiliary provider" is a knob no product surface turns.

---

## 6. B3 and B4 — keep and justify, site counts corrected

B3 (5 rows) stands as filed. Corrected reach:

| Item | file:line today | Real call sites |
|---|---|---|
| `test_support` (module) | `epsilon-core/src/lib.rs:28` (no `cfg`) | 7, **all** in `#[cfg(test)]` — incl. cross-crate `epsilon_core::test_support::dir_link` at `epsilon-tools/src/util.rs:999` |
| `LineReassembler` | `epsilon-llm/src/providers/mod.rs:1549` | 1 (`llm/tests/wire_fixtures.rs:154`) |
| `from_script_json` | `epsilon-llm/src/faux.rs:101` | 1 (`faux.rs:345`) |
| `recorded_requests` | `epsilon-llm/src/faux.rs:108` | 3 (`faux.rs:340,351`; `core/tests/loop_scenarios.rs:676`) |
| `recorded_requests_handle` | `epsilon-llm/src/faux.rs:113` | **21** (1 in `cli/src/main.rs` tests, 20 in `core/tests/`) |
| `load_or_build_test_manifest` | `epsilon-ext/src/manifest.rs:74` | 3 (`ext/tests/event_forwarding.rs:104,108`) |
| `HttpEventStream::empty_for_tests` | `epsilon-llm/src/providers/mod.rs:247` | 12, all tests |

The B3 reasoning holds and is now sharper: `test_support` is reached from `epsilon-tools`
(`util.rs:999`) — a **different crate** — so it cannot go behind `#[cfg(test)]` in
`epsilon-core`. The ledger's proposed fix (a workspace dev-dependency) is the right one and it
is still undone.

B4 (15 items) stands as filed. Corrected counts, for anyone re-deriving:

`without_agent` **14** ✅ · `set_write_fault_for_tests` **5** (4 in `core/tests/`, 1 in
`cli/src/main.rs` tests) · `FrameSummary::drew` **25, not 42** (17 in `app.rs`, 8 in
`tests/window.rs`) · `::hotspot` **24, not 25** · `seed_transcript` **31, not 27** · `set_draft` 6
✅ · `set_focus` 4 ✅ · `run_palette_command` 5 ✅ · `sidebar_view` 1 ✅ ·
`store_handle_for_tests` **8, not 7** · `start_goal_round_for_test` 1 ✅ ·
`write_fault_pending_for_tests` 1 ✅ · `mark_dead` 1 ✅ (`mcp/src/lib.rs:289`, inside the
`#[cfg(test)]` at `:244`) · `dir_link` 7 · `file_link` 1.

---

## 7. The (a) rows — what has to go with them, and was the test worth more?

No `use` statement anywhere in the workspace names any of the 15, so no deletion orphans an
import and `clippy -D warnings` has nothing to catch. **One exception, and it is a real one:**

### ⚠ `Skill::load_body` (`epsilon-core/src/skills.rs:29`) — deleting it breaks the build

`strip_frontmatter` at `skills.rs:59` is **private** and is called only by `load_body` (`:32`)
and its own test (`:403`). Delete `load_body` and `strip_frontmatter` becomes a private function
with no non-test caller — which `rustc`'s `dead_code` lint reports in a normal build. The
`clippy -D warnings` gate is `-D warnings`, so this **fails the build**, not just the test. The
correct sweep is `load_body` **and** `strip_frontmatter` **and** the test at `:403`, together.

The reason `load_body` is dead is itself the finding: **production loads skill bodies in
`epsilon-tools/src/skill.rs:153-164`**, with its **own private copy** of `strip_frontmatter` at
`:188`. `epsilon-runtime/src/lib.rs:5140` reads `SKILL.md` a third time. Three readers of one
file format; the `epsilon-core` one is the orphaned twin.

| # | Item | Test sites that go | Was the test worth more? |
|---|---|---|---|
| 1 | `keys_text` | `footer.rs:467,552,554,560` (4) | **No — and the fix is free.** `footer.rs:460 full_line()` renders the same strings through the real draw path (`render()` at `:443`). Re-point the 4 assertions at `full_line` and the test gets *stronger*, per `AGENTS.md` "assert on the frame". |
| 2 | `probe_event_forward_latency` | `host_protocol.rs:864` (1) | Borderline. The ADR-019 "~2.0 s" measurement lives in its doc (`:316-321`); move that sentence to the test before deleting. |
| 3 | `usable_count` | `fallback.rs:209,211` (1 test, 2 lines) | **No.** The same test already uses `p.chain.len()` at `:207,212`. Two assertions collapse to one. |
| 4 | `is_watching` | `diff.rs:438,455,482` (1 test, 3 asserts) | **No.** The assertions are about `begin()` refusing a command string as a path; they do not need a getter. |
| 5 | `is_custom_model_id` | `custom.rs:173,175,178` (1 test) | **No.** One-line wrapper over `lookup_custom`, which has 4 live production callers (`providers/mod.rs:1397`, `fallback.rs:98`, `runtime/lib.rs:5829,5934`). Delete it — and delete the doc at `:106-107`, which claims "the question the CLI asks while resolving a model"; the CLI asks `lookup_custom`. |
| 6 | `is_unknown` | `session_entry.rs:551`; `session/tests/session_store.rs:2089,2192` (3) | **No.** Two of the three already write `matches!(kind, EntryKind::Unknown{..})` alongside the call. |
| 7 | `session_allows` | `gate.rs:225,273`; `safety/tests/additive_authorization.rs:224` (3) | **No.** Reads one `Mutex<Vec<_>>`; tests can promote then assert on the effect they already assert. |
| 10 | `load_body` | `skills.rs:385` **+ `:403`** | **No** — but see the cascade above. This one fails the build if swept alone. |
| 13 | `with_max_bytes` | `checkpoint.rs:982` (1) | **No.** Its doc says "tests, and a future config knob". The knob does not exist; `CheckpointStore::open` (`:144`) is the only constructor in production. |
| 17 | `SKIP_DIRS` | `tools/tests/builtin_tools.rs:2186` (1) | **No — and the test says so itself:** `// referenced so the constant stays documented in tests too`. A test whose stated purpose is to keep a constant alive is the weakest kind of test. Ignore the 4 Python `SKIP_DIRS` in `scripts/` — unrelated locals. |
| 18→(b) | `refused_heads` | — | **The test is worth far more.** Kept; see §4. |
| 19 | `has_trust_requiring_resources` | `runtime/lib.rs:1858` (1) | **No.** `trust_requiring_resources` (`:655`) is live; the boolean is a one-line convenience over `.is_empty()`. |
| 20 | `discover_agents` | `agents.rs:346,381,389,417,497,541` (6) | **No, but it is the widest sweep in the table.** 6 sites, all rewriting `discover_agents(` → `discover_agents_with_notices(`. Check each drops its notices sink honestly — `runtime/src/lib.rs:5197` records that a call site once used the wrapper and lost its notices, with a negative control. |
| 22 | `skill_roots` | `runtime/lib.rs:1987` (1) | **No.** The test asserts `skill_roots(ws) == skill_roots_trusted(ws, true)` — i.e. that the wrapper adds nothing. It is a test of a wrapper. |
| 23 | `extension_roots` | `runtime/lib.rs:1988` (1) | **No.** Same shape, same test. |
| 24 | `latest_session_file` | `runtime/lib.rs:1778,1796` (2) | **No.** Production uses `latest_session_file_in`. The doc references at `:4801,5005` are comments about the old name — they need editing or they become stale prose. |

---

## 8. One more row the ledger parked in "false alarms"

`SessionStore::branch_with_summary` — `epsilon-session/src/store.rs:1695` — the ledger calls it
"still dead … belongs in the next round's decision". It has **7** call sites (not 8):
`core/tests/compaction_scenarios.rs:635` and `session/tests/session_store.rs:2677,2688,2800,2829,2870,2900`.
51 lines, no production caller, and `tui/src/app/commands.rs:1149` records it as "a faithful port
with the same defect class". Verdict **(c)**: wire `/fork`, or delete 51 lines and a test file's
worth of branch coverage. It is the largest single item in this class and it is not in the
"50 items" count anywhere.

---

## 9. Do these first

Ordered by *safety × size*, not by size. Each needs its own battery (`AGENTS.md` §4).

1. **`followup_queue_len`, `preview_target`, `SessionLease::acquire_now`,
   `has_trust_requiring_resources`, `extension_roots`** — the five (a)-adjacent rows that are
   one-line wrappers or policy seams whose **decision** is the cheap part. Deciding them costs a
   paragraph and closes 5 of the 8 (c)s.
2. **`Skill::load_body` + `strip_frontmatter`** — the only row in this class that **fails the
   build** if swept alone. Do it as one commit, with `skills.rs:403`. It also removes one of the
   three `SKILL.md` readers.
3. **The seven thin getters** — `usable_count`, `is_watching`, `is_custom_model_id`,
   `is_unknown`, `session_allows`, `with_max_bytes`, `SKIP_DIRS` (+ its self-admitted keeper
   test). ~30 lines, 12 test sites, zero production impact. One battery.
4. **`keys_text`**, re-pointed at `full_line` — the one delete that *improves* a test.
5. **`ExtManifest::wants_event`** — make `host.rs:301` and `:520` call it. Deletes two inline
   copies of the predicate and the false doc at `host.rs:89`.
6. **`Transcript::note_lag`** — **wire it before deleting anything.** One line at
   `session.rs:1016` fixes a status bar that can never report. This is the only Class B row I
   would call a defect rather than a cleanup.
7. **Last, and not as a sweep:** `describe_task` (wire it into `task.txt` or delete it and the
   feature gap with it), `branch_with_summary` (51 lines), `search_history` (re-file as Class A),
   `refused_heads` / `key_text_round_trips` / `push_external_text` / `SINGLE_GLYPH_FIELDS` /
   `snapshot_rows` (kept).

**Do not** take the header's "50 items, 652 lines" into a plan, a STATUS line or a PR title. The
tables hold 40 rows; the B4 list holds 15 more; neither number is 50 and neither sum is 652.