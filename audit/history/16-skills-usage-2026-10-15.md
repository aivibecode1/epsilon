# 16 — Skills discovery and usage-accounting audit (2026-10-15)

**Provenance, stated first.** Written by a read-only subagent of the 2026-10-15 campaign
that wrote this report to disk and was still running when its batch consolidated; its
final chat answer never arrived. The report below is its full text. The two headline
results are the integrator's verification targets for the next wave: the Gemini
`thoughtsTokenCount` under-count (half 2c) and the untested user-tier root table (G1).

---

# Audit 2026-10-15 — criterion 4 (skills discovery) + criterion 6 (usage accounting)

Repo HEAD: `08798f9` ("docs(parity): P-158 and P-200 move to done…"). Read-only; no cargo run.
Scope: `crates/epsilon-core/src/skills.rs` (cross-harness root table landed 2026-10-13) and the
usage pipeline (`epsilon-llm` wire boundary → `epsilon-core` run loop → `epsilon-tui` pricing).

---

# HALF 1 — criterion 4: skills discovery test coverage

## What exists

`crates/epsilon-core/src/skills.rs` carries **24 unit tests** (lines 512–1232) and **no other
crate has a discovery test**: grep of `user_roots()` / `project_roots()` / `all_roots(` /
`discover_with_notices(` / `skills::discover(` across all crates finds callers only in
`epsilon-runtime/src/lib.rs:772` (composition), `:862` (`agent_user_roots`),
`epsilon-tui/src/app/commands.rs:511` and `:1685` (the `/skills` listing and `/reload`) —
production code, no assertions on the tables.

The 24 tests, by clause:

| Clause (criterion 4) | Tests covering it (skills.rs) |
|---|---|
| Discovery across global + project-local dirs (project tier) | `a_skill_written_for_another_harness_is_discovered_and_loaded` (:952, all nine project roots discovered *and* body-loaded, plus every `project_roots` entry reaches `all_roots`); `the_project_roots_are_the_conventions_they_claim_to_be` (:1202, exactly nine conventions, alias last); `an_untrusted_workspace_contributes_no_roots_and_the_user_tier_survives` (:1085, trust gate); `a_clean_tree_is_silent_and_discover_still_returns_the_same_skills` (:783) |
| Discovery across global dirs (user tier) | **none — see G1** |
| Loading (bodies, frontmatter, validation) | `load_body` exercised at :560, :1013, :1043, :1074, :1142; `frontmatter_edge_cases` (:576); `catalog_lists_name_and_description_only` (:564); rejection tests :546–550, :816–847 |
| Conflicts / precedence (deterministic) | `discovers_validates_and_shadows_by_priority_root_order` (:535, last root wins); `a_collision_names_both_paths_and_says_which_one_won` (:740, both paths + winner named); `within_a_tier_the_portable_agents_alias_outranks_the_tool_specific_directory` (:1025); `a_workspace_skill_outranks_a_user_skill_of_the_same_name` (:1055) |
| Duplicate detection (dedupe) | `one_directory_spelled_two_ways_is_scanned_once` (:1114, `.`/`..` spellings collapse; observable half = no self-shadow notice); `the_dedupe_keeps_the_first_spelling_rather_than_the_last` (:1151); `a_root_that_does_not_exist_still_gets_a_usable_key` (:1172, nearest-existing-ancestor fallback) |
| Missing directories | `a_root_that_is_simply_absent_is_not_a_notice` (:723); `a_root_that_cannot_be_read_is_named_with_its_reason_instead_of_reading_as_empty` (:692, root is a file); `a_root_that_is_a_link_to_nothing_is_announced_rather_than_read_as_absent` (:880, dangling link); `a_skill_md_that_is_there_but_unreadable_is_announced` (:855, invalid UTF-8); `a_root_that_does_not_exist_still_gets_a_usable_key` (:1172) |
| Notices / catalog policy (adjacent) | `the_catalog_lists_only_what_the_model_may_load` (:512); `a_catalog_of_nothing_loadable_is_empty_rather_than_a_bare_heading` (:525); four `disable-model-invocation` spelling tests (:616, :636, :659, :676) |

## Platform blind spots — what is actually cfg-gated

`skills.rs` contains **zero `#[cfg(unix)]`/`#[cfg(windows)]`** — every platform-sensitive
behaviour is delegated, and both delegates are the documented portable design:

1. `root_is_absent` (skills.rs:417–419) → `crate::paths::is_link` (paths.rs:257–274), which is
   **platform-split in its implementation**: Windows tests the reparse-point attribute
   (paths.rs:264–269, covers symlink *and* junction), non-Windows `is_symlink()`
   (paths.rs:270–273). The doc comment explains why `is_symlink()` is wrong on Windows.
2. The dangling-link test uses `crate::test_support::dir_link` (test_support.rs:29–46),
   which is cfg-split (unix symlink / Windows `symlink_dir` falling back to `mklink /J`
   junction, :80–106) and **returns `false` instead of skipping silently** — the test at
   skills.rs:885–888 asserts loudly if the platform refused, and :902–907 pins the platform
   fact the branch depends on (`read_dir` on a dangling link → `NotFound`). This test
   therefore really executes on Windows via junctions; it is not a silent skip.
3. Platform-specific *path conventions*: the home directory is `$HOME` → `$USERPROFILE`
   (epsilon-llm/src/auth.rs:238–244, read in exactly one place; guarded by the
   source-scanning test `the_home_directory_is_computed_in_exactly_one_place`,
   epsilon-core/src/config.rs:1309–1339). `user_roots` (skills.rs:55–73) resolves through
   it, so the `.claude/.codex/.gemini/.agents` roots land under `C:\Users\<user>` correctly.

**Blind spots a Windows-only suite cannot see:**

- **G1 — the user-tier root table (`user_roots`, skills.rs:55–73) has no test at all.** No test
  asserts its five roots, its order (`.config/epsilon` first so a deliberate Epsilon config
  outranks a directory filled for another tool, :58–61), or that `.agents/skills` is last
  (wins-the-tier) inside the user tier. The one `all_roots`-level test (:1085) only asserts
  "nothing under the workspace is scanned when untrusted" — and since `home_dir()` returns
  `None` when neither `$HOME` nor `$USERPROFILE` resolves, `all_roots` can return a list
  with no user roots and every existing assertion still passes. Worst direction of the gap:
  a root deleted from `user_roots` (or reordered past the alias) breaks global discovery
  for Claude Code / Codex / Gemini users with no failing test. Mirror-image of the
  project-tier test at :952/:1202, which does exist.
- **G2 — the case-insensitive canonicalisation claim in `root_key` (skills.rs:113–118) is
  untested.** The comment motivates dedupe with "`/c/Users/hp` versus `C:\Users\Hp`", but
  every dedupe test uses `.` / `..` spellings (:1114, :1151, :1172) — never a
  case-differing spelling. The mechanism does hold for that case on Windows (nearest-existing
  ancestor canonicalises to the on-disk case), but the *suffix* is re-attached verbatim
  (skills.rs:138–145), so two spellings differing in the case of a **non-existent**
  component do **not** collapse — untested either way, and the comment asserts more than
  any test pins.
- Non-blind, for the record: `root_key`/`dedupe_roots` (skills.rs:119–169) and
  `discover_with_notices` (skills.rs:301–405) are cfg-free std-only code; path joining via
  `Path::join` is platform-correct; nothing in the discovery path touches `\` vs `/`
  literals.

## The named untested clauses and the smallest test that closes each

1. **"Discovery across commonly used *global* directories" (user tier).** Smallest test: a
   `user_roots` analogue of `the_project_roots_are_the_conventions_they_claim_to_be` —
   assert the returned list contains `~/.config/epsilon/skills`, `~/.claude/skills`,
   `~/.codex/skills`, `~/.gemini/skills`, `~/.agents/skills`, that `.config/epsilon` is
   first and `.agents` last, when a home directory resolves (the process env is the only
   lever; set `HOME`/`USERPROFILE` for the duration and restore).
2. **Precedence inside the user tier + user-vs-project ordering driven through the real
   `user_roots()`.** Smallest test: write `commit` into a seeded fake-home `.claude/skills`
   and into `.agents/skills`, run `discover_with_notices(&all_roots(home_as_workspace,
   true))`, assert the `.agents` body wins — the :1055 test currently hand-builds
   `vec![user.path()]` instead of calling `user_roots()`, so the wiring, not just the table,
   is unpinned for the user tier.
3. **The case-differing-spelling dedupe (G2).** Smallest test: create one real temp dir,
   call `dedupe_roots(vec![p, p_with_different_case_ancestor])` and assert length 1. (If it
   fails, the honest fix is to narrow the comment at :113–118 or lowercase on Windows —
   the test would decide which.)

---

# HALF 2 — criterion 6: usage accounting, targeted re-verification

## (a) Do the recorded STATUS.md claims hold at HEAD?

STATUS.md:3982–4014 ("Usage and cost accounting — audited, criterion 6"). Verdict per claim:

| STATUS.md claim | Verdict at HEAD | Evidence |
|---|---|---|
| "usage_arithmetic.rs holds **eleven** tests" | **WRONG — it holds nine** | `grep -c '#\[test\]'` = 9; names at :57, :71, :86, :108, :126, :138, :158, :177, :203. File is byte-identical to M194 (`72297e9`), where it also had nine; M140 (`9b97810`) had six. The "eleven" count was wrong when written. Substantive claims all hold (below). |
| "cached and cache-write tokens are inside `input_tokens` and must be subtracted" (citing pi `openai-completions.ts:1534`, `openai-responses-shared.ts:567`) | **HOLDS** | `seed_usage_compat` mod.rs:310–363: `input = prompt − cache_read − cache_write` with per-term `saturating_sub` (:351–353), matching pi `openai-completions.ts:1538`. Tests: usage_arithmetic.rs:108–121, 126–134, 138–149; saturation test :158–169 (u64 wrap guard); Responses `input_tokens_details` forwarding test :177–198 with the translator at openai_responses.rs:326–338 |
| "integer micro-USD pricing, four buckets, divide once" | **HOLDS** | Rationale at contracts/message.rs:123–127; `breakdown` cost.rs:124–140 (four `u64` scaled products); `total()` cost.rs:86–88 sums then divides once; `breakdown_per_model` cost.rs:173–196 sums scaled per model with a single final division (:169–172) |
| "per-model breakdown (`breakdown_per_model`, M141)" | **HOLDS** | cost.rs:173–196 exists; fed by `record_usage` (state.rs:1130–1171) and `run_.rs:1436–1441` (bills to `message.model_id`, so a post-`/model`-switch response prices to the right model); exercised in cost.rs tests :244–356 and TUI tests state.rs:2559–2628, 4032–4061 |
| "subagent usage folded (M229/A11)" | **HOLDS** | run_.rs:505–529: tool results carrying `usage` combine into the turn total (**:517–518**) and per-model map when the tool names its model (**:523–527**). Integration tests exist after all: `billed_tool_usage.rs:112` (in turn total), `:145` (unattributed bill still counts), `:189` (compaction bill in session totals) |
| "retry cannot double-count — code read, not executed test" | **HOLDS (with a sharper truth)** | Usage is summed on exactly one line, run_.rs:1431, keyed to `StreamEvent::Done`. The retry executor (`retry_stream.rs`) *discards* a retried attempt's terminal error-Done (pre-stream: :114–120; live-but-silent: :200–224) — it is never forwarded downstream, so it never reaches :1431; error Dones carry `Usage::default()` (stream_fn.rs:147–161). Precise nuance: STATUS's "one that did [reach Done] is never retried" is loose — a Done with `stop_reason == Error` **is** retried when nothing visible was emitted (test `a_live_failure_recovered_on_the_next_attempt_streams_normally`, retry_stream.rs:490–544). What holds is the accounting: no usage from any retried attempt can reach the accumulator. Still not covered by a test that counts usage across a retry — STATUS's "no failing-on-purpose control" remains accurate |
| Residual: "reasoning tokens with no site found" | **STILL TRUE, and now concrete** | see (c) |

Stale documentation, worth one line each: STATUS's line refs `epsilon-core/src/lib.rs:2553`
(subagent fold) / `:3558` / `:3124` point at a 417-line file — the composition moved to
`epsilon-core/src/run_.rs` (real sites: fold :517, sum :1431) and the terminal/retry logic to
`epsilon-llm/src/retry_stream.rs`.

## (b) One representative raw response walked end-to-end, two provider shapes

**Shape 1 — openai-completions-style** (also DeepSeek / Groq / xAI / OpenRouter / Copilot /
Gemini-via-compat keys; Responses API after translation).

Raw final chunk (`stream_options.include_usage`):
`{"usage":{"prompt_tokens":8000,"completion_tokens":500,"prompt_tokens_details":{"cached_tokens":6000,"cache_write_tokens":0}}}`
→ `openai_compat.rs:233–236` (`st.seed_usage_compat`) → `mod.rs:310–363`:
`cache_read = /prompt_tokens_details/cached_tokens` = 6000 (:316–318; fallbacks
`prompt_cache_hit_tokens` for DeepSeek :319–323, top-level `cached_tokens` for Kimi :324);
`cache_write = /prompt_tokens_details/cache_write_tokens` = 0 (:326–329);
`prompt = prompt_tokens` = 8000 (:330); `output = completion_tokens` = 500 (:354);
`input = 8000 − 6000 − 0` = **2000** (:351–353); `total_tokens` = provider's `total_tokens`
if present else **0** (:357 — benign: gauges use `input + cache_read + cache_write`,
state.rs:1654/1822, never `total_tokens`); `cache_reported = true` by key presence (:334–349).
Buckets exactly as `usage_arithmetic.rs:108–121` claims (8000 = 2000 + 6000, no double count).
Stored to `usage_tail` only when `input + output > 0` (:360–362); `finish()` (mod.rs:787–792)
takes seed-or-tail — for this wire `finalize_usage_from_tail()` is `false` (openai_compat.rs:252–254).

**Shape 2 — anthropic-style** (and Bedrock, which follows the same non-subtracting shape).

Raw stream: `message_start` with
`{"message":{"usage":{"input_tokens":8000,"output_tokens":1,"cache_read_input_tokens":6000,"cache_creation_input_tokens":0}}}`
then `message_delta` with `{"usage":{"output_tokens":500}}`.
→ `anthropic.rs:185–189` (`seed_usage`) → `mod.rs:269–285`: `input_tokens`→`input` = 8000 **with
no subtraction** — Anthropic's `input_tokens` excludes cache reads, unlike the compat shape;
`cache_read_input_tokens`→`cache_read` = 6000; `cache_creation_input_tokens`→`cache_write`;
`total_tokens` recomputed from components (:276); `cache_reported` = presence of either cache
key (:280–281). → `message_delta` (`anthropic.rs:243–247`) → `update_usage_tail`
(`mod.rs:377–395`): present fields overwrite, absent keep the seed (pi parity,
anthropic-messages.ts:835–842) → final `{input 8000, output 500, cache_read 6000,
cache_write 0}`. `finish()` prefers the tail for this wire (`finalize_usage_from_tail() = true`,
anthropic.rs:293–296; merge at mod.rs:787–791). Anthropic input+cache_read = 14000 is *not*
expected to equal a `prompt_tokens` figure — the compat "buckets add back to the billed prompt"
test (usage_arithmetic.rs:117–120) is a compat-shape property only, and it holds there.

Sibling parsers verified for the same rules: Bedrock `seed_usage_bedrock` (mod.rs:505–531,
`cacheRead/WriteInputTokens`, `cache_reported` per K1 at :522–523, total fallback :524–527) and
Mistral (mod.rs:627–672, eight cached spellings clamped `min(prompt)` :649, `cache_reported`
from pointer presence :662–664). `Usage::combine` (contracts/message.rs:181–194) is
component-wise addition with `cache_reported` OR-ed — order-independent (test
usage_arithmetic.rs:86–105).

## (c) Dropped / mis-mapped provider categories — the reasoning-token residual, checked

`Usage` (contracts/message.rs:128–176) has **no reasoning bucket at all** (input / output /
cache_read / cache_write / total_tokens / cache_reported). Reasoning-token reporting exists on
three wires **today**, and pi maps all three:

1. **Gemini / Google — the real under-count.** `usageMetadata.thoughtsTokenCount` is reported
   by Gemini 2.5+ thinking models. pi adds it to output *and* itemises it
   (`google-generative-ai.ts:237` output = `candidatesTokenCount + thoughtsTokenCount`,
   `:240` reasoning = `thoughtsTokenCount`; same in `google-vertex.ts:245–248`). Epsilon's
   compat path maps only `candidatesTokenCount` → output (mod.rs:354; gemini reaches it at
   google.rs:223–226) and **has no site for `thoughtsTokenCount`** (grep: zero occurrences in
   `crates/`). Under pi's documented separation, Gemini thinking tokens are missing from
   `output` — under-reporting both the token gauge and the output-rate cost. This is the
   audit's named residual ("reasoning tokens with no site found") with the site now actually
   found: the field exists on the wire and the reference maps it; Epsilon does not.
2. **OpenAI completions family (incl. OpenRouter / DeepSeek / Groq / xAI) — itemisation only,
   no under-count.** `completion_tokens_details.reasoning_tokens` is read by pi
   (`openai-completions.ts:1546`) but never by Epsilon (no occurrence in `crates/`). Harmless
   to totals because OpenAI includes reasoning tokens inside `completion_tokens` (pi's own
   comment at :1539), which Epsilon maps to `output`; only the itemised split is lost.
3. **OpenAI Responses — itemisation only, no under-count.**
   `usage.output_tokens_details.reasoning_tokens` (pi `openai-responses-shared.ts:572`) is
   dropped by `responses_usage` (openai_responses.rs:326–338), which forwards
   `input_tokens_details` but not `output_tokens_details`.
4. **Anthropic — itemisation only, no under-count.**
   `usage.output_tokens_details.thinking_tokens` (pi `anthropic-messages.ts:850–854`) is not
   read by `seed_usage`/`update_usage_tail`; Anthropic bills thinking tokens inside
   `output_tokens`, which Epsilon maps, so totals stay correct.

Also checked and clean: no provider today maps into the wrong bucket — cache-read vs
cache-write are separate everywhere, knows the four shapes' different `input` semantics
(compat subtracts, Anthropic/Bedrock/Mistral do not), and `cache_reported` (K1) is set by key
presence in all four parsers (mod.rs:280–281, 334–349, 522–523, 662–664).

**Smallest test that would close the residual:** drive a Gemini-shaped `usageMetadata`
(`promptTokenCount` 8000, `cachedContentTokenCount` 6000, `candidatesTokenCount` 400,
`thoughtsTokenCount` 300) through `compat_usage_from_json` and assert `output == 700`
(pi's rule) — RED today at 400.

---

## Summary

- **Criterion 4** is well covered on the project tier, conflicts, missing directories and
  dedupe (24 tests, all cfg-free logic, both platform delegates tested for real on Windows).
  The untested clauses are exactly: the **user-tier (global) root table** `user_roots()`
  and its ordering ([G1]), the user-tier precedence wiring through the real function, and the
  **case-insensitive canonicalisation** claim in `root_key`'s doc comment ([G2]).
- **Criterion 6**'s substance holds at HEAD (pipeline, pricing, per-model, subagent fold with
  integration tests, retry non-double-count), but the recorded audit text has drifted in three
  places: "**eleven** tests" is **nine**, the `epsilon-core/src/lib.rs` line refs are stale
  (composition lives in `run_.rs`), and the reasoning-token residual is now sharper than "no
  site found" — Gemini's `thoughtsTokenCount` is a live, mapped-in-pi, unmapped-here
  **under-count** of output.
