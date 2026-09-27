# M168 — `/tree` was a copy of `/timeline`, and closing the gap found a crash

## What this did

M167 measured that 18 of 30 user-typable slash commands are never executed by a single TUI
test, and named `/tree` (97 lines) as the largest. This milestone closes that one — and the
reason the gap existed is the most interesting part.

## `/tree` and `/timeline` were the same 96 lines, twice

Both arms were 96/97 lines. After normalising the notice label and the helper suffixes
(`snip2`/`kind_tag2` vs `snip`/`kind_tag`), they were **byte-identical except the variant
name**. Two copies of 70 lines of nested helper functions, differing in one string.

The first comparison I ran reported all 96 lines differing, which was an artefact: I zipped
two lists of different length and compared every line against the wrong partner. The
corrected comparison found exactly one differing line. Worth stating because a plausible
"they're totally different" reading would have justified leaving the duplication in place.

They are now one `open_timeline_picker(agent, label)`, with `timeline_snip` and
`timeline_kind_tag` lifted to module level where they can be tested. `app/commands.rs` is
1,387 → 1,324 lines.

## The real finding: nine production call sites could panic the TUI

The first test I wrote failed with

```
panicked at crates\epsilon-core\src\lib.rs:989:22:
no session store attached
```

Not a test problem. `Agent::store_handle_for_tests()` is `#[doc(hidden)]` and panics when
no store is attached — and `Agent::session_store()` exists beside it precisely for that,
documented as: *"an agent with no session file (a plain `--tui` run, a `task` child before
its store exists) is a normal state, and a command like `/session` has to describe it rather
than crash."*

**The TUI called the panicking one in nine places.** `/tree`, `/timeline`, `/fork`,
`/clone`, `/name`, `/export` (twice — HTML and JSON), `/share` via export, the
Enter-to-follow-up key in `keys.rs`, `switch_session_file`, and `run_branch_command` four
times. So: a user typing `/tree` in a plain `--tui` run took the TUI down.

There was already a comment at `commands.rs:333` — on the *one* correct site — saying
"`store_handle_for_tests` panics on it, so the command asks for the Option instead". Someone
had hit this, fixed one call, and left the other nine. That is the strongest argument for
fixing the class rather than the reported site, and it is why the comment now names the rule
instead of restating the accident.

All nine now use `session_store()` and describe the missing store: `no session store
attached` for the commands, `"no session store attached"` as an `Err` for the two exports,
`None` where the caller already handled it. `agent.store_handle_for_tests()` no longer
appears in production code anywhere in the TUI.

## The control

Restoring `open_timeline_picker` to the panicking accessor:

```
369 passed, 1 failed
panicked at crates\epsilon-core\src\lib.rs:989:22: no session store attached
  a_command_needing_the_session_store_says_so_instead_of_crashing
```

The test fails with the *panic itself*, which is the whole point: before this, the crash was
real and the suite was silent about it.

## Six new tests

| test | what it holds down |
| --- | --- |
| `tree_and_timeline_open_one_picker_and_say_which_one_ran` | both commands reach one picker, with the right kind tags and snippets |
| `an_empty_session_says_which_picker_was_asked_for` | the label is passed through, not hard-coded |
| `a_long_entry_is_cut_and_marked_rather_than_pushed_off_screen` | 100 chars + `…` |
| `a_whitespace_only_entry_reads_as_empty_rather_than_blank` | `(empty)`, not a blank row |
| `each_entry_kind_gets_the_tag_the_picker_shows` | `user` / `compact` / `turn`, so a kind added later falls to `other` rather than lying |
| `a_command_needing_the_session_store_says_so_instead_of_crashing` | the regression |

The crash test asserts `agent.session_store().is_none()` first, so it cannot pass vacuously
if `test_app()` ever starts attaching a store.

## Three of my own mistakes, recorded

- `SessionStore::open_in(path, cwd)` takes a **file** path. I passed a directory, got
  `Access is denied`, and initially blamed the temp directory and the `TMP`/`TEMP` vars.
- Index arithmetic across two insertions silently overwrote a function signature. I
  restored from `HEAD` twice, and the second restore also reverted the merge, because
  `HEAD` is M167. The fix that actually worked was to stop reasoning about indices and
  re-find every anchor by search.
- `"Command::Tree =>"` also matches the *dispatcher's* one-line arm, so my replacement
  swallowed an entire `match` block. Requiring the exact `"... => {"` made it unique.

Each of these cost more time than the milestone's actual content. The generalisation is
already written down in M166 — anchor on something unique, then assert the invariant before
editing — and I broke it anyway, twice.

## Verification

| gate | result |
| --- | --- |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets --features real -- -D warnings` | exit 0 |
| `cargo test --workspace --features real` | **62 binaries, 1,295 passed, 0 failed** |
| `cargo run -q -p epsilon-cli -- --demo` | exit 0 |

`epsilon-tui` 371, up from 365.

## Limits

- **16 of the 18 uncovered commands are still uncovered.** This closed the two largest
  (`/tree`, `/timeline`) and the crash that hid behind them.
- **The crash fix is unverified in a real console.** It is proven by a test that runs the
  commands; nobody has typed `/tree` into a plain `--tui` run and watched it not die.
- **`store_handle_for_tests` remains available and still panics.** It is `#[doc(hidden)]`
  and used by core integration tests, which is legitimate. Nothing now prevents the next
  person from calling it from production code — only a comment does.
- **The other arms of `run_transfer_command` (308) and `run_view_command` (289) are still
  monoliths**, and `/export`, `/import`, `/share` are untested — so their interaction with
  the new `Err("no session store attached")` path is unexercised.
- **No real-console run**, because no frame changed.
