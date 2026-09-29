# Working note — the frame-reading pass (started during M199)

**Status:** open findings from reading rendered frames at three widths. These belong
to the t2 ledger, not to the M199 commit. Kept here so the next pass does not
re-derive them, and because the evidence is on disk and cheap to re-read.

## How to look

```sh
export CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR="C:/Users/Hp/AppData/Local/Temp/epsilon-target"
EPSILON_FRAME_DIR="C:/Users/Hp/Desktop/My Agent Space/Epsilon/audit/evidence/frames/<label>" \
  cargo test -p epsilon-tui --lib frame_dump:: -- --ignored --nocapture --test-threads=1
```

`EPSILON_FRAME_DIR` must be **absolute**. A relative value resolves against the
package root (`crates/epsilon-tui/`), because that is a `cargo test` process's
working directory — the harness now refuses a relative one and prints every path it
writes. Baseline frames: `audit/evidence/frames/m199-baseline/`.

## F1 — a word is mutilated at 60 columns (measured)

The same assistant message renders three ways from one fixture:

| width | row 1 | last row |
|---|---|---|
| 100x30 | `…in the workspace and then read the layout module to understand` | `…changes to it.` |
| 80x24 | `…in the workspace and then read the layout` | `…changes to it` — **the final `.` is gone** |
| 60x20 | `…in the workspace and the` | `composed before making any changes to it.` |

At 60 the word **"then" is cut to "the"** and the next row starts at "read". At 80
the sentence loses its full stop. This is text the user reads as their own
conversation, and at 60 the model also reads it — the transcript is the same rows.

**Not in `wrap_line`.** `wrap.rs:25` hard-splits a token only when
`token.width() > width` — a word longer than the whole line — and otherwise moves the
whole word down. `"then"` is four columns, so the wrapper cannot have produced
`"and the"`.

**So the two widths disagree.** Arithmetic points at the gutter: at a 60-wide frame
the row is `• ` + 58 content columns. If the wrap ran at 58, `"…and"` (56) plus
`"then"` would be 61 and the word would have moved down whole. It did not, so the
wrap ran at a width **larger** than the drawn row and something downstream clipped it
to 58 — cutting mid-word.

**The probe that settles it** (do this before fixing; do not fix by reading): wrap
the exact fixture string at 56/57/58/59/60/61/62 and compare against the drawn row.
That names the component, and the two candidates want opposite fixes — a width that
does not account for the gutter (one subtraction at the call site) versus a clip
that should re-wrap rather than truncate (a renderer change).

## F2 — the header loses a glyph at 60 columns, silently

```
100: idle                               ?/1.0M · ↑0 ↓0 · R0 W0 CH n/a · Faux (offline scripted) · offline
 60: idle 0 ↓0 · R0 W0 CH n/a · Faux (offline scripted) · offline
```

At 60 the `↑` is gone and the row reads `idle 0 ↓0` — a **bare `0` where a labelled
pair used to be**. This is the M196 class exactly ("a partial badge is worse than a
missing one, because a wrong number is not noticed"): `0` reads as a value, not as
the loss of a label. The model picker must degrade by dropping whole segments
(`R0 W0 CH n/a`), never by dropping the marker that gives a number its meaning.

## F3 — `?/1.0M` and the trailing `· offline` are unlabelled

Carried from `agents/memory/next-hardening.md`, still open, still visible in every
frame above: nothing on screen explains `?`, `CH n/a`, or what the rightmost segment
is. Two lines in `/help` close it. The right edge also truncates hard against the
border (`…\Epsilon\crates\epsilon-tui`), so the last segment is cut mid-path.

## What is *not* wrong, measured

- **Markers are single.** `▎ ❯ run the tests` and `• …` — the M198 double-glyph
  defect is genuinely fixed; no `• •` anywhere in the baseline frames.
- **Cache rate reads `CH n/a`,** not `CH 0%` — M194's fix holds on screen.
- **The permission dialog renders and states its keys** — `y allow once · a always …`
  is present in the footer.
