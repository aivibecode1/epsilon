# M164 — the read side of `app.rs`, and a test that named a file

## What this did

`impl App #2` — 22 methods, 531 lines — moved verbatim from `app.rs` into
`crates/epsilon-tui/src/app/view.rs`. `app.rs` is now **2,079** lines, down from 4,526 when
M161 started this work.

The methods are the read side of the TUI: `run_state`, `metrics`, `cost_summary`, `hints`,
`mirror_context_size` (what the frame reports), and the rest — which card is expanded, which
overlay owns the keyboard, how tall the transcript window is, whether it is empty, where
`/search` is and which match it is on, and how much of a streaming tool's output still fits.

## Why one file and not three

The first plan was to split it again — view state, status/metrics, search — into three
modules. That plan was abandoned mid-execution, and the reason is worth recording: the
boundaries between the three were derived by **searching for method heads**, and the search
silently found 18 of 22 methods because four signatures are wrapped across lines by
`rustfmt`. Every boundary derived from that list would have been wrong, and three wrong
boundaries produce a file that compiles and behaves subtly differently.

One contiguous range, whose two ends are structurally unambiguous (`impl App {` at 2002 and
its closing brace), is worth more than three tidier ones derived from a detector that was
wrong and did not know it. The next split can be made by reading the methods properly.

## The visibility was not chosen, it was collected

Rather than guessing which of the 22 methods the parent still calls, the first build was
allowed to fail and the compiler's `E0624 method ... is private` diagnostics were harvested:
eleven names. A second round, after the test build, added `transcript_window` and
`measure_transcript`, which only `app/composer_viewport_tests.rs` calls — invisible to a
plain `cargo build`. Thirteen `pub(super)` markers, every one of them requested by a
compiler that had a caller in front of it. A file's contents do not get to decide their own
visibility.

## The test that named a file

`the_card_open_rule_is_borrowed_and_not_restated` reads the *source* of `card_is_open` and
fails if the open/closed rule is written out by hand rather than delegated to the renderer.
It did so with `include_str!("../app.rs")` — and moving the method made it fail with:

> `card_is_open` is gone; the draw path and the key path now decide this independently
> and nothing in the tree says the two agree

which is a **misleading message**: nothing about the agreement had changed, and the test's
own claim — that the rule is stated in one place — was still true. The test was reporting a
file move as a lost invariant.

It now reads a `concat!` of the whole `app` module, so the next move will not break it
either:

```rust
const APP_MODULE_SOURCE: &str = concat!(
    include_str!("../app.rs"),
    include_str!("../app/commands.rs"),
    include_str!("../app/panels.rs"),
    include_str!("../app/view.rs"),
);
```

**The control matters more than the fix here.** A source-grep test is exactly the kind that
can be made vacuous by the change that repairs it, so the control was to *restate* the rule
in `card_is_open` — the `find(..).map(..).unwrap_or(..)` chain the test exists to forbid —
and confirm it still bites from its new file. It did: **362 passed, 2 failed**, with
`the_card_open_rule_is_borrowed_and_not_restated` among them. Repairing a test's input path
is only safe if you show the test can still fail.

## Byte-identity

All 22 methods are present, in the same order, and the block is byte-identical to
`app.rs` at the parent commit apart from the 13 `pub(super)` markers and **one rustfmt
wrap**: adding `pub(super)` to a signature pushed it past 100 columns, so rustfmt wrapped it
and added a trailing comma. A whole-block comparison reports exactly two differences — a
space and a comma. This is the same artifact M161 recorded for `draw_transcript`, and it is
worth knowing by name now: it is what "I only ran fmt" looks like underneath.

## Verification

| gate | result |
| --- | --- |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo test --workspace --features real` | **62 binaries, 1,288 passed, 0 failed** |
| `cargo clippy --workspace --all-targets --features real -- -D warnings` | exit 0 |
| `cargo run -q -p epsilon-cli -- --demo` | exit 0, 6 messages, `tokens in/out: 300/150` |

`epsilon-tui` is 364 passed. One test was *modified* (the source path it reads) and none
was added or removed.

## Limits — what this milestone does *not* prove

- **`view.rs` is 531 lines and mixes two things**: what the frame reports (`metrics`,
  `cost_summary`, `hints`) and where the user is (`overlay_open`, `measure_transcript`,
  the search methods). The three-way split is still the right end state; it needs a careful
  read of all 22 methods rather than a boundary detector.
- **`impl App #1` in `app.rs` is still 929 lines** and is now the single largest item in the
  crate. It mixes construction, composer editing, slash/`@` completion, submission, the
  OAuth bridge, model application and live-cache refresh.
- **Two more self-contained blocks are still in `app.rs`**: the editor bridge (five free
  functions, 91–206) and the OAuth UI types (1664–1738). Both are clean seams that have not
  been taken.
- **No real-console run**, because no frame or keypress changed.
