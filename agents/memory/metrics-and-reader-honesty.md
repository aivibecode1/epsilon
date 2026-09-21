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

## Open, recorded not fixed

- **G18 — cost prices cached tokens at zero.** `tokens_in` excludes cached tokens by the
  providers' split, and cache reads/writes are not priced at all. Needs a `ModelInfo`
  field, a column in `scripts/regen_catalog.py` (models.dev publishes the prices) and the
  regenerated `catalog_data.rs`. Until then the estimate is low on any cached session.
  Recorded in `audit/11`.
- **G17's class is not closed**: `pub` remains a hole in the dead-code gate, so anything
  else superseded by a redesign and left `pub` is still invisible.
- Refusing an unknown final line means an older build cannot open a newer session at all —
  intended (no data loss) but there is no "show me the rest anyway" path.
