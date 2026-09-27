# M166 — the 929-line `impl App`, in three

## What this did

`impl App #1` held 43 methods in 929 lines. Thirty of them — 584 lines — moved into three
files:

| new file | what | methods | lines |
| --- | --- | --- | --- |
| `app/session.rs` | construction and which session is in front of the user | 6 | 224 |
| `app/editing.rs` | the composer line: characters, cursor, kill, yank, undo, redo, history | 19 | 228 |
| `app/completion.rs` | slash-command and `@file` completion | 5 | 132 |

`app.rs` is now **1,313** lines, down from 4,526 at the start of this work in M161. All
three blocks are byte-identical to the parent commit apart from the `pub(super)` markers.

## How the boundaries were chosen, and why that is the whole point of this milestone

M164 abandoned a split because its boundaries came from a regex over method heads that
silently found 18 of 22 methods — four signatures are wrapped across lines by rustfmt. This
time each boundary is derived from three *named* anchor methods: `insert_char`,
`sync_slash_state`, `submit_editor`, each located by exact match and then walked back over
its own doc comment. A miss is impossible when the boundary **is** the anchor. The first
five attempts at these boundaries were all off by one or over-reached into a neighbouring
doc comment, and each was caught by an assertion on the tail line — the slice must end on
`    }` — before anything was written.

The rule is now: *state the invariant you are about to rely on, then assert it before the
edit rather than after.*

## The three groups are layers, not lists

- `editing.rs` is a **layer**: every method composes the ones above it (`backspace` uses
  `byte_cursor_at`, `yank` uses `stash_draft`), so the order in the file carries meaning.
  Its `*_tracked` wrappers are the seam worth naming — the key handler calls those, and each
  applies the plain primitive *and then* re-derives the slash-popup state, so no caller can
  forget the second half.
- `completion.rs` exists for one invariant: **the popup state is derived from the editor
  buffer, never remembered.** `sync_slash_state` and `sync_at_state` recompute open and
  index from `self.input` rather than patching it, which is why any editing method can call
  them freely and a stale popup cannot survive a keypress.
- `session.rs` is the identity question — which session is this — as against its contents.

## The control: three mutants, one build, and every file proved live

For a move, "the tests still pass" proves almost nothing — the obvious failure is code that
*compiles, is byte-identical, and is never called*. So one mutant was installed per new
file, in a single build:

| file | mutant | caught by |
| --- | --- | --- |
| `session.rs` | `switch_session_file` returns early | `sessions_command_browses_and_enter_switches_the_session` |
| `editing.rs` | `undo` returns `false` | `ctrl_z_undoes_backspace_and_kills`, `ctrl_dot_redoes_what_ctrl_z_undid` |
| `completion.rs` | `apply_slash_selection` returns early | `tab_completes_highlighted_candidate`, `enter_applies_highlighted_suggestion_then_runs_it`, `enter_with_popup_on_exact_text_still_edits_before_running` |

**359 passed, 6 failed** — one per file at minimum, and every failure names the behaviour
rather than the file. All three new files are genuinely exercised from their new homes, and
the visibility markers are correct, since every one of these is called across a module
boundary.

Getting the mutants to compile took two attempts: the first used a bare `return;` in
functions returning `Result<usize, String>` and `bool`. That is worth one sentence —
*a control that does not compile is not a control*, and the compiler stopping you is the
cheapest possible outcome.

## Verification

| gate | result |
| --- | --- |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets --features real -- -D warnings` | exit 0 |
| `cargo test --workspace --features real` | **62 binaries, 1,289 passed, 0 failed** |
| `cargo run -q -p epsilon-cli -- --demo` | exit 0, 6 messages, `tokens in/out: 300/150` |

`epsilon-tui` is 365 — unchanged, as it must be. No test added, none removed, no behaviour
changed.

## Limits — what this milestone does *not* prove

- **Thirteen methods and ~345 lines remain in `impl App #1`**, including `over_ui` at 206
  lines (the welcome screen) and `rows_below` at 58. `over_ui` is the largest single method
  left in the TUI and is a genuine candidate for its own file.
- **The mutants prove the code is called, not that it is correct.** They rule out dead code
  and wrong visibility; they say nothing about whether `switch_session_file` picks the right
  file in an edge case.
- **`APP_MODULE_SOURCE` is a hand-maintained list**, and it was extended to all nine files
  under `app/` in this milestone rather than left to rot — but it is still a list, and the
  next split will forget it. The rule written into the constant's own doc comment is that a
  new file under `app/` is added there in the same change that creates it. A macro or a
  build script that enumerates the directory would remove the obligation entirely; that is
  the better fix and has not been done.
- **No real-console run**, because no frame or keypress changed.
