# Independent review of the objective — 2026-10-06

**What this is.** An outside review of Epsilon against the objective it was built for, and the
evidence for every claim in it. It was written by an agent that did not build this tree, working
read-only except for this file, [`../plans/025-completion-plan.md`](../plans/025-completion-plan.md),
two factual corrections in [`../STATUS.md`](../STATUS.md), two proposals appended to
[`../AGENTS-PROPOSALS.md`](../AGENTS-PROPOSALS.md), and one battery log copied out of the disposable
`target/` directory into [`../../audit/evidence/`](../../audit/evidence/).

**The objective, as given to this project.**

> Create a unique harness that combines the best features of Pi Agent and DeepSeek Harness. The
> terminal is a direct port of Pi Agent; the desktop application is a direct port of the
> DeepSeek Harness desktop version. The additional references in `references/` are there to help
> integrate the best aspects of each.

**Read [`../STATUS.md`](../STATUS.md) for what is true, [`../HANDOFF.md`](../HANDOFF.md) for how to
work here, and [`../plans/024-finishing-the-objective.md`](../plans/024-finishing-the-objective.md)
for the item-level list of known defects.** This document does not repeat those; it judges the
objective and records what they do not.

---

## 0. Verdict

**As architecture and as research, the objective was met. As a port, it was not — and the objective
as written could not have been met by any amount of work, because it names two mutually
contradictory references as "direct" targets and never says what would count as finished.**

Measured at the end: the terminal reproduces **39 of 339** paired Pi behaviours as *exercised*
(**11.5%**), and the desktop window **11 of 433** DSH Desktop behaviours (**2.5%**). Those numbers
are the project's own, from the matrices it built on its last day, and they are honest — every row
carries a citation on both sides. The remaining 691 rows are `partial` (209), `missing` (453) or
`deliberately different` (60). The window answers **7 of the 37** command heads the terminal shares;
a real screen reader has **never** been run against it; and on this machine the window **does not
present at all**.

This is not a failed project. It is a **successful harness and an unfinished port**, and the gap
between those two descriptions is the whole finding.

---

## 1. The tree this review describes

| Fact | Measurement |
|---|---|
| HEAD | `67dde17` (2026-10-05), *"feat(m312): the desktop can change its own colour scheme"* |
| Working tree | **18 modified tracked files, +2,397 / −320 lines, 48 untracked entries** — M313, M314, M315 and M316 exist only on disk. *(Measured before this review's own four documentation edits, which bring the same counts to 19 / +2,515 / −322 / 51.)* |
| Newest source edit | [`../../crates/epsilon-desktop/src/app.rs`](../../crates/epsilon-desktop/src/app.rs), 2026-10-05 17:36 |
| The battery that describes this tree | [`../../audit/evidence/gate-m316d-2026-10-05.txt`](../../audit/evidence/gate-m316d-2026-10-05.txt) — 10 legs, all `[exit=0]`, **2,088 passed / 0 failed / 12 ignored across 78 binaries**, `GATE PASS`, 2026-10-05 19:07 |
| The battery `STATUS.md` cites | `gate-m315b`, 2026-10-05 **16:29** — which **predates** the newest source edit, so it describes a tree that no longer exists |
| Source | 138,489 lines under `crates/*/src` (185 files); 165,806 `.rs` lines in total; ≈2,120 declared test functions |
| Documentation | 338 `.md` files / 46,018 lines under `docs/`; 120 `.md` / 24,569 lines under `audit/`, plus 613 evidence files |
| History | 712 commits, 2026-08-23 → 2026-10-05 (43 days); **≈52% of commits touch no file under `crates/`**; one tag (`v0.1.0`) |

**What I ran, and what I did not.** I ran the five text gates — `check-links` (**901 relative links,
0 broken**), `check-agents-lock`, `check-lint-tables`, `check-line-endings`, `check-control-bytes`,
all `exit=0` — and the repository's own `check-reference-citations.py`. **I ran no cargo command and
launched no binary.** The last full battery on this tree is `gate-m316d`, which was already green
when I started; re-running a 40-minute battery to confirm a log I could read would have spent the
machine for no new information. Everything below is a measurement of the files, or is labelled as
the project's record.

---

## 2. The objective, clause by clause

| Clause | Verdict | Evidence |
|---|---|---|
| "a unique harness" | **Met** | One core, two front ends, one composition point: `build_runtime` and `resolve_launch` are called from `epsilon-cli`, `epsilon-desktop` and `epsilon-runtime`'s own tests and nowhere else; no crate depends on `epsilon-cli`. It is genuinely one engine. |
| "combines the best features of Pi and DSH" | **Met as research, partly as product** | Six vendored references, [`../references/`](../references/README.md), 869 source-cited inventory rows, [`../../CREDITS.md`](../../CREDITS.md), [`../references/TUI-DECISION.md`](../references/TUI-DECISION.md). The *integration* is real: permissions, goals, checkpoints, MCP and skills come from the references and exist here. |
| "the terminal is a direct port of Pi Agent" | **Not met** | [`../parity/pi-tui.md`](../parity/pi-tui.md): 339 rows → **39 done / 121 partial / 161 missing / 18 deliberately different**. One Pi behaviour in nine is exercised here. |
| "the desktop is a direct port of DSH Desktop" | **Not met** | [`../parity/dsh-desktop.md`](../parity/dsh-desktop.md): 433 rows → **11 done / 88 partial / 292 missing / 42 deliberately different**. Seven of 37 shared command heads are answered ([`../../crates/epsilon-desktop/src/commands.rs`](../../crates/epsilon-desktop/src/commands.rs), `SUPPORTED`). The window cannot present on this machine. |
| "the references are there to integrate the best aspects" | **Met** | This is the strongest part of the work, and the only clause that is fully closed. |

**A "direct port" of both was never possible, and the project proved that itself.** Pi is a
TypeScript terminal program whose README states it has *no* permission system; DSH Desktop is an
Electron shell around a web application, and [`../../AGENTS.md`](../../AGENTS.md) §2.3 forbids a
webview. [`../parity/README.md`](../parity/README.md) says so in its second paragraph: *"these two
references do not want the same thing."* The reconciliation the project chose — one core, one model,
two draw calls — is the right engineering answer. It also means **neither front end can be a direct
port**, and nothing in the objective acknowledged that trade.

---

## 3. What is missing

### 3.1 The gap, in the project's own numbers

| Matrix | rows | done | partial | missing | deliberately different |
|---|---|---|---|---|---|
| [`../parity/pi-tui.md`](../parity/pi-tui.md) | 339 | 39 | 121 | 161 | 18 |
| [`../parity/dsh-desktop.md`](../parity/dsh-desktop.md) | 433 | 11 | 88 | 292 | 42 |

Both totals were recomputed from the tables by this review, not copied. The DSH matrix's own total
line ([`dsh-desktop.md:123`](../parity/dsh-desktop.md)) agrees; `STATUS.md` did not (see F1).

**Where the desktop's 380 open rows sit**, by the matrix's own sections — recomputed:

| Section | rows | done | partial | missing | diff |
|---|---|---|---|---|---|
| C. Transcript, streaming, reasoning, tool cards | 82 | 1 | 18 | 57 | 6 |
| A. The Electron shell (`apps/desktop/**`) | 70 | 0 | 9 | 45 | 16 |
| D. Keyboard, settings, themes, localisation, account, models | 68 | 1 | 8 | 54 | 5 |
| F. Layout, sidebars, file tree, terminal, browser, workspaces, sessions | 65 | 0 | 21 | 39 | 5 |
| E. Command palette, quick open, composer, connection state, design system | 54 | 2 | 14 | 33 | 5 |
| G. Approvals, questions, schedule, jobs, goal, plan, subagents, deliverables | 50 | 3 | 10 | 37 | 0 |
| H. Un-numbered cited rows | 30 | 1 | 2 | 22 | 5 |
| B. Approvals and permission presets | 14 | 3 | 6 | 5 | 0 |

**That table is not a work plan, and treating it as one would waste months.** Section A is 70 rows
about Electron: native context menus, an Application menu with *Check for Updates* and *Reload
Page*, caption-menubar roving tabindex, a feedback payload carrying `platform`/`cpu`/`memory_gib`.
Only 16 of those 70 are marked `deliberately different`; **45 are filed as `missing`**, which makes
the headline number look like a product gap when much of it is a carrier gap. The opposite error is
also in there: D-007/D-008 (a context menu in an editable field) and D-031 (*"every ordinary quit
asks the Host what it would interrupt"*) are real desktop behaviours, and the second one was only
closed on 2026-10-05 by M316.

### 3.2 What a user of this product would notice is missing

**The window** (verified against the matrices and the code):

- **No session verbs.** `/new`, `/resume`, `/fork`, `/clone`, `/name`, `/import`, `/export`,
  `/branch`, `/tree` are all refused by name. A reader cannot start, rename, fork or resume a
  session from the desktop.
- **No permission-preset surface**, so the one control that could raise privilege has no risk gate
  to sit behind (rows D-080–D-082, I-121, I-131).
- **No context menu in any editable field** — no cut/copy/paste/select-all by mouse (D-007, D-008).
- **Nothing has ever announced it.** A screen reader has never been run against this window, on this
  machine or any other. `audit/evidence/desktop-reader-walk-2026-10-05b.txt` says of itself: *"This
  is the script a reader would be handed, not a recording of one speaking."*
- **It cannot be seen.** *"No wgpu backend presents on this machine"*
  ([`../../crates/epsilon-desktop/src/frame_raster.rs`](../../crates/epsilon-desktop/src/frame_raster.rs),
  [`../HANDOFF.md`](../HANDOFF.md) §4). Every desktop pixel claim comes from a **test-only** offscreen
  rasteriser (`#[ignore]`d, needs a GPU adapter), not from the production present path.

**The terminal** (from the matrix's open rows; 232 of 282 open rows are user-noticeable by a keyword
rule this review applied, so treat that as an upper bound):

- The model picker: ranking order (P-250), unsaved/unavailable marking and a struck-through
  `[unavailable]` (P-260), `Ctrl+O`/`Shift+Ctrl+O` filter cycling (P-294).
- Deletion prefers the system trash and says which happened (P-237) — it does not.
- OSC 133 prompt zones around assistant/user blocks (P-132).
- Settings: 20 of 22 rows open; themes: 14 of 18.
- Pi's extension-renderer machinery (registered renderers, `customType`, error boxes) — 13 rows,
  most of which this project may not want.

### 3.3 What is missing that is not in the matrices at all

| Missing thing | Evidence |
|---|---|
| A shippable artifact | One tag (`v0.1.0`); install is `cargo install --path crates/epsilon-cli`; no published binary. Nothing a person can download and run. |
| A credential store | `AGENTS.md` §6 requires the OS credential store. Provider keys **and OAuth refresh tokens** are written in plaintext to `<config dir>/auth.json`. `keyring`, `wincred`, `service` and `keychain` appear **nowhere** in `crates/**/*.rs`. On Windows the `0600` is unix-only ([`auth.rs:775-780`](../../crates/epsilon-llm/src/auth.rs)), by the code's own admission: *"Windows relies on the user-profile ACL."* |
| A binding recorder | `keybindings.json` is read by both front ends since M315, and **nothing writes it** — no atomic write, no `0600`, no recorder. |
| A check that can see code-span paths | `check-links` matches markdown links only. **36 repo-relative paths written in backticks resolve to nothing**, including `audit/04-roadmap.md` in [`../plans/README.md`](../plans/README.md). |
| A green battery that is *committed* | The only log describing the current tree lived in `target/`, which `AGENTS.md` calls disposable. This review copied it to `audit/evidence/`. |

---

## 4. Flaws this review found that the project had not written down

Each was verified first-hand against the file named; none is a repetition of a recorded issue.

**F1 — `STATUS.md`'s parity table is wrong, in the document a reviewer reads first.** *(high)*
It published the DSH matrix as `433 rows: 11 done / 85 partial / 295 missing / 42`. Recounted, the
matrix holds **11 / 88 / 292 / 42**, which is what the matrix's own total line says. Two of four
columns are off by three. The Pi row (339 / 39 / 121 / 161 / 18) is exact.
*Evidence:* row parse of [`../parity/dsh-desktop.md`](../parity/dsh-desktop.md) with `\|` un-escaped
before splitting (three rows put an escaped pipe inside the `behaviour` cell and mis-parse without
it); corrected in `STATUS.md` by this review.

**F2 — the cited "latest green battery" describes a tree that no longer exists.** *(high)*
`STATUS.md` cited `gate-m315b` (16:29:58) as *"the tree this file describes"*, but
[`../../crates/epsilon-desktop/src/app.rs`](../../crates/epsilon-desktop/src/app.rs) was edited at
17:36:19 — after it. The log that describes the tree is `gate-m316d` (19:07:54, 2,088 passed), which
sat in `target/` with nothing pointing at it.
*Evidence:* mtimes of both logs against the newest `.rs`; `test result:` sums recomputed for both.

**F3 — four milestones exist only on disk, and HEAD is not the product.** *(high)*
M313–M316 are uncommitted: 18 modified tracked files (+2,397/−320) and 48 untracked entries
(19 and 51 after this review's own documentation edits), including all of
[`../parity/`](../parity/README.md) and both new keybinding files. A fresh clone
does not contain the parity matrices, the rebindable desktop shortcuts, or the window's quit guard,
and its `CHANGELOG.md` describes work its tree does not have.
*Evidence:* `git status --porcelain -uall`, `git diff --shortstat`.

**F4 — the parity matrices already carry 11 stale citations, from a refactor made the same day.**
*(high)* Eleven rows cite `crates/epsilon-tui/src/keybindings.rs` at line numbers that no longer
hold binding declarations: M315 moved the table to `epsilon-contracts`. P-002 cites a **blank line**
(`:149`), P-027 cites a blank line (`:37`), L-000/P-110/P-228 cite `modifiers: KeyModifiers,`
(`:27`). The real home is [`../../crates/epsilon-contracts/src/keybind.rs`](../../crates/epsilon-contracts/src/keybind.rs)
(`TERMINAL`, `:242-365`).
*Evidence:* printed the cited lines from both files; verified `app.exit` at `keybind.rs:252`,
`app.editor.external` at `:364`.

**F5 — two `done` rows whose evidence does not establish them.** *(high)*
P-252 claims *"entering a query resets the selection to row 0"* and names
`two_stage_picker_descends_and_applies_model`; that test
(`crates/epsilon-tui/src/app/tests.rs:2194-2213`) never types a query and never asserts
`picker_index`. The tree's only `picker_index` assertion (`tests.rs:195`) asserts it is
**unchanged**, the opposite scenario. P-313 claims a badge column, citing two lines that are a title
string and a hint row; the badge is `panels.rs:281-292`. The code is right in both cases; the
`verified` column is not — which is exactly what [`../parity/README.md`](../parity/README.md) says
`done` must not mean.
*Evidence:* read both tests and both cited sites.

**F6 — the binding-id count is 23, not 24.** *(medium)*
Rows L-000 and P-110 both say *"24 ids"* / *"24 ids, every one with a `keybindings.matches(...)`
arm"*. Parsed from `keybind.rs:242-365`: **23** entries (19 `app.*`, 4 `tui.*`), none with an empty
chord list. L-001's stated mechanism is also wrong: `app.transcript.halfUp`/`halfDown` are read by
handlers (`keys.rs:1686`, `:1691`) and are simply **absent from the table**.
*Evidence:* entry count over the `TERMINAL` array.

**F7 — the completeness audit's number does not reproduce.** *(medium)*
`STATUS.md` records *"`crates/**/*.rs` returns 71 matches"* for
`TODO|FIXME|XXX|unimplemented!|todo!|not implemented|coming soon|stub|placeholder`, and concludes
every one is legitimate. Using that exact pattern I measure **97 matching lines in the working tree
and 93 at HEAD** — so the drift is not explained by the uncommitted work either. The *conclusion*
may still hold (the hits I read are `placeholder` in UI strings, `is_placeholder_key`, and a
`TODO_BAD` test fixture); the **count** is a measurement that does not reproduce. I found **one**
sharp marker in the whole tree, and it is a string inside a test fixture.
*Evidence:* line counts by pattern over `crates/**/*.rs`, working tree and `git show HEAD:<path>`.

**F8 — the link gate cannot see 36 dead paths.** *(medium)*
`check-links.py` matches markdown links only. Paths written in backticks are invisible to it. **36
repo-relative code-span paths resolve neither from the repository root nor from the file that names
them**: 21 in `docs/milestones/`, 5 in `docs/design/`, 4 at the docs root, 4 in `docs/plans/`,
2 in `docs/research/`. Live examples: [`../plans/README.md`](../plans/README.md) sends a reader to
`audit/04-roadmap.md` (the file is `audit/history/04-roadmap.md` — and `check-links` deliberately
excludes `audit/history`, so nothing else would catch it either);
[`../AGENTS-PROPOSALS.md`](../AGENTS-PROPOSALS.md) cites `gate-base-2026-10-03.log` for a battery
whose file is `.txt`.
*Evidence:* extracted every backticked `*.md|rs|py|ps1|sh|json|toml|log|txt` span under `docs/` and
tested both resolutions.
*Correction made while writing this:* one of the 36 is **not** a defect — `STATUS.md:1866` names
`scripts/desktop-probe.py` inside a roadmap item, i.e. a file that is *proposed* and does not exist
yet. I read the surrounding lines before claiming it, and a future checker needs a convention for
proposals (an explicit marker, or an allow-list) rather than treating every absent path as broken.

**F9 — the citation checker still checks two-thirds of its citations against the wrong tree.**
*(medium)*
`scripts/check-reference-citations.py` prints, in its own words: *"2 named version(s) are not
vendored (pi-0.84.2, pi-0.85.1). Their citations are checked against pi-1.0.0, which is NOT the tree
they name. That is a real limitation, not a pass."* In tracked files, `pi-0.85.1` occurs **113**
times and `pi-0.84.2` **81** times, against **34** for the vendored `pi-1.0.0`. M314 re-pointed 219
shifted citations; the version mismatch underneath is still open.
*Evidence:* the checker's own output; occurrence count over `git ls-files`.

**F10 — "one command registry" is true of the data and not of the dispatch.** *(medium)*
`COMMAND_HEADS` holds **37** rows and the desktop's `SUPPORTED` holds **7** — a real improvement on
the 3 it answered before M312. But `SUPPORTED` and the match arms remain two hand-maintained
statements of one fact, joined by a test rather than by construction, as the code and
[`../plans/024-finishing-the-objective.md`](../plans/024-finishing-the-objective.md) §5 both say.
Nothing prevents the 8th head from being answered without being declared.
*Evidence:* parsed both arrays; read the residual note in `commands.rs`.

**F11 — the window's own evidence comes from a path no user runs.** *(medium)*
`frame_raster.rs` is `mod frame_raster;` under a test-only gate in `lib.rs`, and its `#[ignore]`d
tests need a GPU adapter. So the claim *"the window's pixels, rasterised offscreen, are not blank"*
is a claim about draw output on a non-production backend. The project says so; the risk is that a
reader treats the twelve rendered PNGs as evidence the product displays.
*Evidence:* `lib.rs:35,59-63`; the `#[ignore]` reasons in `app.rs:7442,8033`.

**F12 — `STATUS.md`'s "open rows" list was stale within hours of being written.** *(low)*
Its parity section lists five top open rows and says two were closed by M314. Row 5 — *"quitting
with work in flight ends the process"* — was closed by M316 (`answer_close_request`,
`app.rs:5064`, six tests and a negative control) **after** that list was written. Three of the five
remain, not five.
*Evidence:* `answer_close_request` exists and is called from `paint` at `app.rs:2751`.

---

## 5. Why it failed — structural causes, not effort

Ranked by how much each explains. `FACT` = measured here or in the artefact named; `INFERENCE` = my
reading.

**RC1 — the objective was never turned into a unit of work.** *(FACT)*
The project's own phase log records the moment: *"A native desktop IDE and a half-built shared
command registry **already existed**, which changed the plan from 'build' to 'finish and unify'"*,
and the task was restated as *"Evolve Epsilon into one coherent agent harness with two front ends on
a single core"*. By then the tree already held ~125k lines of source. From that point the unit of
account was **the defect**, not **the distance to the owner's two ports** — and the owner's sentence
was never the thing being delivered.

**RC2 — there was no falsifiable definition of done, and the only measuring instrument arrived last.**
*(FACT)* `AGENTS.md` §13 defines done as a green battery, tests, docs, credits and a current status
— all true of this tree — and says nothing about port completeness. The parity matrices are the
instrument that could answer the objective's question, and `STATUS.md` says of them: *"the
objective's own deliverable and **it did not exist** until now"* — day three of three. They are also
**untracked in git**. For the whole build, no artefact could answer *"is it finished?"*, so effort
followed ranked defect lists instead.

**RC3 — the stated target was incoherent by construction.** *(FACT)*
Two references that *"do not want the same thing"*, one of which the repository's own invariant
forbids reproducing (`AGENTS.md` §2.3: no webview). Section A of the DSH matrix alone is 70 rows
about Electron. **Part of the denominator was unreachable before the first line was written**, and
that ceiling was quantified only at the end.

**RC4 — the baseline moved underneath the port.** *(FACT)*
Citations name three Pi versions (`pi-0.85.1` 113×, `pi-0.84.2` 81×, `pi-1.0.0` 34×) and four DSH
versions, of which exactly one of each is vendored — the DSH tree arriving the morning the objective
began. M314 spent a milestone re-pointing 219 shifted citations. Maintenance of the evidence base
consumed work that the port needed.

**RC5 — verification and documentation outgrew the product.** *(FACT)*
**≈52% of all commits touch no file under `crates/`** (measured: 373 of 712). `audit/` is 42.9 MB
across 642 files, 614 of them under `audit/evidence/`, against 6.70 MB of Rust; **312 files are named
`gate-*`**; M312 cost six batteries and M313 seven. The
discipline is real and it found real defects (see §6) — but in the objective's window, the desktop's
product surface moved from 3 to 7 of 37 command heads.

**RC6 — the project's own rules made the endgame documentation-only.** *(FACT + INFERENCE)*
The budget rule was *"new-feature freeze at ~2/3 of the budget; after that only fixes, docs,
verification, cleanup"*, and *"a working, verified product beats breadth."* Both are good rules for
a shipping project and, combined with §5's *"deletion beats addition"* and the proposal that *"a
recorded gap is not a task"*, they mean that **the final third could not add the surface the parity
gap was made of**. The endgame was structurally unable to close the objective; it could only
document it more precisely. The 453 open rows are the result. *(INFERENCE: that this was a
predictable consequence rather than a coincidence.)*

**RC7 — scope was narrowed unilaterally and never ratified.** *(FACT)*
`docs/parity/README.md` redefines the objective's central word — *"'Port' here means behavioural
parity … not a rebranded copy of the pixels"* — and 60 rows are declared `deliberately different`.
Each cut is documented locally and none is ratified by the owner. The owner's completion test never
changed while the project's did. This is the single cheapest thing to fix, and
[`../plans/025-completion-plan.md`](../plans/025-completion-plan.md) §1 is where.

**RC8 — the last mile needed things an agent cannot supply alone.** *(FACT)*
A human screen-reader session; a keyboard-only pass by a person; a GPU that presents; an owner's
decision on a credential store that needs a new dependency and an ADR. Four of the largest remaining
items are blocked on a human or on hardware, and none of them was scheduled as such.

---

## 6. What is genuinely strong

The plan must not throw these away, so they are listed with the same rigour as the flaws.

- **The one-core invariant actually holds.** `build_runtime`/`resolve_launch` are called only from
  the front ends and the runtime's own tests; no crate depends on `epsilon-cli`. This is the claim
  most projects make and this one can be checked in one grep.
- **A real suite.** ≈2,120 declared test functions, 2,088 passing in the battery that describes the
  tree, 78 test binaries, and integration tests for failure paths rather than only happy paths.
- **Defects found that a green suite could not see**, each with causation evidence and, in several
  cases, a negative control that reprints the defect as a value: `/restore` deleting a file the user
  had edited after the agent created it (M298); a session's "always allow" inherited by every
  subagent (M300); an operator's `before_tool_call` hook never reaching subagent tool calls (M309);
  a project config spawning arbitrary binaries at startup, ungated (M276).
- **The parity matrices.** 772 paired rows from 869 source-cited inventory rows, both sides cited,
  `done` defined as *exercised, not written*, and an explicit refusal to dress a gap up as a
  decision. This is the instrument the objective needed. It arrived late and is untracked; it is
  also the best thing in the repository.
- **Visual and platform verification that mostly is not hand-waving**: an offscreen 1440×900 raster
  with a measured ink fraction, 53 accessibility nodes read back through Windows UI Automation from
  another process, a third palette with all 21 roles measured, and a frame-level test suite for the
  TUI that made two of this review's own first readings wrong.
- **Machine faults diagnosed instead of worked around**: the `D8050`/`c1.dll` build-directory
  finding, the filename-bound antivirus block, the `CARGO_PROFILE_TEST_DEBUG` accumulation. Each is
  written down with the probe that proved it.
- **Honesty about its own drift**, including the corrections in plan 024's header. That habit is why
  this review could be written at all: almost everything here was checkable.

---

## 7. What success requires — the definition of done that was missing

The objective cannot be finished until it can be *decided*. A definition that would have worked, and
that the plan is built on:

1. **The product runs.** `cargo install --path crates/epsilon-cli` succeeds on a clean machine; the
   terminal opens and completes a turn against a real provider; **the desktop window opens and draws
   on the owner's machine**, or the owner states in writing that offscreen-only rendering is
   acceptable. *Today: unverified on this machine — no wgpu backend presents.*
2. **The port is scored by the matrices, not by prose.** Every row is `done`, `partial`, or
   `deliberately different` **with a ratified reason**. `missing` reaches zero for an owner-chosen
   *must-have* subset, and the subset is named in advance. *Today: 453 missing.*
3. **One command registry by construction.** A command answered by a front end is answered because
   it is declared, not because two lists agree. *Today: 7 of 37, joined by a test.*
4. **The evidence describes the tree.** The battery that describes the current tree is committed and
   cited from `STATUS.md`; no milestone is uncommitted; the parity matrices are in git.
   *Today: four milestones on disk only.*
5. **Accessibility is witnessed.** One transcript of a real screen reader against the window; one
   human keyboard-only pass of both front ends. *Today: neither has ever happened.*
6. **No plaintext credential on disk**, or `AGENTS.md` §6 amended by the owner to say what is
   actually enforced, on which platforms. *Today: the rule is violated and the Windows case is
   unhardened.*
7. **A release.** A tagged, installable artifact with the front ends in it.

---

## 8. Limits of this review

- **No cargo command was run and no binary was launched.** Every statement about behaviour is a
  reading of source, of a log, or of a frame this repository already had. "Exercised" is therefore
  inherited from the project's own battery, not re-established here.
- **The DSH matrix was not adversarially sampled row by row.** I recounted all 433 rows and read
  section A and the command rows closely; the Pi matrix was sampled at every 30th row by a separate
  read-only pass and the defects it found were re-verified by me. A full row-by-row audit of 772
  rows remains unperformed.
- **The `user-noticeable` split in §3.2 is a keyword rule I wrote**, not a measurement of user
  impact; it is an upper bound and the rule is stated where it is used.
- **Two review passes died mid-flight** and their scopes — the security sweep beyond the credential
  finding, and the frame-by-frame UX reading — are therefore covered only by what is written here:
  the credential contradiction, the frame readings in §3.2, and the accessibility gap.
- **This review changed no source.** The corrections it made are the two numbers in `STATUS.md`, the
  pointer to the current battery, two proposals, and the plan. Everything else is a claim for the
  next session to act on.
