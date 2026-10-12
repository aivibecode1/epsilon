# Parity matrices

**What this folder is.** Epsilon's terminal UI is a *port* of somebody else's
work: [Pi](https://github.com/earendil-works/pi)'s interactive
terminal mode. "Port" here means **behavioural parity**: the
same thing happens, in the same way, at the same moment — not a rebranded copy of the pixels.

[`dsh-desktop.md`](dsh-desktop.md) is **retired** (ADR-039): the desktop frontend it measured
is removed, and the file is frozen history, not a live matrix.

| Matrix | The target | Our side |
|---|---|---|
| [`pi-tui.md`](pi-tui.md) | Pi's interactive terminal mode | `crates/epsilon-tui`, `crates/epsilon-cli` |
| [`dsh-desktop.md`](dsh-desktop.md) | DSH Desktop | RETIRED (ADR-039): `crates/epsilon-desktop` is removed; frozen history |

## The one thing to understand first

**A port is not a copy, and these two references do not want the same thing.**

Pi is a terminal program written in TypeScript. Its file layout, its package manager and its
event model have no equivalent in a Rust binary, and its README states it has no permission
system at all. DSH Desktop is an Electron shell around a web application, and its UI lives in
sixty-odd packages that ship to a browser. Epsilon is a single native window and a single native
terminal binary, with no runtime to install.

So a row in these matrices is allowed to say **no**, and is *required* to say why. The honest
reasons are the ones that would make the product worse if we did the reference's thing:

- **Licence.** Nothing was copied. Behaviour and structure were reimplemented; see
  [`../references/README.md`](../references/README.md).
- **Platform limit.** A terminal cannot show a sidebar; a canvas-less native window cannot embed a
  browser engine without becoming a web app, which [`../../AGENTS.md`](../../AGENTS.md) §2.3
  forbids.
- **A conflict between the two ports.** Pi and DSH disagree about sessions, permissions and
  tasks. Epsilon defines **one** model in the core and maps each front end onto it. The
  reconciliation is recorded in the row, not left to whichever front end you happen to open.
- **The accessibility standard.** [`../../AGENTS.md`](../../AGENTS.md) §8 requires WCAG 2.2 AA.
  Where the reference is below that — and Pi carries some state by colour alone — we meet the
  standard and say so. That is divergence, and it is deliberate.

Anything else is not a reason. "We did not get to it" is written down as **missing**, not dressed
up as a decision.

## How to read a row

Every row has the same shape:

| Column | What it holds |
|---|---|
| **id** | Stable, never reused. `P-###` for the Pi TUI, `D-###`/`C-###`/`S-###`/`P-###`/`L-###`/`I-###` for DSH Desktop, `E-###` for our terminal, `ED-###` for our window. |
| **behaviour** | One sentence, as a person using the program would notice it. |
| **reference** | Where the reference defines it — a `path:line` into the vendored tree under [`../../references/`](../../references/). If there is no citation there is no row. |
| **status** | `done`, `partial`, `missing`, or `deliberately different`. |
| **ours** | Where we implement it — a `path:line` into `crates/`. |
| **verified** | How it was checked: the name of a test, or the screen or key sequence a person can repeat. |

**`done` means exercised, not written.** A row is `done` only when something was actually run or
rendered and looked at. Code that exists but was never reached from any surface is `partial` or
`missing`, and the row says which. This is [`../../AGENTS.md`](../../AGENTS.md) §5: a passing test
is not a claim that is true until you have seen what it drew.

## Where the detail lives

The matrices are the **verdict**. The evidence they were built from is under
[`../../audit/evidence/`](../../audit/evidence/), and every row's citations resolve there:

| File | What it is |
|---|---|
| `parity-pi-tui-reference.md` | Pi's interactive mode, read from source: 356 rows, 613 citations |
| `parity-dsh-desktop-reference.md` | DSH Desktop, read from source: 403 rows, each cited |
| `parity-epsilon-tui.md` | our terminal UI, read from source |
| `parity-epsilon-desktop.md` | our window, read from source |

Reference sources are vendored read-only. **Never edit, move or delete anything under
[`../../references/`](../../references/).**

## What is deliberately not here

- **A diff of the source.** Neither matrix is a line-by-line comparison, and neither claims to be.
  They compare what a person can observe.
- **A quality ranking.** Where our behaviour is better than the reference's, the row still says
  `done`, and the note says why we diverge. These documents are not a way to score anybody.
- **Anything that moves.** Progress, what is still open, and what was cut live in
  [`../STATUS.md`](../STATUS.md). A matrix says whether a row is closed; it does not say what this
  week is for.