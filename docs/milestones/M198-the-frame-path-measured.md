# M198 — the frame path, measured

## What

A performance and honesty pass over the TUI's per-frame work, the LSP header
reader, the lint law, and the transcript's own typography. Everything here was
found by reading the code and by **rendering a frame and looking at it** — the
last of which turned out to matter more than expected.

## Why a frame was rendered

The audit that preceded this milestone had a gap it named itself: "A claim about
what a user sees is verified by measuring, not by reading." It had never been
applied to the transcript's *glyphs*.

So a scratch test drove the real `draw` through a `TestBackend` and printed the
screen. Two things were visible immediately, and neither is in the defect ledger:

```
 2|• • The layout module owns the spatial model.          <- notice
 3|• • It splits the frame into a header, a transcript…    <- notice
 4|• • and it decides what happens when the terminal is…  <- notice
```

`• • `. Every system notice rendered its gutter glyph **twice**. The same was
true of `▸ ▸` for a pending call, `✓ ✓` for a result and `✗ ✗` for a failure —
four of the seven row kinds, so nearly every row in the transcript. It is not a
subtle thing; it is the first thing a screenshot shows.

**Cause.** `ui::transcript::row_style` paints the role glyph in the gutter
(column 0), and `render_cache::row_marker` painted *the same glyph* again in the
body. The marker was introduced, per its own comment, so that "the row's own
marker repeats it in the body position so the distinction survives at a glance" —
but the gutter already carried the distinction, and the two glyphs were identical.

**Fix.** A body marker is emitted only when its glyph **differs** from the
gutter's. The user turn is the row that keeps one — its gutter is the cursor bar
`▎` and its marker is the prompt `❯`, two different characters carrying two
different facts. `▎ ❯ hello` reads; `▎ ▎ hello` does not. Every other kind now
emits one glyph.

The role information is not lost, and the test that says so is the right one:
`a_tool_call_and_a_notice_are_distinguishable_on_a_real_frame_without_colour`
compares **the cell at column 0**, which is where the gutter lives. Four golden
assertions were updated from `▸ ▸ …` to `▸ …`, and a new assertion walks every
row and fails if the first two characters are equal.

## The render hot path

Five separate costs, each on the draw path or one step from it.

| # | Site | Was | Now |
|---|---|---|---|
| 1 | `wrap::tokenize` | one `String` **per token** | borrowed `&str` slices |
| 2 | `wrap::split_at_width` | one `String` **per character**, on the hard-split path | `UnicodeWidthChar::width(ch)` |
| 3 | the streaming tail | wrapped **twice** per frame, once to count and once to draw | count borrows too; the copy is gone |
| 4 | `RenderCache::reconcile` | `HashSet` build + `retain` over every line, **every frame** | gated on the transcript having shrunk |
| 5 | `catalog::lookup_model` | a linear scan of ~380 rows, 2–3× per frame | `OnceLock<HashMap>` |
| 6 | `App::hints` | two `env::var_os` + a path re-shorten, per frame | computed once in `App::new` |

Number 2 deserves its own note. `ch.to_string().width()` is a correct and
completely ordinary-looking line, and it was allocating once per character on the
path taken by any word wider than the wrap width — which for a coding agent means
a file path, a URL, a long identifier, or a base64 blob. It is exactly the text
tool output is made of.

Number 3 is the one with the largest constant. The in-flight answer is wrapped
twice per drawn frame: `view.rs:streaming_tail_rows` builds the whole
`Vec<Vec<(String, Style)>>` only to read `.len()`, and `panels.rs:draw_transcript`
builds it again to draw it. At thirty frames a second over a 20 KB answer that is
~16,000 heap allocations per frame, half of them discarded. Both callers now hold
a `&str`, so both borrow. The invariant that the count and the draw agree is
preserved *by construction* — it is one function called twice — and
`counting_and_drawing_agree_because_they_are_one_function` pins it across six
widths.

Number 4 contradicts a doc comment that claimed "O(1), independent of transcript
length". The rendering is O(1); the bookkeeping around it is not. The comment now
says what is true.

## The per-event scan

`restale_finished_cards` walks every transcript line and adds every finished card
to a set that was a `Vec<u64>`, tested with `contains`. That is `O(lines + cards²)`
per call, and the event loop drains up to 128 events before drawing one frame —
so a streaming run paid it 128 times per frame to almost always find nothing.

Two changes, both conservative:

- the set is a `HashSet`, so the membership test is O(1) and the whole call is
  O(lines);
- `event_can_end_a_card` skips it for `MessageUpdate`, the streaming prose delta
  that dominates a run by two orders of magnitude and cannot end a card.

The predicate returns `true` for anything it is unsure about. A false negative
would leave a card drawn as pending after it returned — "reports success while
the thing did not happen", the failure this repository treats as the worst
available. A false positive costs one linear walk.

## A reader that could not say no

`epsilon-lsp`'s `read_message` checked `MAX_HEADER_LINE` **after**
`read_line` had returned. `read_line` appends to a `String` until it sees a
newline or EOF, so the cap could only be consulted once the allocation it exists
to prevent had already happened. A language server that never sends a newline —
hung, broken, or not a language server at all — could make the process allocate
without bound.

`epsilon-ext` already had the right shape (`read_line_bytes`, `host.rs:706`): a
`fill_buf`/`consume` loop that checks the running total *before* storing the chunk
it is about to refuse. LSP now uses the same loop, and the module's own comment
("trusts the header only as far as it is bounded") becomes true.

**The test is a counting reader**, and that is the point of it. It offers 64 KiB
one byte at a time and records how much the reader under test actually pulled. An
unbounded reader consumes the whole budget, sees a clean EOF, and returns
`Ok(None)` — *successfully*. A test that asserted only "the call returned" would
pass against exactly the defect it exists to catch. The assertion is on
`consumed <= MAX_HEADER_LINE + 1024`, and a second one asserts it is strictly less
than the budget, so "bounded" cannot be satisfied by "it eventually stopped".

## The lint law, and the checker that enforces it

`epsilon-safety` must declare `unsafe_code = "allow"`, because `forbid` is
un-overridable and the Job Object FFI is the only `unsafe` in the workspace. That
forces a hand-written `[lints]` table — and Cargo does not merge package and
workspace lint tables, so **that crate alone** was opted out of every lint the root
defines. It is the crate containing every `unsafe` block in the workspace.

The obvious fix does not exist. Measured, not read off a document:

```
error: cannot override `workspace.lints` in `lints`, either remove the overrides
       or remove `lints.workspace = true` and manually specify the lints
```

So `scripts/check-lint-tables.py` is the answer instead: every crate must either
inherit the workspace table or hand-copy it **exactly**, and any override must name
a reason. Wired into both gate scripts as a seventh gate.

**The first version of the checker passed on a workspace whose lint law was a
fiction, and two of three controls passed when they should have failed.** The
cause: `[workspace.lints]` is nested under `[workspace]` in the parsed TOML, and
the script read a top-level `lints` key — so it compared every table against `{}`
and found nothing to complain about. The control that caught it was *removing a
lint from the hand-copied table*, which then had nothing to be compared against
and so passed.

Three controls, each failing for the reason it names:

| Control | Result |
|---|---|
| omit `missing_docs` from the copy | FAIL — "missing … from the workspace table" |
| change `all` from `warn` to `allow` | FAIL — "sets all = allow, but the workspace says warn" |
| strip the reason from the comments | FAIL — "overrides unsafe_code … must say why" |

Recorded because the general form is the repository's own: **a control that
passes is a control you have not yet proven works.** This one passed three times
before the fixture was right.

## An unrouted command, in the build users run

`run_command` routes six groups by name and sends everything else to
`run_workspace_command`, whose module doc claimed: "A new `Command` variant
therefore cannot be silently unrouted: it lands in the third executor, where **the
compiler will insist its arm exist**."

It would not. The executor ended with `#[allow(unreachable_patterns)] other => {…}`,
and that arm is precisely what removed the compiler's insistence. `debug_assert!`
fires only in a debug build, so a **release** binary with an unrouted variant
returned `false` in silence: a slash command that does nothing, with no notice.

Removing the arm is not an option — the catch-all receives *every* variant,
including the 29 its six siblings own by name, so it would have to re-enumerate
all of them. The claim was the thing that was wrong, not the code. So the arm
stays and is made honest: it now pushes a notice naming the command.

`the_catch_alls_other_arm_tells_the_user_instead_of_failing_silently` asserts the
arm exists **and** that it reports, reading the source — because the branch is
statically unreachable in a correct build, and a runtime test could not reach it
to observe anything.

## A flag that was written and never read

`UiState::compacted` was set in four places and read in none, carrying
`#[allow(dead_code)]` and a ten-line comment explaining that the accessor which
read it had been deleted as dead.

The flag was not dead; its *reader* was. It is the only thing in the app that
knows the difference between a context size that is unknown because nothing has
run yet and one that is unknown because a compaction just replaced it. The second
means the number the compaction trigger reads is, right now, an estimate with
nothing behind it — which is a thing a reader watching that trigger wants to know.

The gauge now prints `· compacted` beside the `?` in that window. The metric's own
comment had claimed the flag "had nothing left to say"; the two arms it removed
printed the same string because the string itself carried no reason, which is the
defect, not a reason to drop the reason.

## Dead code, and why dead code with a comment is worse

| Removed | Why |
|---|---|
| `catalog::CustomProvider` + `lookup_custom` | byte-identical duplicate of `custom::`'s; **zero** callers |
| `llm::http::status_error` | no caller, and its comment claimed a wiring that did not exist |
| `files::_path_witness` | existed to justify the import it consumed; both gone |
| `mcp::invalidate_health` | `#[cfg(test)]` **and** dead — not even a test called it |
| 4 redundant `#[allow]`s | items were live and the lint could not fire |
| `jobobject::terminate`'s doc | said "test/diagnostic hook"; the **production** timeout path calls it |

The `http::status_error` case is the one worth keeping. A suppression plus a
comment describing a wiring that does not exist is worse than no function at all,
because the next reader believes the comment. And the live equivalent —
`webfetch::status_error`, five times the snippet cap — was in the tree the whole
time.

## A guard that caught me, and then a control that caught the fix

`no_source_file_lost_a_test_since_head` failed on this milestone. It reported six
deleted test names: `_path_witness`, `from_config`, `invalidate_health`,
`lookup_custom`, `matches_id`, `status_error`.

Every one was **dead code I had deliberately removed**, and not one was a test.
The guard collected every `fn` in the diff, so deleting a dead helper read
exactly like deleting a test — its own comment predicts the consequence ("a guard
that cries wolf is a guard that gets disabled"), and that is what happened, one
milestone after the guard was written.

Narrowed to functions actually carried by `#[test]` / `#[tokio::test]` /
`#[bench]`, with a control
(`the_test_classifier_reads_an_attribute_from_the_hunk_above`) that exercises
the classifier on diff text directly rather than mutating the tree.

**The control earned its place immediately, twice.**

1. It returned an empty set. The lookback tested `line.trim()`, which for a diff
   leaves `-#[test]` — not `#`-prefixed — so **every test looked like a plain
   function**. The guard would have gone quiet, which is the exact failure it
   exists to prevent, and nothing else would have reported it.
2. Its next assertion — "a rename's net effect must be nil" — failed, and **my
   assertion was wrong, not the guard**. Renaming a test *is* a deletion of one
   name and an addition of another, and the guard reporting that is right: a
   milestone doc or a `cargo test <name>` in someone's muscle memory now refers
   to nothing. The first version asserted the guard is blind to renames, which is
   a defect rather than a requirement.

## Three of my own assertions were wrong before the code was

1. `by_id_agrees_with_a_linear_scan` compared `&ModelInfo` **pointers** and failed
   on every row, reporting a duplicate `faux-1`. There is no duplicate:
   `CATALOG_ROWS` is a `const`, and a `const` is materialised separately at each
   use site, so two `MODELS` references are two equal copies at two addresses.
   The assertion was measuring where the compiler put a constant. Now compares
   `(label, context_window)` — what consumers actually read.
2. The LSP boundary test built its two fixtures separately and the second lost
   the `X-Pad: ` prefix, so "one byte past the cap" was in fact far under it.
   Both now come from one parameterised helper that asserts its own length.
3. The wrap tests asserted `rows.concat().len() == 200`, which joins the inner
   `Vec`s and so measured the **row count** (29), not the characters. And the
   "counting and drawing agree" test compared `wrap_line` on text *containing*
   newlines against a split-then-wrap sum — two different operations, since a
   newline is a line break to the callers and mere whitespace to the function.
   It failed at width 1 with 30 against 33, which said nothing about either.
3. **In the implementation itself.** The first bounded reader checked
   `buf.len() + pos > MAX`, forgetting that `pos + 1` bytes — terminator
   included — were about to be stored. That accepted exactly one byte more than
   the check it replaced: a silent widening of a bound nobody was told about.
   The boundary test is what found it, which is what it is for.

## The eviction gate I wrote was wrong, and an existing test caught it

The per-frame `HashSet` + `retain` is gone, gated so it only runs when a line
actually left the transcript. My first gate was `lines.len() < prev_lines_len`,
and it is **unsound in exactly the case that matters**: `/clear` followed by a
re-append of the *same number* of lines leaves the length unchanged, the gate
never fires, and the stale entries survive forever.

`clear_then_reappend_drops_stale_entries` — which already existed, for the
`/clear` path — failed on the fix. It is the reason a change this size is made
under a suite rather than by reading.

The correct condition is `reuse < prior_reconciled`: if the whole
previously-reconciled id prefix is still the prefix of the live lines, then every
id the cache holds is either in that prefix or was added during this call, and
the sweep has nothing to find. The prefix breaking is the actual signal. That
needs one extra local and no field at all — the first version added a
`prev_lines_len` field to the struct, which the correct version does not need.

## The doubled glyph was in two places, not one

`row_marker` in the render cache is the body prefix for plain rows. Tool **cards**
bypass it entirely: `ui::card::title` printed its own status glyph, and the
gutter printed the same one. Removing the body marker alone fixed notices and
left every card still doubled — which is exactly what the battery reported, and
why `▸ ▸ bash` and `• • run finished` failed while the fix looked complete.

The card title now prints the **status word** (`bash boom · failed · 0ms`) and
not the glyph. The word is the part that survives `NO_COLOR`, a 16-colour
terminal, and a font that cannot draw the glyph — and `partial` and `denied`,
the two statuses with no distinct gutter glyph, are unambiguous by word alone.

A new golden test holds the property for every row kind at once
(`no_transcript_row_repeats_its_gutter_glyph`): it reads the glyph the gutter
**actually painted**, from the buffer cell at column 0, and asserts the body does
not open with it. The three corrected string literals that preceded it each had
to be edited by hand and none of them noticed the cards.

## Key decisions

- **The frame was rendered before anything was changed.** Not read about. The
  doubled glyph was found in a screenshot, and it is the single most visible
  defect in this milestone, and no amount of reading the transcript module had
  found it.
- **`wrap_line` takes `&str`.** Not `Cow`, not a new type. Two of its three
  callers already held a slice and were manufacturing an owned copy for it.
- **The catch-all's `other` arm stays.** The alternative was worse and the doc
  comment, not the code, was the defect. Removing the arm was tried; the compiler
  reported 28 uncovered patterns, because the catch-all receives *every* variant.
- **`event_can_end_a_card` is deliberately over-permissive.** A missed restale
  shows a finished card as pending. A spurious one costs a walk.
- **`check-lint-tables.py` allows an override to differ in *level*, not just to
  exist.** `unsafe_code` is `forbid` at the root and `allow` in `epsilon-safety`;
  flagging that would be flagging the one difference allowed to exist.

## What nothing exercises

- **`--tui-bench` still cannot see any of this.** It calls `draw_frame` directly
  and its `bench_app` holds no streaming text, so it measures neither the
  double-wrap (fixed), the per-event restale scan (fixed), nor the
  `MessageUpdate` path (now skipped). Every number in the table above is derived
  from the code's asymptotics and its measured call counts, not from a benchmark.
  A bench that streams a tail and replays 128 events would be the honest
  instrument, and it does not exist.
- **The `?` / `· compacted` pair is tested at the `Metrics` level only.** Nothing
  asserts it reaches a real frame at a width where the header still has room.
- **`check-lint-tables.py` is not itself tested.** Its four controls are recorded
  in `audit/evidence/gate-m198-final.md` and are reproducible by hand, but no test
  runs them. It is the one new component here whose failure mode is "silently
  stops checking", which is precisely the one the control history says arrives
  without warning.
- **The glyph work is asserted on text, not on styling.** The two glyphs were
  always styled identically, so no style assertion could have caught the
  doubling — and `no_transcript_row_repeats_its_gutter_glyph` reads the gutter
  cell to be precise, but the *fix* is unobservable to a style test.
- **Nothing asserts the glyphs in a real conhost.** The terminal integration test
  drives the real binary and is in the battery, but it asserts on `contains`.

## Verification

Battery `m198-final4`: **GATE PASS**, all seven gates `exit=0`.
**1,560 tests passed, 0 failed, 1 ignored, across 63 test binaries** — counted by
summing the log's `test result: ok. N passed` lines, not by subtracting a
per-crate sweep from a battery total, which is the arithmetic this repository has
already caught itself doing once.

Three attempts failed before this one, and each found something real:

- `m198-final` — `fmt --check`, and a `clippy::iter_nth_zero` denial in the new
  glyph assertion.
- `m198-final2` — `no_source_file_lost_a_test_since_head` firing on the dead code
  this milestone deliberately removed.
- `m198-final3` — the glyph fix being **half** a fix (cards bypass `row_marker`),
  and the unsound `reconcile` eviction gate.

The third is the one worth remembering: the half that was done looked finished,
and it took a battery to say otherwise. The fourth is the first whose log
describes a tree that still exists. Full gate table, timings, and the four
controls for the new lint checker are in
`audit/evidence/gate-m198-final.md`.

## Files

- `crates/epsilon-tui/src/wrap.rs` — borrowed tokens, no per-character allocation
- `crates/epsilon-tui/src/render_cache.rs` — gated eviction, marker rule, `HashSet` restale
- `crates/epsilon-tui/src/app/{view,panels,commands}.rs`, `app.rs` — tail borrow, event gate, catch-all notice
- `crates/epsilon-tui/src/app/{session,tests,golden}.rs` — location cache, guard, marker expectations
- `crates/epsilon-tui/src/ui/metrics.rs`, `src/state.rs` — the compaction reader
- `crates/epsilon-llm/src/catalog.rs` — id index; dead duplicate removed
- `crates/epsilon-llm/src/http.rs`, `retry_stream.rs` — dead code, stale suppressions
- `crates/epsilon-lsp/src/framing.rs` — bounded header read + counting test
- `crates/epsilon-mcp/src/lib.rs`, `epsilon-tools/src/files.rs` — dead code
- `crates/epsilon-safety/src/sandbox/jobobject.rs`, `Cargo.toml` — false doc, lint note
- `scripts/check-lint-tables.py`, `scripts/gate.{ps1,sh}` — the seventh gate
