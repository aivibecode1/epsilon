# M163 — the catch-all stops being a monolith

## What this did

M162 ended by naming its own next problem: `run_workspace_command` was 655 lines, and it
was the catch-all, which is correct for routing and wrong for cohesion — `/export` and
`/theme` shared nothing but being unmiscellaneous. This milestone splits it by **what each
command acts on**.

| executor | commands | lines |
| --- | --- | --- |
| `run_view_command` | `/help` `/skills` `/tasks` `/timeline` `/hotkeys` `/settings` `/changelog` | 274 |
| `run_transfer_command` | `/trust` `/export` `/import` `/share` `/copy` | 289 |
| `run_workspace_command` *(catch-all)* | `/clear` `/search` `/quit` `/theme` `/compact` `/unknown` `/editor` | 85 |

The dispatcher now has five routes. The catch-all is still a catch-all — a new `Command`
variant still lands where the compiler demands its arm — but it is now the *smallest* of the
three, and its doc comment says why it is allowed to be the vague one.

## Why "what it acts on" and not "what it looks like"

The first grouping attempt sorted by shape: big commands together, small ones together. That
produces a file where `/timeline` (96 lines, a navigable branch picker) sits beside
`/changelog` (42 lines, prints a file) and `/export` (63 lines, writes a directory) beside
`/trust` (78 lines, changes a security decision). Nothing in that grouping would help a
reader.

Sorting by subject instead puts `/trust` with `/export` and `/import` — all three are about
**what Epsilon is allowed to touch**, which is why `/trust` belongs with the file movers
rather than with `/settings`. That grouping is arguable and the doc comment says so; what
matters is that a reader can predict which function a command lives in.

## The arms are provably intact

All 19 re-homed arms were sliced by measured line range and compared against `HEAD`, each
searched inside the executor that now owns it. **19/19 byte-identical.** The
`app/commands.rs` arm census found 34 distinct arms, none missing, none duplicated.

The executor sizes were predicted before the edit (274 / 289 / 85, totalling 648 arm lines)
and came out exactly that, which is the check that catches a lost slice.

## Four mistakes, all mine, all caught before commit

1. **Mixed line endings in one splice.** Structural lines were joined with `\n` while arm
   text used `\r\n`, and the `Theme` arm came out missing its middle — the file still
   *compiled-looking* but `rustc` reported an unclosed delimiter. Fixed by normalising the
   file to LF, indexing there, and writing CRLF back.
2. **`arm()` closed over a list I had already mutated.** The first splice shifted the
   indices, so the second and third executors read the wrong lines and twelve arms
   vanished. The arm census caught it: `missing: [Changelog, Copy, Export, Help, …]`.
   Fixed by materialising all 19 arms *before* any mutation.
3. **The fallback arm outside the match.** `#[allow(unreachable_patterns)]` after the
   match's closing brace is a statement, not a pattern. Same trap as M162.
4. **Over-applying a tuple pattern.** My probe had already reported `Share` and `Copy` as
   unit variants; I added `(_)` to them anyway, and the compiler said so.

Every one of these was found by a machine — the compiler, or an invariant I had written —
rather than by reading the diff carefully. That is the argument for checking invariants
rather than trusting the eye.

## The routing tripwire, and its control

The control misrouted the seven view commands into the workspace executor:

```
test result: FAILED. 361 passed; 3 failed
panicked at app/commands.rs:717:17: Hotkeys is not a run_workspace_command case
```

Three tests failed, and the panic named the mistake exactly. This is the same tripwire
M162 added, and it is what makes a five-way dispatcher safe to maintain: routing is not a
comment in the source, it is an assertion that fires in every test build.

## Verification

| gate | result |
| --- | --- |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo test --workspace --features real` | **62 binaries, 1,288 passed, 0 failed** |
| `cargo clippy --workspace --all-targets --features real -- -D warnings` | exit 0 |
| `cargo run -q -p epsilon-cli -- --demo` | exit 0, 6 messages, `tokens in/out: 300/150` |

`epsilon-tui` is 364 passed — unchanged, as it must be. No test was added and no behaviour
changed.

## Limits — what this milestone does *not* prove

- **`run_transfer_command` is 289 lines and `run_view_command` is 274.** Both are under the
  size that made the original unreadable, and both could still be split by domain
  (`/timeline` alone is 96 lines). The rule applied here was "split until the boundaries are
  legible", not "split until every function is small".
- **The dispatcher is now five or-patterns deep and is itself the thing a new command must
  be added to.** It is 40 lines and easy to read, but the count of places a command's
  identity is written down has grown from one to two (enum + dispatcher). Worth watching; a
  per-group `const` of variant names would centralise it if it keeps growing.
- **No real-console run**, because no keypress or frame changed.
- **`app.rs` is still 2,611 lines** with state mutation, viewport measurement, the editor
  bridge and the OAuth UI all in one file. That is the next seam, and unlike this one it is
  separated by kind rather than by shape, so it needs the read-the-code-first pass again.
