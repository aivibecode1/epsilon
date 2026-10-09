# Investigation sweep — 2026-10-09

**The stream this serves:** goal *"fresh, evidence-cited audit → converged ranked plan
([`../../docs/plans/026-convergence.md`](../../docs/plans/026-convergence.md)) → implement the top tier"*. This file is the audit half:
what was measured, where, and with what citations. Nothing here was read from memory; every
claim below was produced by a command or a file read in this session.

## Method, stated honestly

**No subagent tool exists in the harness this session ran in.** The goal's investigation brief
anticipated parallel read-only subagents; the available tools were read, bash, edit and write
only. The streams therefore ran as **batched read-only passes in one session** (parallel tool
calls), with the same citation discipline AGENTS.md §5 demands of any investigation. The
parallelism the brief bought was investigation breadth, not process parallelism. Where a claim
could not be verified in-session it is marked *unverified* rather than repeated.

## 0. Scoreboard health (the instrument itself, first)

| Check | Command | Result |
|---|---|---|
| Citations resolve | `python scripts/check-parity-citations.py` | **467 citations in [`pi-tui.md`](../../docs/parity/pi-tui.md) + [`dsh-desktop.md`](../../docs/parity/dsh-desktop.md) resolve** (exit 0) |
| Counts agree | `python scripts/check-parity-counts.py` | exit 0 |
| Verb counts agree | `python scripts/check-verb-counts.py` | exit 0 |
| Links | `python scripts/check-links.py` | **966 links, 0 broken** (exit 0) |

**Row statuses re-counted from the matrices** with escaped-pipe handling:

| Matrix | done | partial | missing | deliberately different | total |
|---|---|---|---|---|---|
| `parity/pi-tui.md` | 61 | 114 | 141 | 23 | 339 |
| `parity/dsh-desktop.md` | 20 | 93 | 278 | 42 | 433 |

These match `docs/STATUS.md` exactly. A naive parser on the same files produces
91/277 for DSH — the escaped-`|` trap `STATUS.md` already documents, reproduced and then
corrected. **No duplicate ids in either matrix.**

**`done` rows' named tests exist:** all 119 distinct test names extracted from the 81 `done`
rows' `verified` cells exist as `fn` names under `crates/` (measured by grep of the tree).
That is existence, not pinning — see the caveat under plan 026.

## 1. Architecture and the agent loop (task-1a)

| Invariant | Verdict | Evidence |
|---|---|---|
| §2.2 one composition point | **Holds** | `build_runtime`/`resolve_launch` are called from `crates/epsilon-cli/src/main.rs`, `crates/epsilon-desktop/{src/lib.rs,src/app.rs,src/session.rs}` and defined in `crates/epsilon-runtime/src/lib.rs` — no second composition site. |
| §2.5 no library depends on `epsilon-cli` | **Holds** | `grep -rn "epsilon-cli" crates/*/Cargo.toml` returns only comments. |
| §2.6 refusal path never exits | **Holds** | `process::exit` in library crates: only `crates/epsilon-ext/src/bin/ext-fixture.rs:61,66,251,252` — a deliberate test fixture binary. `epsilon-desktop::run` returns `Err(String)` (README, `src/lib.rs:118`). |
| §2.4 one command registry | **Holds for commands and keybindings** | `epsilon-contracts/src/command.rs` is read by both front ends; the window now reads the shared `epsilon-contracts/src/keybind.rs` table (`crates/epsilon-desktop/src/keys.rs:1-20` — the M315 fix). |
| Sessions root (M217) | **Holds** | The window discovers sessions through `epsilon_runtime::sessions_root` (`crates/epsilon-desktop/src/app.rs:9843,10361,10435,10668`). |

Scale, measured: **180,466 lines of Rust** across 14 crates; largest single file
`crates/epsilon-desktop/src/app.rs` at **18,578 lines** (test module included; the
`#[cfg(test)]` block starts at `:8729`), then `crates/epsilon-tui/src/app/tests.rs` (7,577).

## 2. TUI information architecture (task-1b)

`crates/epsilon-tui/src/ui/transcript.rs` is a considered presentation layer, not a printer:
a two-column gutter carries role ("who is speaking") with colour as reinforcement, never as
the only carrier — `row_style` (`:44`) returns glyph+style+band per `LineKind`, and the
`has_surfaces` fallback (`:52-63`) swaps in glyphs when truecolor bands are unavailable. The
acid test `every_role_is_distinguishable_without_colour` (`:437-489`) enumerates the roles and
asserts pairwise distinct gutters, including the answer/reasoning pair and the failure/system
pair; its own doc records that it once asserted the collision **equal** and was rewritten
(`:481-489`). `decorate` (`:148`) is documented as the production path and the comment
(`:163-166`) records that it used to be test-only — the exact class of defect where state
assertions passed while the screen was missing the feature.

**Assessment.** The philosophy's TUI complaint ("presentation, not functionality") has been
under active repair for 30+ rounds (70–103) and the remaining gaps are itemised in the matrix
rather than visible on the screen. The highest-value remaining presentation rows are the
matrix's own transcript partials: **P-058/P-128 (fixed `Error:`/`Warning:` prefixes)**,
**P-121/P-128 (compaction card)**, **P-183 (abort text lands on the answer, not a system row)**.

## 3. Desktop vs the DSH-Desktop bar (task-1c)

Strengths verified: `crates/epsilon-desktop/src/layout.rs` is a pure, tested geometry function
with a four-step degradation order (`:18-28`) and **a recorded WCAG deviation with its
mitigation** (`SIDEBAR_HANDLE_W` = 6 pt against SC 2.5.8's 24 pt; `:105-113`). The palette is
converted one way from the terminal's roles (`src/theme.rs:28`), not forked.

### Verified live defect — D-073 / I-104: the permission gate's Enter routing

The matrix rows (verified by reading both sides this session):

- `docs/parity/dsh-desktop.md:217` — D-073: *"the window resolves the gate from a global
  unmodified-Enter read that runs before any widget is drawn, so Enter always means 'allow' —
  Enter aimed at the Refuse button still allows."*
- `docs/parity/dsh-desktop.md:569` — I-104, the same claim for the Reject button.

Code confirms it end to end:

1. `crates/epsilon-contracts/src/keybind.rs:503-504` — `window.approval.allow` = `enter`,
   `window.approval.deny` = `escape` (unmodified).
2. `crates/epsilon-desktop/src/app.rs:4605` — `approve_pending` is called at the top of the
   frame's key handling, before panes and before any widget is drawn; its comment
   (`:4598-4604`) shows the ordering was deliberate for the composer.
3. `crates/epsilon-desktop/src/app.rs:8711-8730` — `approve_pending` resolves `AllowOnce` from
   `keys.take(ctx, "window.approval.allow")`.
4. `crates/epsilon-desktop/src/keys.rs:225-243` — `take` **removes the matching event from
   egui's stream** (`i.events.retain(...)`), so the widget the reader aimed at never sees it.

So with an ask parked and focus on the **Refuse** button, Enter allows. The buttons work by
click; the keyboard disambiguation is the defect. This is a §6-domain safety defect with a
wrong-direction failure mode (permissive instead of conservative), and the fix is small:
the gate must yield Enter/Escape to a focused approval widget. It is the top of plan 026.

## 4. Session and state infrastructure (task-1d)

Sessions live under the shared sessions root and are re-read by both front ends
(`epsilon_runtime::sessions_root`, cited above; the store itself is `epsilon-session`).
Continuity is exercised from both sides: `crates/epsilon-desktop/tests/window.rs` drives
`switch_session`; the terminal's `/resume` and `/import` run through the same
`epsilon-session` paths (`crates/epsilon-tui/src/app/commands.rs`,
`crates/epsilon-tui/src/app/session.rs`). No divergence found.

## 5. Tools and tool-call presentation (task-1e)

Not deep-dived beyond the matrix. The presentation side is covered by rounds 70–98 (cards,
caps, spill names, severity rows) and the matrix's remaining rows (P-138–P-177 cluster) are
itemised. No fresh defect found in this pass; recorded as **no finding** rather than padded.

## 6. Extensions, MCP, skills, `.agents`, and the ponytail verdict (task-1f)

- **`epsilon-ext`** is out-of-process by design (ADR-009): line-delimited JSON over stdio,
  per-call timeout, crash budget (`crates/epsilon-ext/README.md:1-12`).
- **Skills** implement the Agent Skills spec (ADR-015): `SKILL.md` directories with
  frontmatter, one level deep (`crates/epsilon-core/src/skills.rs:1-2,:100`).
- **Ponytail verdict: ADOPT — keep, and it is already load-bearing.** It is not a new idea
  bolted on: `AGENTS.md` §5 *is* the ponytail ladder in prose ("Ask Ponytail first… walk the
  ladder… write the minimum"). The skill adds the sharpened rungs, the "test only where the
  corner is cut" rule, and the `ponytail:` marker discipline, and the tree already carries
  **12 `ponytail:` markers** in Rust source (measured: `epsilon-llm`, `epsilon-session`,
  `epsilon-tools`, `epsilon-tui` ×4, `epsilon-tui/src/latex.rs` ×2, others). Nothing should be
  adopted *because* the skill exists; the verdict is adoption on merit: the repo's own
  evidence-discipline section (§5) is the same failure mode ponytail names, packaged
  sharper and with an enforcement marker.

## 7. Reference re-audit (tasks-1g/1h) — sample verification

Four rows pulled and verified against both sides:

| Row | Matrix claim | Verification |
|---|---|---|
| D-073 / I-104 | partial; Enter always allows | **Confirmed live** — §3 above. |
| S-122 | missing; no keybinding recorder | **Confirmed**: `crates/epsilon-desktop/src/keys.rs` has load/save paths and no recorder; the table to record into exists (`epsilon-contracts/src/keybind.rs`). |
| D-033 | missing; quit dialog keyboard shape | Consistent with code: dialog exists with `CancelClose` enforcement (round 17) and Escape= Cancel; button order is a recorded deliberate difference. |
| D-007/D-008 | missing; context menus in editable fields | Confirmed blocked on egui 0.36's public API (round 59 record); no `arboard`/menu surface exists. |

**The matrices' partial/missing claims sampled here are honest.** The one class we could not
re-verify in-session is "the named test pins the behaviour" (F5 from the 2026-10-06 review);
test *existence* is 119/119 measured above, pinning quality is unverified at scale and is
named as a sampling candidate in plan 026 rather than assumed.

## 8. Other references (task-1i)

Six references, all MIT, all studied and credited in `docs/references/README.md` with
per-project "what we took" columns. Nothing was copied. The codewhale page explicitly records
what is still missing (no reduced-motion path) — a living gap list, not a finished claim.

## 9. Duplication and complexity hunt (task-1j)

| Candidate | Verdict |
|---|---|
| Three `safe_to_show`/`display_safe` strips (`epsilon-runtime/src/text_safety.rs:92`, `crates/epsilon-desktop/src/app.rs:7189`, `crates/epsilon-tui/src/app/panels.rs:43`) | **Not a defect.** Both front-end copies are one-line adapters over the shared primitive (`epsilon_runtime::text_safety::display_safe`), differing only in return type. The reuse invariant holds. |
| Two markdown renderers over pulldown-cmark (`epsilon-tui/src/markdown.rs`, `epsilon-desktop/src/markup.rs`) | **Accepted platform duplication** — two draw targets (ratatui `Line`s vs egui text) over one parser. Same shape as the shared `call_head` (round 81). |
| TUI-local `/skills` row formatting (round 43 note: "ten TUI-local lines") | **Recorded, small.** Candidate for the shared-formatter move if a third formatter appears; not worth a round alone. |
| `crates/epsilon-desktop/src/app.rs` at 18.5k lines | **Complexity signal.** Tests start at `:8729`, so production code is ~8.7k lines in one file. Not a defect by itself; it is the file every desktop round touches, and the parser/`approve_pending` split (`:8711`) shows the seam pattern the file should keep following. |
| `ponytail:` markers | 12, each naming a ceiling — the debt ledger is small and current. |

## 10. The ranked findings the convergence starts from

1. **D-073/I-104 Enter routing in the permission gate** — safety-domain, wrong-direction
   failure, small fix, §6 rule implicated. (Verified §3.)
2. **The human items** (plan 025 §2.H): no screen-reader transcript, no keyboard-only pass,
   neither front end. Already the most-deferred item; needs the owner's calendar, not code.
3. **The credential contradiction** (plan 025 §2.E): `auth.json` plaintext vs locked §6;
   proposal 7 answered? Needs an ADR decision.
4. **The desktop design-quality program** (rounds 100–103, four slices landed; the owner's
   standing instruction).
5. **S-122 keybinding recorder** — largest single missing desktop surface (mechanism exists).
6. **TUI transcript partials** P-058/P-128 (prefixes), P-121 (compaction card), P-183 (abort
   text on the answer).
7. **Release** (plan 025 §2.I): preview install verified round 73; not tagged.
8. **Stale-row sampling** at scale (F5 class): existence measured, pinning not.
