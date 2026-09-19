# M71 — the edit matcher: three more stages and the guard that bounds them

**Scope:** port-plan item 9 (opencode `tool/edit.ts:694-737`, via
`audit/references/opencode-portable-parts.md` item 2).

## What landed

Every coding agent hits the same wall: the model knows what to change, and the edit tool refuses
because the text it quoted does not byte-match the file. Epsilon's chain had three stages (exact,
line-trimmed, indentation-flexible) and no bound on how much a loose stage could match. It now has
six stages, ordered by **rising risk** rather than by opencode's observed-frequency order, so a
file that contains the text verbatim is never "found approximately" somewhere else first:

| # | Stage | Catches |
|---|---|---|
| 1 | Exact | the common case |
| 2 | Line-trimmed | trailing whitespace from a rendering artifact |
| 3 | Indentation-flexible | the model guessed the indent depth wrong |
| 4 | **Escape-normalized** | `\n`/`\t`/`\r` written as literal two-character escapes |
| 5 | **Trimmed-boundary** | blank lines the model's copy has and the file does not |
| 6 | **Block-anchor** | openings and closings remembered, the middle paraphrased |

Stages 4–5 are fixed-width (they compare a window exactly as tall as `old_string`). **Stage 6 is the
only variable-height stage** — it finds a line resembling the model's first line, the nearest
following line resembling its last, and treats the region between as the block. That is precisely
why it needs the guard, and why the other five can never trip it: a test asserts that as a
*property* over a 300-line file rather than by example.

### The refusal

opencode's `isDisproportionateMatch`, ported: a span of `max(old_lines + 3, old_lines * 2)` lines or
more, or 500+ characters longer than `old_string`, is refused instead of applied. The `edit` tool
turns that into a diagnosis, not a mystery:

```
refused to edit big.rs: matching on the first and last lines found a region 42 lines tall for a
3-line old_string (438 extra characters). This usually means the anchors matched a different
block. Re-read the file and give the exact text to replace, or use a longer, more distinctive
old_string.
```

The file is not touched — asserted after the refusal, not assumed.

## Key decisions

- **opencode's real rule, not my paraphrase of it.** The roadmap's acceptance criterion said "a
  fuzzy match spanning > 30 % of the file is refused". Implementing that literally would refuse a
  40-line replacement in a 100-line file — a legitimate editing pattern, not a runaway match. The
  reference's rule is relative to `old_string`, which is the thing that can be disproportionate, and
  that is what shipped. The roadmap entry was corrected rather than the code.
- **Risk-ascending stage order** (deviation from opencode, documented in the module): opencode
  interleaves its stages by how often each is needed; Epsilon runs the safest first so a looser
  stage can never shadow a tighter one that would also have matched.
- **`new_string` stays verbatim.** These stages locate a region; they do not restyle the model's
  text. Where that leaves a visible artifact (a `new_string` that ends in a newline inside a file
  that supplies its own) the diff in the tool result shows it and the match note says the match was
  loose — pinned by test rather than waved away.
- **The guard applies to the pick, not the list.** Under `replace_all` the user asked for every
  occurrence, so a large span is not by itself a mistake; the refusal exists for the single-edit
  case where "matched" can mean "coincidence".
- **Batch edits stay exact-only.** The multi-edit path (`edits[]`) matches byte-exactly by design —
  several loose matches applied atomically is a much bigger blast radius. Its not-found message now
  says so and tells the model to send that edit on its own to get the fuzzy stages.
- **Six of opencode's nine stages, and the three that remain are named** (whitespace-normalized,
  context-aware, multi-occurrence): each needs an index-mapping layer from normalized text back to
  file line ranges, which is a bigger change than this slice and is now the recorded next step in
  the plan rather than an unstated gap.
- **Levenshtein is hand-rolled** (~20 lines, two rows) because the crate carries no algorithmic
  dependencies and should not gain one for a similarity score over two single lines.

## Test-caught bugs

- **A test that did not test what it claimed.** `the_guard_cannot_fire_on_a_fixed_width_stage`
  passed — but through the *anchor* stage, not the fixed-width one. Only the M3 mutation (removing
  the anchor stage) exposed it, because the test then failed. Rewritten to exercise a genuine
  fixed-width match (a whitespace-only line caught by line-trimmed) plus a property assertion over
  a tall file.
- **Three test premises were wrong, not the code.** The boundary stage kept "losing" to the exact
  stage: a payload with *one* leading blank line is a substring of the file's own text (the file's
  newline supplies it), so exact matching is correct and no boundary case exists. The stage only
  fires when the model's copy carries blank lines the file genuinely lacks — the test now constructs
  that, and asserts the clean result plus the verbatim-`new_string` artifact.
- **An anchor test proved the wrong thing.** "Two different modules" matched one region because
  `mod A {` and `mod B {` are 0.86-similar — the implementation was right (and careful), the test's
  premise was wrong. Rewritten with two blocks that share their anchors and differ in height, which
  also exercises variable-height `replace_all`.

## Verification

| Mutation | Result |
|---|---|
| disproportionality guard disabled | 2 tests fail (the line rule and the character rule) |
| anchor similarity threshold → 0.0 | 6 tests fail |
| block-anchor stage never runs | 6 tests fail |

New tests: 12 in `edit_match.rs` (including `similarity`/`levenshtein` basics, the guard's inclusive
boundary, `replace_all` over variable-height spans, and the fixed-width property) and 1 tool-level
test that drives the real `edit` tool through an anchor match and then through a refusal, asserting
the file is byte-identical afterwards.

Gate: `scripts/gate.sh m71` — fmt clean, clippy zero warnings, **650 tests, 0 failed**, demo
completes, `real`-feature build compiles.

## Plan status

Item 9 is **ported, with three of nine stages remaining** (named above, and in `03-port-plan.md`).
Tier 3 now has items 10 (path-conflict batch planner), 11 (subdirectory `AGENTS.md` hints), 12
(`@`-reference expansion) and 13 (typed corruption reasons) open.
