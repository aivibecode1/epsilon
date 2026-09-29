# M198: the frame path, measured

**Status: CLOSED.** Battery `m199c` — GATE PASS, 1,568 tests across 63 binaries — was
run against the tree carrying this milestone's changes, so the M198 work is verified.
The two failed attempts `m199` and `m199b` belong to the M199 slice, not to this one;
their findings are in `m199-containment.md`.

## The lesson to carry forward: render a frame and look at it

The TUI's transcript rendered every marker **twice**:

```
 2|• • The layout module owns the spatial model.        <- notice
 3|• • It splits the frame into a header, a transcript…  <- notice
```

`• •` for notices, `▸ ▸` for pending calls, `✓ ✓` for results, `✗ ✗` for
failures — four of the seven row kinds. `ui::transcript::row_style` paints the
role glyph in the gutter; `render_cache::row_marker` painted *the same glyph*
again in the body.

Nothing found this. It is not in `audit/03-defects.md`, it is not in the ledger,
and the module that causes it has ~450 lines of documentation explaining the
gutter system. **One `TestBackend` screenshot found it in a minute.** AGENTS.md
already says "a claim about what a user sees is verified by measuring, not by
reading" — it had just never been applied to *glyphs*.

**Do this first for any UI work here.** A scratch test that drives the real
`draw` and prints the screen is ~20 lines. Revert it afterwards.

## The rule that came out of it

A body marker is emitted only when its glyph **differs** from the gutter's. The
user turn is the row that earns a second glyph: gutter `▎` (whose turn), marker
`❯` (where input goes) — two different characters carrying two different facts.
Every other kind emits one.

`▎ ❯ hello` reads. `▎ ▎ hello` does not.

## What changed on the frame path

| Site | Was | Now |
|---|---|---|
| `wrap::tokenize` | a `String` **per token** | borrowed `&str` |
| `wrap::split_at_width` | a `String` **per character** | `UnicodeWidthChar::width(ch)` |
| streaming tail | wrapped **twice/frame** | count borrows too |
| `RenderCache::reconcile` | `HashSet` + `retain` over all lines, **every frame** | gated on the transcript shrinking |
| `catalog::lookup_model` | linear scan of ~380 rows, 2–3×/frame | `OnceLock<HashMap>` |
| `App::hints` | 2× `env::var_os` + path shorten, per frame | once in `App::new` |
| `restale_finished_cards` | `O(cards²)` per event, ≤128×/frame | `HashSet` + skipped for `MessageUpdate` |

`wrap_line` now takes `&[(&str, Style)]`. Two of its three callers already held a
slice and were manufacturing an owned copy.

## Correctness

- **LSP header read was unbounded in practice.** `read_line` appends to a
  `String` until newline-or-EOF, so `MAX_HEADER_LINE` could only be checked after
  the allocation it prevents. Now a `fill_buf`/`consume` loop, the shape
  `epsilon-ext` already used. The cap counts the terminator, because the check it
  replaces did — dropping that is a silent one-byte widening of a bound.
- **The catch-all dispatcher's `other` arm now prints a notice.** Its module doc
  claimed "the compiler will insist its arm exist". It would not: the
  `#[allow(unreachable_patterns)] other =>` *was* the reason, and `debug_assert`
  fires only in debug builds — so a **release** binary with an unrouted command
  returned silently. Removing the arm is NOT the fix (the catch-all receives
  every variant, including the 29 its siblings own by name — verified by the
  compiler: removing it gives 28 uncovered patterns).
- **`UiState::compacted` was write-only.** Given the reader it was written for:
  the gauge prints `· compacted` beside its `?`. Deleting the flag was the
  wrong call; its *reader* was missing.

## `scripts/check-lint-tables.py` — new seventh gate

Cargo **rejects** `workspace = true` on a table that carries overrides:

```
error: cannot override `workspace.lints` in `lints`, either remove the overrides
       or remove `lints.workspace = true` and manually specify the lints
```

(Measured, not read off a document — the Cargo book's wording invites the
opposite conclusion.) So `epsilon-safety` — the crate holding **every** `unsafe`
block — hand-copies `[workspace.lints]` and is opted out of every root lint. The
checker makes any drift a gate failure.

**Its first version passed on a workspace whose lint law was a fiction.**
`[workspace.lints]` is nested *under* `[workspace]` in parsed TOML; the script
read a top-level `lints` key, compared everything against `{}`, and found nothing.
Two of three controls passed when they should have failed. Three now fail naming
the defect; reproduce them by editing `crates/epsilon-safety/Cargo.toml`.

## Two of my own tests were wrong before the code was

Both worth recording, both the same shape as the repository's standing warning.

1. `by_id_agrees_with_a_linear_scan` compared `&ModelInfo` **pointers** and
   failed on every row, reporting a duplicate `faux-1`. There is no duplicate:
   `CATALOG_ROWS` is a `const`, and a `const` is materialised separately at each
   use site, so two `MODELS` references are two equal copies at two addresses. The
   assertion was measuring where the compiler put a constant. Fixed to compare
   `(label, context_window)` — what consumers actually read.
2. The LSP boundary test built its two fixtures separately and the second one
   lost the `X-Pad: ` prefix, so "one byte past the cap" was in fact far under
   it. Both are now built by one parameterised helper that asserts its own
   length.

## Machine notes

- `pwsh -File scripts/gate.ps1 <label>` — `bash` is WSL here and has no cargo.
- `CARGO_BUILD_JOBS=1`, `CARGO_TARGET_DIR=$env:TEMP\epsilon-target`.
- Never `bash -c` string surgery on a `.rs` file: four separate
  `-replace`/`Set-Content` attempts mis-handled CRLF and ate closing brackets.
  Use the edit tool.
- A focused `cargo test -p <crate>` still costs 60–90 s to compile after any
  dependency changed.
