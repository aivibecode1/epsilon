# Session-store integrity (M105)

**Status: closed, 2026-09-21.** Committed as `497093e` (with the milestone log and the
history row), after battery `m105d`: all five gates exit 0, **1,023 passed / 0 failed /
1 ignored across 59 test binaries**. The evidence is copied out of `target/` to
`audit/evidence/gate-m105d.log` (the passing run) and `gate-m105c.log` (the first
attempt, which failed only `fmt --check` and is kept because it is why the battery was
re-run). Milestone: `docs/milestones/M105-session-repair-integrity.md`.

Files: `crates/epsilon-session/src/store.rs` +
`crates/epsilon-session/tests/session_store.rs`.

## The two defects (both were committed, both reachable from the product)

1. **A repair could delete the session's identity.** `truncate_to_index` cut the file
   back to the last indexed entry's end, or to **zero** when no entry survived. A kill
   during a new session's *first* append leaves a header plus half a line — no indexed
   entry — so the repair emptied the file while `self.header` still answered `Some`.
   The next append then skipped the header it believed was already there, and the file
   reopened as a headerless v1 session: fresh ids, no cwd, and for a delegated child no
   parent link. Silent, and exactly in the path a user reaches after something already
   went wrong.
2. **`pop_last` desynchronised the read cache.** It popped the index but not the
   read-through entry cache, so the *next* append's sequence number was answered from
   the popped copy. `branch_entries` resolves the leaf id through those reads, so the
   active branch came back **empty** — as if the conversation had never happened.
   `pop_last` is not test scaffolding: `epsilon_core::pop_last_message` calls it to
   discard the message that overflowed the window before retrying.

## The fix

- `header_end` is recorded in **both** places a header can appear — `reindex` (a file
  that already had one) and `write_header` (a file the store stamp itself) — so it is a
  reliable property rather than a sometimes-value.
- `keep_end()` = end of the last indexed entry, else the header end, else 0. Both
  `truncate_to_index` and `pop_last` cut to it, so "drop what the index does not cover"
  keeps the header, which is content: id, cwd, parent link.
- `pop_last` truncates the cache to the index length, which repairs a cache that is
  either one short (after appends) or one long (the bug).
- `migrated_entries` → `cached_entries`, with a doc that matches the code. The old name
  and doc said "populated only when `migrated`"; line 600 populated it for **every**
  file, and reads always prefer it. The invariant is positional: `cached_entries[i]` is
  the entry at `index[i]`, never longer than the index; appending leaves it one short on
  purpose so a torn write still surfaces as a read error.

## Tests (the regression, both fail before the fix)

- `a_torn_first_entry_does_not_take_the_header_with_it` — hand-written header + partial
  line; asserts one line survives, the header id is the same one, and the reopened file
  is still a current-version child.
- `popping_the_last_entry_keeps_the_header_and_the_read_cache_in_step` — pop, append,
  then assert `read_all` and `branch_entries` show `["first", "third"]` and not the
  popped `"second"`; then pop to nothing and assert the header outlives its entries.
- New helper `entry_text` beside `view_texts`.

## Evidence

- Battery `m105d` → copied to `audit/evidence/gate-m105d.log` (and `gate-m105c.log`,
  the fmt-only failure that forced the re-run).
- `audit/evidence/console-probe-2026-09-21.md` + `console-probe-runs-2026-09-21.json`
  are the TUI half of the same round.
- The `epsilon-ext` frame-cap test from the same round is its own commit, `74ed230`.

## Reinstall

Owed and done this round: session repair runs on every resume, so the slice is
CLI-visible and the owner tests the installed binary.

## Open, recorded not fixed

- **F08's live half:** a complete final line with an unknown `kind.type` is still classed
  torn and truncated at open, rather than refused. Needs a typed tail diagnosis
  (`reindex -> bool` cannot express it). Design: `audit/evidence/g3-g4-checkpoint-restore-design.md`.
- **F07, confirmed and separate:** `epsilon-core`'s `persist` does `let _ = st.append(..)`
  and `pop_last_message` does `let _ = s.pop_last()`, so a failed durable write leaves
  memory and the model-view projection silently divergent.
- The store slice is CLI-visible (resume repairs a file), so the global `epsilon.exe`
  needs a reinstall once the battery is green.
