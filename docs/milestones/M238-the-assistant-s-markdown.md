# M238 — the assistant's markdown, and a message that arrived alone

Until now the transcript drew every answer as one flat coloured line. A model that writes
`**bold**` had its asterisks on screen.

## What is shared and what is not

The terminal frontend has a markdown renderer (`epsilon_tui::markdown`). **It is not
portable**, and pretending otherwise would repeat the mistake M236 just corrected in the
other direction: two renderers share a *parser* and nothing else. `epsilon_tui::markdown`
emits ratatui `Line`s over `Style`, coupled to a width-aware table layout and to the
terminal's own `Theme`.

So the parse is shared and the draw is per-frontend — which is exactly what
`pulldown_cmark` (already in this workspace for the terminal) is for. The rule both renderers
keep is the one that matters: **when a construct cannot be laid out honestly, fall back to
something readable rather than to something that looks like a rendering bug.** A table
becomes plain rows, for the reason the terminal renderer already gave.

### Only the assistant's text is interpreted

A user who types `**hi**` must see `**hi**`. A shell command in the transcript stays
verbatim. A diff line is a diff line. **Rendering is an interpretation, and applying it to
text that is not prose makes the window lie about what was said** — and
`a_users_markdown_is_not_interpreted` pins that at the frame, not at the renderer.

## Two egui 0.36 details that shaped the design

**`RichText` has private fields and no public concatenation.** A block with mixed styling
cannot be built by collecting `RichText`s. It is built from a `LayoutJob` — which is also
the *right* answer independently: one layout job wraps as one paragraph, where one label
per span would not.

**`egui_extras` 0.36 has no `markup` module.** The reuse answer here is the parser already
in the tree, not a new dependency.

## Five bugs, and the parser is not the hard part

Writing the renderer was straightforward. Getting it *right* was four of the five:

1. **Every bullet rendered as `1.`, `2.`, `3.`** — `Tag::List(start)` gives `Some(n)` for an
   ordered list and `None` for a bullet one, and the first version pushed "ordered" for both.
2. **A paragraph inside a list item overwrote the item's kind**, so every bullet rendered as
   a plain paragraph and the marker was lost.
3. **A one-line code fence became two lines**, the second empty: pulldown-cmark delivers a
   fenced block's text *including* its trailing newline.
4. **The table header row vanished**, because the parser emits the header's cells **with no
   `TableRow` around them** — `End(TableHead)` needed the same handling as
   `End(TableRow)`. Found by dumping the event stream, not by reading harder.
5. **`MessageEnd` alone produced no transcript row at all.** The reducer *updated* a row
   that `MessageStart`/`MessageUpdate` had opened; a provider that reports only the finished
   message left the agent's answer invisible. That is the same class as the `MessageStart`
   omission M235 fixed, one event later, and it was found by the markdown frame test seeding
   exactly that event shape.

The fifth is the one that matters most. It is not a rendering defect: the agent ran, the
answer arrived, and the window said nothing.

## Verification

**Battery `m238` — GATE PASS on attempt 1, 1,752 tests across 72 binaries, 0 failed**
(`audit/evidence/gate-m238.log`). Up from 1,735: fifteen markup tests, two frame-level ones
and one reducer regression. clippy clean across 14 crates.

## What nothing exercises

- **Markdown through a real model.** The frame tests seed events; no live provider emits
  markdown, so the streaming path re-parsing on every `MessageUpdate` is reasoned about and
  bounded but not measured.
- **The cost of that re-parse.** It is O(n) per delta on a growing document — bounded by the
  transcript's row cap, and the first thing to revisit if a long answer stutters.
  - **Links and images.** A markdown link -- square brackets around the text, then a URL in
    parentheses -- renders as its text and loses the URL. A link the reader cannot follow
  all today, which is a narrower, more honest failure, but it is still a gap.
- **Syntax highlighting** inside fenced blocks. The language tag is shown; nothing colours
  it, and `syntect` is already in the tree through `egui_extras` if that is wanted next.
