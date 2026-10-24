# The Pi parity matrix

**What this folder is.** Epsilon's terminal UI is a *port* of somebody else's work:
[Pi](https://github.com/earendil-works/pi)'s interactive terminal mode. "Port" here means
**behavioural parity**: the same thing happens, in the same way, at the same moment — not
a rebranded copy of the pixels.

| Matrix | The target | Our side |
|---|---|---|
| [`pi-tui.md`](pi-tui.md) | Pi's interactive terminal mode | `crates/epsilon-tui`, `crates/epsilon-cli` |

(The DSH Desktop matrix was retired with the desktop in ADR-039. It is not kept here.)

## The one thing to understand first

**A port is not a copy.** Pi is a terminal program written in TypeScript. Its file layout,
package manager and event model have no equivalent in a Rust binary, and its README states
it has no permission system at all. Epsilon is a single Rust binary with one permission
gate over every front end.

So a row is allowed to say **no**, and is *required* to say why. The honest reasons are the
ones that would make the product worse if we did the reference's thing:

- **Licence.** Nothing was copied. Behaviour and structure were reimplemented; see
  [`../references/README.md`](../references/README.md).
- **Platform limit.** A terminal cannot show a sidebar or embed a browser engine.
- **A conflict with another front end.** The core defines one model; each mode maps onto
  it. A reconciliation is recorded in the row.
- **The accessibility standard.** [`../../AGENTS.md`](../../AGENTS.md) §8 requires WCAG
  2.2 AA. Where the reference is below that — Pi carries some state by colour alone — we
  meet the standard and say so.

Anything else is not a reason. "We did not get to it" is written as **missing**, not dressed
up as a decision.

## How to read a row

| Column | What it holds |
|---|---|
| **id** | Stable, never reused. `P-###` for the Pi TUI, `E-###` for ours. |
| **behaviour** | One sentence, as a person using the program would notice it. |
| **reference** | Where the reference defines it — a `path:line` into the vendored tree under [`../../references/`](../../references/). If there is no citation there is no row. |
| **status** | `done`, `partial`, `missing`, or `deliberately different`. |
| **ours** | Where we implement it — a `path:line` into `crates/`. |
| **verified** | How it was checked: the name of a test, or the screen or key sequence a person can repeat. |

**`done` means exercised, not written.** A row is `done` only when something was actually
run or rendered and looked at. Code that exists but was never reached from any surface is
`partial` or `missing`. This is [`../../AGENTS.md`](../../AGENTS.md) §5: a passing test is
not a claim that is true until you have seen what it drew.

Reference sources are vendored read-only. **Never edit, move or delete anything under
[`../../references/`](../../references/).**

## What is deliberately not here

- **A diff of the source.** The matrix compares what a person can observe.
- **A quality ranking.** Where our behaviour is better than the reference's, the row still
  says `done`, and the note says why we diverge.
- **Anything that moves.** Progress and open work live in
  [`../04-STATUS.md`](../04-STATUS.md). A matrix says whether a row is closed; it does not
  say what this week is for.
