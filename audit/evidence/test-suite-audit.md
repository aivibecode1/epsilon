# Test-suite credibility audit (2026-09-19)

## Method and coverage of this audit (what you read, what you ran)

**Read, not run.** No `cargo` command was executed (the brief warned of a concurrent build holding the
lock); every claim comes from source reading. Nothing was modified except this file.

1. **Census.** Every `crates/**/*.rs` was parsed: each `#[test]`/`#[tokio::test]`/`#[tokio::test(...)]` attribute located, its `fn` name and brace-matched body extracted (comment- and raw-string-aware), and each body scanned for `assert!`/`assert_eq!`/`assert_ne!`/`panic!`/`expect(` — 884 test functions, ~796 KB of body text. A second-pass script failed to extract 12 bodies; the first pass covered 880/884 and backs the zero-assertion numbers. Every candidate was re-read by hand before being reported.
2. **Targeted greps** for `assert!(true)`, identical-operand `assert_eq!`, `len() >= 0`, `.is_ok()`-only asserts, `#[should_panic]`, `#[ignore]`, `#[cfg(...)]`-gated tests, `let _ = ...` discards, and expectations built from production constants.
3. **Public-API reachability.** 381 `pub fn`s from `epsilon-core`, `epsilon-session`, `epsilon-tools`, `epsilon-llm` checked against the concatenated text of all 884 test bodies — *name* reachability, not a call graph: "no test body anywhere names this function".
4. **Independent re-verification.** Two delegated readers swept `epsilon-core` and (`epsilon-tools` + `epsilon-session`); every claim kept from them I re-opened and read at the cited line. Four candidates were dropped as false positives (last section), and a hermeticity check of test isolation against the real filesystem produced V-4.

**Verdict:** this suite is materially better than the genre norm. The classic cannot-fail shapes are nearly
absent: **0** `#[should_panic]`, **0** `assert!(true)`/self-comparison/`len() >= 0`, **1** `#[ignore]`, and
after hand-verification exactly **one** of 884 bodies contains no assertion. The defects are narrower: tests
asserting on their own construction, names promising more than bodies prove, and a hermeticity failure in
`epsilon-tui` documented in a comment rather than fixed.

## Totals per crate (table: crate, tests, ignored, integration test files)

**884 test functions** (579 `#[test]` + 301 `#[tokio::test]` + 4 `#[tokio::test(start_paused = true)]`),
**1 ignored**, **28 integration test files**.

| crate | tests | unit | integration | ignored | integration files |
|---|---|---|---|---|---|
| epsilon-cli | 57 | 48 | 9 | 0 | 2 |
| epsilon-contracts | 25 | 25 | 0 | 0 | 0 |
| epsilon-core | 207 | 108 | 99 | 0 | 13 |
| epsilon-ext | 5 | 0 | 5 | 0 | 2 |
| epsilon-llm | 153 | 107 | 46 | **1** | 3 |
| epsilon-lsp | 23 | 18 | 5 | 0 | 1 |
| epsilon-mcp | 6 | 4 | 2 | 0 | 1 |
| epsilon-safety | 59 | 45 | 14 | 0 | 4 |
| epsilon-server | 15 | 15 | 0 | 0 | 0 |
| epsilon-session | 43 | 16 | 27 | 0 | 1 |
| epsilon-tools | 128 | 82 | 46 | 0 | 1 |
| epsilon-tui | 163 | 163 | 0 | 0 | 0 |
| **total** | **884** | **631** | **253** | **1** | **28** |

`epsilon-core/src/lib.rs` (3129 lines, the agent loop) has **0** `#[cfg(test)]` modules and **0** unit
tests — integration-only coverage. `epsilon-tui` ships no `tests/` directory (163 unit tests, 68 inside the
7101-line `app.rs`); `epsilon-contracts`, `epsilon-server` and `epsilon-tui` ship no integration tests.

## Findings

### V-1 — HIGH — `crates/epsilon-tools/tests/builtin_tools.rs:1382`
Claims the walker honours the skip list; never calls the walker, so it cannot fail for any input.
```rust
fn util_walker_respects_skip_list() {
    let _ = util::SKIP_DIRS; // referenced so the constant stays documented in tests too
```
The behaviour *is* covered at `util.rs:843` (creates `target/`, asserts exclusion), so the harm is a
misleading green rather than a hole. Prove it: delete it, or assert on `walk_files` output.

### V-2 — HIGH — `crates/epsilon-tools/src/webfetch.rs:291`
`max_chars_truncation_shape` claims the tool's `max_chars` truncation works, but rebuilds the string locally
and asserts on its own construction — production is never called.
```rust
let kept: String = body.chars().take(limit).collect();
let out = format!("{kept}\n... [{truncated} more chars truncated by max_chars={limit}]");
assert!(out.contains(&"x".repeat(100)));
```
Deleting the real branch (`webfetch.rs:220-226`) leaves it green. Prove it: extract a pure
`apply_max_chars(body, limit)` from the async `execute()` and assert on production output.

### V-3 — HIGH — `crates/epsilon-core/tests/loop_scenarios.rs:646`
`task_depth_cap_refuses_nested_delegation` claims the depth cap refuses nesting; it asserts the **opposite**
(depth-0 succeeds) and closes with a comment pointing at a proof that does not exist.
```rust
assert!(res.text.contains("parent done"), "got: {}", res.text);
// Nested refusal (child at depth 1 calling run_task again) is proven at
// the tool level below: a scripted child that issues its own `task` call
```
The `fn` ends two lines later, and grepping `depth` across all 13 `epsilon-core` test files returns only
this test and its comments: **nested-delegation refusal has no test anywhere**, yet the depth cap is the
runaway-fan-out guard. Prove it: a scripted child issuing its own `task` call, asserting the depth message
returns as a tool failure.

### V-4 — HIGH — `crates/epsilon-tui/src/app.rs:2391` (tests at `:6554-6790`) — tests are not hermetic
`apply_model` unconditionally calls `save_model_id` (`:2408`) and `push_recent_model` (`:2426`), which
resolve through `epsilon_core::config::global_dir()` = `$HOME`/`$USERPROFILE` `.config/epsilon`, with **no
test override**. TUI tests reach it via F2 / the model picker, and the suite knows:
```rust
// Persistence side effects: cycling routes through apply_model, so the
// saved model id must reflect the last switch. ... asserting the real global
// config here races every other TUI test's writes (lesson 16b)
```
A known race was documented instead of sandboxed. Physical evidence on this machine:
`C:\Users\Hp\.config\epsilon\config.toml` contains `"faux-1"` in `[picker] recent` — a test-only model id in
the developer's real config. Second consequence: `app.rs:6714` (`f2_cycles_favorites_and_guides_when_empty`)
asserts "empty list → guidance" against `app.favorites`, loaded from that same file, so it passes here only
because the real config happens to hold no favorites. Prove it: point `HOME` at a `tempfile::tempdir()` for
the test process, then assert the persisted bytes.

### V-5 — MED — `crates/epsilon-core/src/compact.rs:811`, `:820`
Both claim multi-byte clipping is panic-free **and** the body carries a marker; the assertion cannot tell a correct clip from a broken one, because `summary_context` always pushes literal text into `system_prompt`, so the first disjunct is true by construction: `assert!(!ctx.system_prompt.is_empty() || !ctx.messages.is_empty());`. The file's comment says "body contains a clipped marker"; nothing checks it. Prove it: assert the marker and the retained byte count in `ctx.messages`.

### V-6 — MED — `crates/epsilon-core/src/registry.rs:138`
The name claims the M50 guarantee (doc: "sharing the same tool Arcs … never copies"); the body checks names only — `assert!(sub.get("write").is_some()); assert!(sub.get("read").is_none());` — and no assertion can see a copy. Prove it: `Arc::ptr_eq` between the original and subset registrations.

### V-7 — MED — `crates/epsilon-core/tests/loop_scenarios.rs:1262`
An assertion inside the test double's `prepare_next_turn`: `assert!(matches!(ctx.message, Message::Assistant(_)), "the hook sees the turn's own assistant message");`. Production reaches this hook through `contain_hook(..., None)` (`src/lib.rs:1486`), which turns a panic into the fallback `None`, so the assertion cannot fail the test — dead code shaped like a proof. Prove it: record the observed message and assert in the test body.

### V-8 — MED — `crates/epsilon-core/src/config.rs:687`
`precedence_documented_flag_over_saved_over_default` claims to pin the model-resolution precedence, then re-implements the expression inline and asserts on its own locals — the product code is never invoked (the comment concedes the logic lives "at call sites"): `let resolved = flag.or(saved).unwrap_or_else(|| DEFAULT_MODEL_ID.into()); assert_eq!(resolved, "claude-opus-4-1");` Prove it: call the real resolver with an injected flag/saved pair.

### V-9 — MED — `crates/epsilon-llm/src/faux.rs:335`
The name claims two things, the body checks one: both streams are discarded with `let _ =`, so the exhausted "empty EndTurn" — the half the faux provider's contract depends on (`faux.rs:78-79`) — is unverified: `let _ = p.stream(...).await; let _ = p.stream(...).await; // exhausted -> empty EndTurn` then `assert_eq!(p.recorded_requests().len(), 2);`. Prove it: fold the second stream and assert its terminal message text is empty.

### V-10 — MED — `crates/epsilon-session/tests/session_store.rs:358`
`lease_blocks_second_writer_until_released` claims a lease blocks a writer *until released*; nothing is released and nothing writes — `assert!(matches!(SessionLease::acquire(&session).unwrap(), epsilon_session::LeaseOutcome::Held { .. }));`. Release is covered at `lease.rs:187`; the name here still over-promises. Prove it: drop the lease, re-acquire, write.

### V-11 — MED — `crates/epsilon-tools/src/util.rs:932`
Tautology — `str::len()` is always a char boundary, so this cannot fail: `assert!(t.text.is_char_boundary(t.text.len()));`. The real claim is carried by the panic-if-split behaviour and `contains("好")`. Prove it: assert the byte at the cut is not a UTF-8 continuation byte.

### V-12 — MED — `crates/epsilon-tools/tests/builtin_tools.rs:348`, `:357`
Expectations built from the same constant production uses, so a wrong cap still passes — the brief's category 4, worst instance found (a *boundary* use, so it still proves a cap exists at whatever value it holds): `for size in [util::MAX_READ_BYTES - 1, util::MAX_READ_BYTES]` and `check_edit_source(edit_source_of_size(util::MAX_READ_BYTES + 1), Some("limit")).await;` Damage limited: siblings do use literals (`:657` "exceeds 256.0KB limit", `:869` "262144"). Prove it: `262_144`/`262_145`.

### V-13 — MED — `crates/epsilon-llm/tests/live_smoke.rs:71`
The only test touching a real provider wire is `#[ignore]`d **and** self-disabling: with no credential it prints and returns a pass — `let Some(info) = resolve_candidate() else { /* "Not a failure: an unauthenticated machine is a legitimate state." */ eprintln!("live smoke: skipped — no credential found"); return; };`. On any unconfigured machine — including CI, as the doc intends — request build → send → stream fold has **zero** executed coverage while the suite reports green, though the module doc claims the file "closes that gap". Prove it: make the no-credential path fail loudly under an opt-in strict flag.

### V-14 — MED — `crates/epsilon-tools/src/lib.rs:150` + `src/shell.rs:85`
Two public entry points on the product path with no test. `builtin_registry_with` is what `main.rs:1547` uses (`Lsp::Auto`), wiring the diagnostics provider and the secret-scrubbing `scrub_env` callback into `write`/`edit`; tests only call `builtin_registry` (`Lsp::Off`). `with_policy` ("Override the confinement policy") has **no caller anywhere in the workspace** — not production, not tests — so `BashTool.policy` can only ever be `ConfinementPolicy::default()` and the documented knob is unreachable. Prove it: a test calling `builtin_registry_with(_, Lsp::Auto)` and asserting `write`/`edit` carry a diagnostics provider.

### V-15 — LOW findings (each with anchor + snippet)
- `epsilon-session/tests/session_store.rs:355` (`:164`, `:954`, `:1189`) — dead witnesses:
  `let _ = tool_result_msg("tc_1", "bash"); // shape witness for future assertions` — `tool_result_msg`
  (`:76`) is used nowhere else.
- `epsilon-core/tests/guard_scenarios.rs:183` — vacuous with no tool results:
  `for r in tool_results(&agent) { assert!(!r.contains("[harness]"), ...); }` (the sibling at `:115` pins
  `results.len() == 4`).
- `epsilon-core/src/prompt.rs:639` — "nearest directories win" is untestable as written (all eight
  candidates are depth 0, so deleting the sort at `prompt.rs:244` leaves it green); the count is asserted
  against production — `assert_eq!(hints.len(), HINT_MAX_FILES, ...)`.
- `epsilon-session/tests/session_store.rs:1246` (`:559`) — the version is compared to the same constant
  production writes; `:511` does it right, anchored by `"version":3` at `:532`.
- `epsilon-tools/tests/builtin_tools.rs:1334` — claims a locator, never opens it:
  `assert!(out.contains("full output saved to"));` (`util.rs:866` reads the file back — unused here).
- `epsilon-tools/src/util.rs:981` — the `0700` spill assertion sits in `#[cfg(unix)]`, so it never runs on
  the development platform; production `0700` is unix-only too (`:617-622`, `:649-653`).
- `epsilon-tools/tests/builtin_tools.rs:346`, `:355`, `:360`, `:367`, `:1279`, `:1285` — six tests whose body
  is one call to a helper holding the assertions (`bash_common().await;`): a helper edit empties six tests.
- `epsilon-tui/src/app.rs:6554` — claims a substitution ("quits **instead of** running against unreviewed
  config") but asserts only the quit: `assert!(type_and_enter(&mut app, &agent, "trust no"), ...)`.
- `epsilon-core/src/compact.rs:892`, `tests/task_scenarios.rs:391`, `tests/loop_scenarios.rs:591` —
  "says how many" checked only by the phrase `"omitted for length"` (contrast `compact.rs:1047`, which pins
  `"(+6 more)"`); a delegated model asserted only in reported tool metadata, never in the request; a `task`
  context override whose `model` third is unasserted and unobservable (scripted model = agent default).

### Worst 10 name/body mismatches (ranked)
1. `epsilon-core/tests/loop_scenarios.rs:646` — the sole named property is untested; the comment claims a proof that does not exist. **HIGH**
2. `epsilon-tools/tests/builtin_tools.rs:1382` — body is `let _ = CONST;`. **HIGH**
3. `epsilon-tools/src/webfetch.rs:291` — tests its own string math, not the tool. **HIGH**
4. `epsilon-core/src/config.rs:687` — re-implements the logic and asserts on the copy. **MED**
5. `epsilon-core/src/registry.rs:138` — "shares the tool / never copies" unasserted. **MED**
6. `epsilon-session/tests/session_store.rs:358` — "until released" with no release. **MED**
7. `epsilon-llm/src/faux.rs:335` — the "empty EndTurn" half is discarded. **MED**
8. `epsilon-tools/tests/builtin_tools.rs:1334` — "with locator", locator never opened. **MED**
9. `epsilon-tools/src/webfetch.rs:302` — "spill_helper_reused" tests the helper, not webfetch. **LOW**
10. `epsilon-core/src/compact.rs:892` / `epsilon-tui/src/app.rs:6554` — a phrase asserted where a count / a non-run was claimed. **LOW**

### `#[should_panic]` and `#[ignore]`
**No `#[should_panic]` anywhere**, so the "message too loose to distinguish the intended failure" category is
empty: the repo uses `Result` + `unwrap_err().contains(reason)` instead (`builtin_tools.rs:319-327`), which
is strictly stronger. Exactly **one** `#[ignore]` (`live_smoke.rs:72`) — the only coverage of the real
provider transport, handled at V-13. No other crate has an ignore; nothing is `#[cfg]`-gated away on Windows.

## Behaviours with no coverage

Reachability is **by name search** (method §3); where indirect coverage plausibly exists I say so rather than
claiming a hole. **Genuinely uncovered, no tested twin:**
- `epsilon-core/src/config.rs:20 load_config_string_in` / `:32 load_config_string` — the M59 external-editor config reader (TOML table/key lookup; "a config typo must not break a session"): the only config reader in that module with no `_in` twin test.
- `epsilon-core/src/config.rs:125 accept_project_config_hash_in` / `:138 current_project_config_hash_in` — the accept-this-project-config trust decision, called from `app.rs:1129`, `:1131`, `:1809`; nothing tests that an accepted hash stays accepted or that a changed one is re-asked.
- `epsilon-tools/src/shell.rs:85 with_policy` and `src/lib.rs:92 register_builtins_with` / `:150 builtin_registry_with` — see V-14.
- `epsilon-llm/src/auth.rs:148 enterprise_domain_of_in` / `:158 enterprise_domain_of` — GitHub Copilot enterprise domain resolution; no test names either, and there is no tested `_in` variant.
- `epsilon-llm/src/auth.rs:526 save_oauth_full_in` — the full-credential OAuth save (5 bodies cover `save_oauth_in`, none covers this).
- `epsilon-llm/src/catalog.rs:514 resolve_api_key` — the documented key-precedence point ("stored key wins", `auth.rs:11`); reached indirectly via `row_credentials_ready` only.
- `epsilon-llm/src/catalog.rs:453 resolve_model_for_run` — what the CLI and `doctor` call (`main.rs:1147`, `doctor.rs:309`); only the injected `_with` variant is tested.
- `epsilon-llm/src/providers/mod.rs:494 failure_message`, `:1187 try_stream_direct` — provider error text and the direct-stream entry point.
- `epsilon-session/src/store.rs:598 truncate_to_index`, `:740 pop_last`, `:1130 record_turn_end`, `:445 reindex`, `:1143 into_message` — public store/repair API; `record_turn_end` is reached via `repair.rs:133`, `pop_last` from `epsilon-core/src/lib.rs:906`.
- `epsilon-core/src/lib.rs:875 search_history` and `:1762 compact_now` — the `/search` and `/compact` backends: TUI tests drive the features through `handle_key`, but the functions' own contracts (result shape, ordering, failure modes) are pinned nowhere.
- `epsilon-core/src/lib.rs:597 set_skills`, `:613 set_agent_brief`, `:634 skill_names`, `:659 restored_len` — the skills/brief setup API; no test names them.

**Reached only indirectly (name-search gaps, not behaviour gaps):** `compact.rs:12 estimate_tokens` (via
`estimate_messages`), `guards.rs:47 result_signature`, `exec.rs:61 fail_all_truncated`, `references.rs:315
is_sensitive` (via tests named for sensitivity), `session/discover.rs:81 read_session_header`,
`tools/util.rs:176/195/207 read_text_*`, `edit_match.rs:96 normalize_to_lf`, `:518 find_exact_occurrences`,
`util.rs:353 format_size`, `:580 current_tail`, `search.rs:319 truncate_grep_line`.

**Why this list is short:** the workspace systematically pairs each global-dir function with an injectable
`_in` twin and tests the twin (`load_model_id_in`, `toggle_favorite_model_in`, `push_recent_model_in`,
`append_prompt_history_in`, `oauth_access_token_in`, `store.rs:258 open_in`). That discipline is the main
reason the uncovered count came out at 81/381 by name rather than far worse: wrappers are thin, cores pinned.

## The 5 strongest tests (file:line, why they are good)

1. **`crates/epsilon-tools/tests/builtin_tools.rs:293-374`** (`check_edit_source` + the four `edit_source_*`
   cases) — drives the real `EditTool` in three modes against a 256 KB file and asserts the file's **full
   bytes** equal the expected bytes, plus the refusal reason *and* that a refused edit produced no checkpoint.
   A truncating edit, the classic data-loss bug, cannot pass.
2. **`crates/epsilon-core/src/compact.rs:696`** (`no_generated_transcript_can_produce_an_illegal_tail`) — a
   seeded, dependency-free LCG generates 400 transcripts and checks the orphaned-tool-result invariant at
   **every** cut point, not the three hand-written ones. Property testing where an example would have done.
3. **`crates/epsilon-llm/tests/oauth_login.rs:263`, `:281`, `:317`, `:338`** — four
   `#[tokio::test(start_paused = true)]` device-poll tests using tokio's paused clock instead of `sleep`, with
   the RFC 8628 branches pinned separately: pending→slow-down, honoured server interval, plain timeout
   mentioning clock drift, terminal failures verbatim. Deterministic time done properly.
4. **`crates/epsilon-llm/src/auth.rs:1038`** (`corrupt_file_blocks_writes_without_clobber`) — writes garbage
   into the credential store, then asserts save *and* remove refuse, name "corrupt", and leave the file
   **byte-identical**: the failure mode and the no-data-loss guarantee, not just "returns Err".
5. **`crates/epsilon-core/tests/compaction_scenarios.rs:218`** (`for_model_resolves_catalog_windows`) —
   asserts `Some(1_000_000)` and `16_384` as **literals** rather than importing the catalog constants
   production reads: the correct answer to V-12's pattern, in the same workspace.

Honourable mentions: `epsilon-tools/src/util.rs:843` (creates `target/`, asserts exclusion — the test V-1
pretends to be); `epsilon-session/tests/session_store.rs:1192-1346` (fork test: exact ancestry, gapless
`seq`, byte-identical untouched source); `epsilon-cli/src/rpc_mode.rs:467` (legacy wire keys against literal
JSON, "pi reads `data`"); `epsilon-core/src/prompt.rs:432` (byte-identical stable prefix across a context change).

## What you could not check

- **Nothing was compiled or executed** — no `cargo` invocation at all, so I cannot report which tests pass,
  whether any fail here, or wall-time. Every "cannot fail" claim is a static argument (an assertion that is
  tautological, or one the code cannot reach), each quoted so it can be checked by eye.
- **Name reachability, not call-graph reachability.** The no-coverage list can have false negatives (an
  indirectly exercised `pub fn` whose caller's test never names it); indirect cases are marked.
- **Doc-tests are not counted.** `cargo test` also runs `///` examples; the 884 is the unit+integration
  figure, not the full `cargo test` total.
- **Four false positives were removed after hand-verification:** (a) `epsilon-llm/src/sanitize.rs:505` and two `tests/wire_fixtures.rs` cases only *looked* assertion-free — the first parser mis-handled `r#"…"#` raw strings; (b) `epsilon-llm/src/oauth_login.rs:662 poll_device_code` was reported uncovered because those four bodies use `#[tokio::test(start_paused = true)]`, a spelling the regex missed — it is well covered; (c) `builtin_tools.rs:346/355/360/367` and `:1279/1285` delegate to helpers that do assert (downgraded to V-15); (d) `util.rs:981` and the `bwrap` suite are platform-gated by design and run in Linux CI.
- **`epsilon-safety`, `epsilon-lsp`, `epsilon-mcp`, `epsilon-ext`, `epsilon-server`, `epsilon-cli` were sampled, not swept**, beyond the census and the targeted greps. Counts are exact; vacuity review is not exhaustive. `epsilon-ext` (5 tests for a whole crate) and `epsilon-server` (15, no integration files) are the least examined relative to their surface.
- **Not verified:** whether the `epsilon-safety` Windows Job Object path and the Linux `bwrap` path gate the same behaviours, and whether the CI MSRV / `--no-default-features` lanes compile the test files at all — a `#[cfg(feature = "real")]` test file (`live_smoke.rs:22`) can be silently absent from a reduced-feature run, which would make V-13's gap total rather than merely conditional.
