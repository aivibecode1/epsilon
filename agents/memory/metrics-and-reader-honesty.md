# Metrics and session-reader honesty (M106–M108)

**Status: 2026-09-21, in the tree, battery `m106a` running.** Three slices that came out
of one reading pass over the TUI's visibility numbers and the store's repair rules.

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

- Battery `m106a` → copy `target/gate-m106a.log` to `audit/evidence/` before cleanup.
- `epsilon-tui` 220 → 217 (−8 footer tests, +4 git, +1 metrics); `epsilon-session`
  integration 29 → 30.
- Milestone docs: `docs/milestones/M106-cache-hit-accuracy.md`,
  `M107-dead-footer.md`, `M108-unknown-final-line.md`.

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
