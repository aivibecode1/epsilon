# Metrics and reader honesty (M106–M109)

**Status: 2026-09-21.** M106–M108 landed with battery `m106a` (1,021 tests); M109 with
`m109a` (1,025 — four more, exactly the new `at_files` tests). Four slices that came out of
one reading pass over the TUI's visibility numbers, the store's repair rules and the `@`
completion list.

## M106 — the cache gauge divided a session total by one prompt

`Metrics::cache_hit_rate` used the **session-total** `cache_read` over the **latest**
prompt size, while its own doc comment claimed both came from the same call. `UiState`
accumulates `cache_read`; `context_used` is the latest call's prompt. So the ratio
inflated every turn and the `.min(prompt)` clamp pinned it at `cache 100%`.

The test named `cache_rate_is_about_the_latest_prompt_not_the_session` passed — it moved
the denominator and held the numerator fixed, a model no session produces. **A test can
be named for a property and still certify its absence.**

Fix: `UiState.latest_cache_read` (set in both usage arms, beside `latest_prompt_tokens`),
`Metrics.cache_read_last` as the numerator, `cache_read_session`/`cache_write_session`
renamed to say what they are. New test `the_cache_numerator_never_borrows_from_earlier_calls`
plus a rewritten one that fails against the session-total model.

## M107 — the dead second footer

`crates/epsilon-tui/src/footer.rs` was a whole superseded footer with 8 tests; only
`git_branch` had a caller. Invisible to `clippy -D warnings` because `pub` in a `pub mod`
of a library crate is exempt from `dead_code` by construction. It carried a second
`format_tokens` with different rounding and a second copy of the G16 formula.

Deleted; the survivor is `crates/epsilon-tui/src/git.rs` (`git::branch`, 4 tests), and the
linked-worktree pointer that used to render as the branch name `gitdir:` now reports no
branch instead.

## M108 — F08: a complete unknown final line is refused, not truncated

`is_whole_json(fragment)` is now the discriminator at every site that used to ask only "is
it the last line?". A fragment (unparseable) is still repaired as crash debris; a complete
line in an unknown shape is refused with `LineDiagnosis::UnknownLineShape` /
`UnknownEntryType` and the file stays byte-identical. This work was **found in a leftover
git stash** from an earlier session (`stash@{0}`, 2026-09-20) that also contained an
earlier version of M105 — the stash is now redundant and can be dropped once this lands.

Negative control: reverted the three gates in place, the new test failed with
`a complete unknown final line was accepted: {"id":"ent_x",…"future_thing"}`; restored
byte-identical (sha256 `108B5BC17AB12355…`).

**Trap worth keeping:** restoring a file by `Move-Item`/`mv` keeps the *backup's* mtime, so
cargo reuses the stale test binary and a green/red result can be exactly backwards. Touch
the file after restoring. Cost 10 minutes here.

## Evidence

- Batteries `m106a` (M106–M108) and `m109a` (M109), both copied to `audit/evidence/`.
- `epsilon-tui` 220 → 221 across the round (−8 dead footer, +4 git, +1 metrics, +4 at_files);
  `epsilon-session` integration 29 → 30; workspace 1,023 → 1,025.
- Milestone docs: `docs/milestones/M106-cache-hit-accuracy.md`,
  `M107-dead-footer.md`, `M108-unknown-final-line.md`, `M109-at-file-ignore.md`.

## M109 — the `@` list now shares the ignore answer (and the dead-logic find)

`App::at_file_rows` had its own walker and its own hardcoded skip list, so `@` offered
files the tools would refuse while `audit/11` claimed one shared answer. It now loads
`epsilon_core::ignore` (same `IgnoreSet`, same `skip_dir`) and keeps only its traversal,
which is budgeted for typing latency rather than for a tool call. The walk moved to
`crates/epsilon-tui/src/at_files.rs` with four tests.

**The find:** the tool walk's hidden-directory check reads
`name.starts_with('.') && !ignore.is_ignored(..)` and its comment promised a `.gitignore`
negation re-includes a hidden directory. It cannot. `is_ignored` walks ancestors and
returns **false** for `.hidden/` + `!.hidden/` — the same answer as for a directory no rule
mentions — so the clause can only differ from "always skip" when the next check skips the
entry anyway. Dead logic plus an unkeepable promise, in both walks; the tool *test* had
recorded the limitation honestly while the inline comment and `walk_files`'s doc comment
claimed the feature. Clause deleted, comments corrected, and the limitation pinned by
`a_hidden_directory_is_skipped_even_when_a_negation_names_it` so the day the escape hatch
becomes real, that test fails.

Lesson worth keeping: when a test of a documented behaviour fails, find out which of the
two is lying before changing either. Here the matcher's doc (real git semantics for
*matching*) had been over-generalized into a capability the API cannot express.

## M110 — the stall M109 shipped, and how it was caught

M109's own verification found it. `at_file_rows` ran `IgnoreSet::load` **per keystroke**,
and `load` searched gitignored trees instead of pruning with the rules it was finding — so
every character after `@` walked `/references/` (404 MB, gitignored). One- and two-character
queries drew a frame in time; `@argo`/`@Cargo` left the composer showing its *placeholder*,
meaning the app had not drawn since before the keys arrived. That is indistinguishable from
a hang, and it is the owner's original symptom class.

**How the wrong hypothesis was killed:** the first story was timing ("the moment I launch
it and start typing"), so the probe was run at waits of 3, 5, 8 and 12 s — identical
failures every time, while short queries always worked. The variable was how much work the
keystrokes queued. `scripts/console-probe.py` gained `--at SECONDS` to make that
separable, and it stays.

Two fixes, different jobs: `App::at_ignore` caches the rules with a **5 s TTL** (the branch
cache's shape, same reason), and `IgnoreSet::load` **prunes** directories the rules so far
already ignore — git's own rule, and the reason the fix belongs in the matcher: `grep`,
`find` and `read` all call `load`, so every tool call was paying it too.

Evidence: battery `m110a` (1,026 passed, +1 = `the_search_is_pruned_by_the_rules_it_finds`);
before/after probe runs in `audit/evidence/at-popup-{prefix,postfix}-*.json`.

**Rule to keep:** an interactive path that gets slower looks exactly like a deadlock. When
a frame stops updating, time the *keystroke*, not just the frame rate — and repeat a
suspected flake at several timings before believing it is flaky.

## M111 — the cost estimate, and the buckets it was not charging

The header added `tokens_in × input + tokens_out × output`. There are **four** disjoint
buckets: `usage.input` excludes cached tokens (the adapters apply the providers' split at
the wire boundary), cache writes are a third, output a fourth. Cache reads and writes were
charged at **zero** — a cache-heavy session's cost was low by roughly the cached share
times the input price.

Fix: `ModelInfo` gains `cache_read_cost_micro_per_mtok` /
`cache_write_cost_micro_per_mtok`; `scripts/regen_catalog.py` reads models.dev's
`cost.cache_read`/`cost.cache_write`; the table was regenerated (359/557 rows publish a
read price, 114 a write price). The arithmetic moved to
`crates/epsilon-tui/src/ui/cost.rs` with four tests, because a claim about money needs a
test and the old expression — sitting in the draw path — had none.

`0` = *not published*, which is not free, so `Metrics::cost_is_floor` marks the case and
the header prints `$0.42+`.

**Refresh mechanism, worth remembering:** Python's `urllib` cannot reach models.dev here
(MSYS Python has no CA roots → `CERTIFICATE_VERIFY_FAILED`); PowerShell can. So
`Invoke-WebRequest https://models.dev/api.json -OutFile $env:TEMP\models-dev-api.json`,
then `python scripts/regen_catalog.py <path>`. A regeneration also pulls unrelated upstream
drift (this round: 547 → 557 rows, 15 in, 5 out, prices moved) — disclose it in the commit
rather than smuggling it, with `catalog_is_current_generation` as the coherence check.

**Still open:** the header's numbers have no legend on screen. A user sees
`?/164k · ↑0 ↓0 · cache — · $0+ · DeepSeek V3.1` and nothing explains the `?`, the `—` or
the `+`. `/help` (`app.rs:1212-1230`) is the natural home for two lines; queued as its own
small slice so it gets its own battery.

## Open, recorded not fixed

- **G18 — closed in M111.** Kept here only as the pointer: the cost estimate now prices all
  four buckets, and reports a floor when the registry cannot price cache traffic.
- **G17's class is not closed**: `pub` remains a hole in the dead-code gate, so anything
  else superseded by a redesign and left `pub` is still invisible.
- Refusing an unknown final line means an older build cannot open a newer session at all —
  intended (no data loss) but there is no "show me the rest anyway" path.
