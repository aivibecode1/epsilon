# Plan 025 — completing the objective

**What this is.** The objective-level completion plan: what has to be true for
*"a unique harness that combines Pi and DSH, with the terminal as a port of Pi and the desktop as a
port of DSH Desktop"* to be **finished or formally rescoped**, in what order, and how each step is
proved. It was written on 2026-10-06 from
[the independent review](../reviews/2026-10-06-objective-review.md), which is where every number
here comes from.

**How it relates to the plan that already exists.**

| Document | Answers |
|---|---|
| [`024-finishing-the-objective.md`](024-finishing-the-objective.md) | *What are the known defects, in value order, and what makes each hard?* Its eight items are still the item-level truth; **item 2 is landed (M312)** and item 5 is partly landed (M312–M315). |
| **This plan** | *What has to happen for the objective to be decidable and then decided?* It sequences 024's items inside a bigger question, and adds the work 024 does not contain: preservation, the scoreboard, the release, and the owner's scope decision. |
| [`../STATUS.md`](../STATUS.md) | What is true today. |

**Nothing in this plan is authorized until §1 is answered.** The single largest cause of the
objective not being met was that it was never operationalised (review RC2); starting work without
that answer would repeat it.

Read [`../../AGENTS.md`](../../AGENTS.md) before the first change. Read [`../HANDOFF.md`](../HANDOFF.md)
before the first battery.

---

## 1. The decision the owner has to make first

Three coherent readings of the objective. **Pick one in writing.** Everything below assumes **(a)**.

**(a) Ratify the native harness and name the must-have subset — RECOMMENDED.**
Keep *"one core, two front ends"*, drop the phrase *"direct port"* from the goal, and have the owner
name the rows that must be `done` for each front end. The matrices become the scoreboard, the
must-have list becomes the target, and the rest is explicitly out of scope. This is what the project
has effectively already built; it needs the owner's signature, not more code.
*Cost: a decision, then the work in §2. Result: a finished, defensible product.*

**(b) Actually port DSH Desktop.** Not achievable inside `AGENTS.md`: §2.3 forbids a webview, and
the matrix's section A is 70 rows of Electron — context menus, an Application menu, caption
windows, an update channel. Delivering it means either abandoning the native-window invariant or
accepting that ~45 rows stay `missing` forever. If this is the goal, the invariant has to change
first, deliberately and in an ADR.
*Cost: an ADR that reverses a founding decision, then months. Result: a different product.*

**(c) Freeze, release, and stop.** Tag what exists as a **preview**: the terminal is a capable
harness, the desktop is a native IDE with seven verbs. Publish an artifact, declare the parity
matrices as the honest gap list, and stop. *Cost: days. Result: something real, honestly labelled,
that a user can install — and the option to resume at (a) later.*

**Whichever is chosen, do §2.A first.** Four milestones exist only on disk; if the machine dies
today, the parity matrices, the rebindable desktop shortcuts and the window's quit guard are gone.

---

## 2. Workstreams

Each has: why it exists, what to do, **done-when** (falsifiable), the evidence to leave behind, and
the trap that makes it longer than it looks. Sizes: S ≈ an hour, M ≈ a session, L ≈ a day or more.

### A. Land the work that exists only on disk — **P0, do first** (S)

**Why.** M313–M316 are uncommitted: 18 modified tracked files (+2,397/−320) and 48 untracked
entries, including all of [`../parity/`](../parity/README.md). `HEAD` does not contain the
objective's own deliverable, and a fresh clone builds a product its `CHANGELOG.md` does not
describe (review F3).

**What.** Re-run the battery against the frozen tree under a **new label**; copy the log into
`audit/evidence/`; commit in coherent slices (the parity matrices as one commit, M314/M315/M316 as
three) with the battery named in each message; update `STATUS.md`'s "Latest green battery" line to
the log that describes the tree.

**Done when.** `git status` is clean apart from regenerable output; the battery cited by `STATUS.md`
is committed under `audit/evidence/` and postdates every source file; `git show HEAD:docs/parity/pi-tui.md`
succeeds.

**Evidence.** The gate log, the `git show` output, and the commit hashes.

**Trap.** The last battery on this tree (`gate-m316d`, 2,088 passed) is already green and its log is
now preserved at [`../../audit/evidence/gate-m316d-2026-10-05.txt`](../../audit/evidence/gate-m316d-2026-10-05.txt)
— but a *fresh* battery is still required before committing, because a commit is a claim about a
tree and the last one was taken before the review's own doc edits. Budget ~45 minutes with a warm
build tree, more if `%TEMP%\epsilon-target` has been pruned.

### B. Make the scoreboard trustworthy, then use it — **P0** (M)

**Why.** The matrices are the only instrument that can decide the objective, and they already carry
defects: 11 stale `ours` citations from M315's own refactor (review F4), two `done` rows whose
evidence does not establish them (F5), a wrong id count (F6), and a `STATUS.md` table that
misstates the DSH matrix (F1). A scoreboard nobody can trust is worse than none, because it is
quoted.

**What.**
1. Re-point the 11 `keybindings.rs` citations at [`../../crates/epsilon-contracts/src/keybind.rs`](../../crates/epsilon-contracts/src/keybind.rs)
   (`TERMINAL` is `:242-365`); re-derive rather than guess, and re-derive the count of ids (23).
2. Fix P-252 and P-313: either add the test the row claims, or downgrade the row to `partial` and
   say which half is unproven. **Adding the test is preferred** — P-252's behaviour exists in
   `keys.rs:1322-1326` and is simply unpinned.
3. Add `scripts/check-parity-citations.py` to the battery: every `ours` and `reference` citation in
   both matrices must resolve to a file and an in-range line. This is the check whose absence let F4
   happen; it is cheap and it is the same class as `check-links`.
4. Adopt the review's §7 as the project's definition of done for the objective, in
   [`../STATUS.md`](../STATUS.md) where it can move.

**Done when.** The new checker is a `gate.ps1` leg and passes; `STATUS.md`'s parity table matches the
matrices' own totals; no row marked `done` names a test that does not exercise it.

**Evidence.** The checker's output in a gate log; a diff of the corrected rows.

**Trap.** The `ours` column is 320 citations in one file and ~400 in the other. Do not hand-edit;
write the checker, let it print the failures, then fix what it names.

### C. Bring the window up to a usable floor — **P1** (L)

**Why.** The desktop is half the objective and answers 20 of 37 commands (review §3.2, plus `/help` in round 4, `/new` in round 6, `/resume` in round 7, `/fork` + `/clone` in round 8, `/export` in round 15, `/model` in round 20, `/think` in round 30, `/settings` in round 33, `/login` + `/logout` in round 38 and `/hotkeys` + `/copy` in round 40). Four gaps are
the ones a user hits first, and three of them are already itemised in the matrices:

| Gap | Rows | Where |
|---|---|---|
| Session verbs in the window (`/new`, `/resume`, `/fork`, `/clone`, `/name`) | D-1xx, U-0xx | [`../../crates/epsilon-desktop/src/commands.rs`](../../crates/epsilon-desktop/src/commands.rs) |
| A permission-preset surface, so a privilege-raising control has a gate to sit behind | D-080–D-082, I-121, I-131 | `epsilon-safety` + the window |
| A context menu in editable fields (cut/copy/paste/select-all) | D-007, D-008 | the composer and the editor |
| The quit dialog's appearance (M316 tests its behaviour; the rasteriser blanks on it) | — | `app.rs:5141-5150` |

**What.** Take them in that order. For each: RED test through the real draw path, then the
capability, then a rendered frame in `audit/evidence/desktop-frames-<date>/` that a person has
**looked at**.

**Done when.** The window answers every session verb the terminal answers (or refuses it with a
reason that names what is missing), an approval cannot be raised past without a gate, and the quit
dialog has a frame someone has read.

**Evidence.** Frame images plus the tests that drive them; the updated `SUPPORTED` count.

**Trap.** Do not start with the scroll bar or the `SCROLLBAR` role (024 items 5–6). They are
accessibility polish behind a bigger structural question and they have a documented trap each.

### D. Command coverage by construction — **P1** (M)

**Why.** `SUPPORTED` (7 entries) and the match arms are two statements of one fact joined by a test
(review F10, 024 §5). Adding arms is the easy part and it is the part that does not scale.

**What.** Derive the declared capability from the dispatch: one table in `epsilon-contracts` where a
head is answered-by-this-front-end as data, and a `window_runs` that reads it rather than
restating it. The registry module's own doc already says the rows are *"four hand-maintained
tables"* — this is the move that comment is waiting for.

**Done when.** Deleting an arm makes the declaration change without touching a second list; a test
that adds a head to the shared registry fails until the front end declares what it does with it.

**Evidence.** The test that fails for that reason, and the diff that removes the second list.

**Trap.** `Models`/`Model(String)` are two variants for one head — that is why the denominator is 37
rows and 38 variants, and the earlier "3 of 38" figure was wrong.

### E. Close the credential contradiction — **P1** (L, needs an ADR + a dependency)

**Why.** `AGENTS.md` §6 says *"Never write a credential to disk in plaintext. Secrets belong in the
OS credential store."* The product writes API keys **and OAuth refresh tokens** to
`<config dir>/auth.json`; `keyring` is in neither `crates/**/*.rs` nor `Cargo.lock`; and the `0600`
is unix-only by the code's own comment ([`../../crates/epsilon-llm/src/auth.rs`](../../crates/epsilon-llm/src/auth.rs)).
This is the only place where a locked rule and the tree disagree outright.

**What.** Pick one, in writing, and do it:
- **Preferred:** an ADR for the keychain crate, `auth.json` kept as import/export for CI, migrate on
  first read, and the file deleted after migration. Plus an explicit Windows ACL (or a documented
  decision that the profile ACL is enough).
- **Interim:** amend §6 through [`../AGENTS-PROPOSALS.md`](../AGENTS-PROPOSALS.md) to state what is
  actually enforced, per platform, so the locked file stops describing a product that does not exist.

**Done when.** Either no plaintext credential is on disk after a login, or §6 says what the tree
does and names the platform gap.

**Evidence.** A test that reads the file back and fails when the key is present in plaintext; the ADR.

**Trap.** Proposal 7 in [`../AGENTS-PROPOSALS.md`](../AGENTS-PROPOSALS.md) already lays this out and
already recommends the same option. Do not re-litigate it; **answer** it.

### F. The terminal's user-visible Pi rows — **P2** (L)

**Why.** 232 of the terminal's 282 open rows are user-noticeable by keyword rule (an upper bound).
The ones worth doing first are the ones a person would notice in a week of use, not the ones with
the largest counts:

| Behaviour | Row | Why it earns its place |
|---|---|---|
| Deletion prefers the system trash and says which happened | P-237 | Data-loss-adjacent, and `AGENTS.md` §6 is the project's own standard |
| The model picker ranks, marks `[unavailable]` and strikes through | P-250, P-260 | The picker is the most-used surface in the TUI |
| `Ctrl+O` / `Shift+Ctrl+O` cycle the transcript filter | P-294 | Cheap, and the key is already reserved |
| Settings and themes (20 and 14 rows open) | §10, §9 | The largest coherent block of user-visible gaps |

**Done when.** Each landed row moves to `done` in the matrix with a frame-level test, and the frame
has been read at 80×24 and 100×30 (`EPSILON_FRAME_DIR`).

**Evidence.** Updated matrix rows; frames under `audit/evidence/tui-frames-<date>/`.

**Trap.** Pi's *extension-renderer* machinery (13 rows) and its decorative rows (9) are not worth
porting and should be moved to `deliberately different` with the reason written down — which is a
change to the matrix, not to the product.

### G. Evidence integrity — **P2** (M)

**Why.** Three measured holes: `check-links` cannot see 36 dead code-span paths (F8); the citation
checker compares two-thirds of its citations against a tree they do not name (F9); and the
completeness audit's count does not reproduce (F7). Each is the same failure mode the repository
already warns about: *a check that cannot see a thing is indistinguishable from a check that found
nothing.*

**What.**
1. Extend `check-links.py` (or add a sibling) to validate backticked repo-relative paths that begin
   with a known top-level directory. Fix the 36 — most are in `docs/milestones/`, where the honest
   fix is a note that the file moved, not a rewrite.
2. Re-point the `pi-0.84.2`/`pi-0.85.1` citations at the vendored `pi-1.0.0`, or record per
   citation why the version it names is the right one. The checker already prints the limitation.
3. Re-measure the completeness audit and correct the number in `STATUS.md` (measured: 97 lines in
   the working tree, 93 at `HEAD`, one sharp marker — a test fixture).
4. Make evidence preservation part of the battery rather than a habit: `gate.ps1` should copy its
   log to `audit/evidence/` itself, so no green battery can live only in a disposable directory.

**Done when.** Each check is a gate leg, is red on the defect it names, and is green on the tree.

**Evidence.** The gate log; the negative control for each new check.

### H. Witness the accessibility — **P1, needs a human** (S of human time, schedule it first)

**Why.** It is the largest gap in the objective's own definition of done and the only item that
cannot be closed by an agent: a screen reader has **never** announced this window, and no person has
ever done a keyboard-only pass of either front end. UIA proves the platform *exposes* names and
roles; it does not prove anything is announced.

**What.** The procedure is already written in [`024`](024-finishing-the-objective.md) §1: launch the
window, start Narrator, walk the landmarks, **record what it actually announces verbatim** next to
`audit/evidence/desktop-uia-tree-2026-10-05b.txt`, fix what it gets wrong, and pin the wording in a
test. Then the same person does a keyboard-only pass of the terminal and the window.

**Done when.** A transcript of a real reader session and a human keyboard-only pass are both in
`audit/evidence/`, and every defect they found has a test that fails without its fix.

**Evidence.** The transcripts. Nothing else substitutes.

**Trap.** This is the item that has been deferred most often because an agent cannot do it. **Book
the human first**; everything else in this plan can proceed while it waits.

### I. Release — **P1** (M)

**Why.** There is one tag (`v0.1.0`), no published artifact, and install is
`cargo install --path crates/epsilon-cli`. A product nobody can install is not a delivered port.

**What.** Version the workspace, tag, build the two binaries (terminal + desktop), attach them to a
release with the SHA-256 of each, and put the honest scope in the release notes: what the terminal
is, what the window is, and a link to the parity matrices as the gap list. `README.md` and
[`../START-HERE.md`](../START-HERE.md) must be true of the artifact.

**Done when.** A person who has never seen the repository can install and run both front ends from
the release notes alone, and the commands in those notes were run.

**Evidence.** The release artifacts, their hashes, and the transcript of a clean install.

**Trap.** Do not tag while §2.A is open — the release would not contain the parity matrices.

---

## 3. Sequence, and the constraints that shape it

| Order | Workstream | Why here |
|---|---|---|
| 1 | **H (book the human)** | Longest lead time, zero dependency on anything else. Book it before writing code. |
| 2 | **A (land the tree)** | Everything after it is a claim about a tree nobody else can see. |
| 3 | **B (scoreboard)** | The target must be measurable before it is aimed at. |
| 4 | **§1 decision** | The owner's answer decides how much of C, F and I is in scope at all. |
| 5 | **E (credentials)** | One rule, one product, one decision; independent of the front ends. |
| 6 | **D then C (desktop)** | D makes C cheap: every verb added after it is declared once. |
| 7 | **F (terminal)** | Independent of the desktop; can run in parallel on a second writer. |
| 8 | **G (evidence)** | Fold each item in as the workstream that needs it lands; do not batch. |
| 9 | **I (release)** | Last, and only when the release notes are true. |

**The machine rules still apply and they are the real schedule risk.** One build owner at a time;
`CARGO_BUILD_JOBS=1`; build outside the repository; never re-use a battery label; never edit source
while a battery runs. A single battery is ~40 minutes with a warm tree. **Two workstreams that both
need a battery cannot run in the same hour** — plan for that, and prefer one writer on the tree at a
time (the objective was worked by two sessions at once, and `STATUS.md` records the confusion it
caused).

**Revise the freeze rule.** *"New-feature freeze at 2/3 of the budget"* is what made the last third
documentation-only (review RC6). If this plan is adopted, the freeze applies to **the must-have
subset in §1**, not to the calendar.

---

## 4. Stop doing

- **No new ADR-only milestones.** M310 was documentation of a fix not made; it is honest and it
  cost a round. An ADR belongs with the change it records.
- **No new documentation trees.** `docs/` is 338 files and 46k lines. New prose goes into an
  existing file, or it is a row in a matrix.
- **No milestone logs after M276.** The existing decision to stop them is right; do not restart it.
- **No "recorded gap is not a task" as a reason to skip a must-have.** That proposal exists to
  protect deliberate boundaries, not to freeze the objective's own target list.
- **No fixes to the desktop's presentation.** The offscreen path exists and works; presentation is a
  machine fault. Change the *definition of done* (§1) or the *machine*, not the GPU configuration —
  `AGENTS.md` forbids the third option.
- **No counting without a command.** Every number in this plan came from one; keep that.

---

## 5. Definition of done for this plan

The plan is complete when:

- [ ] §1 is answered in writing by the owner, and the must-have subset is named.
- [ ] `git status` is clean; the battery that describes the tree is committed and cited from
      `STATUS.md`; the parity matrices are in git.
- [ ] `missing` is zero across the must-have subset, and every remaining row is `partial` or
      `deliberately different` with a written reason.
- [ ] The window answers every command head in the must-have subset, declared once.
- [ ] No plaintext credential on disk, or §6 amended and the amendment ratified.
- [ ] A screen-reader transcript and a human keyboard-only pass are in `audit/evidence/`.
- [ ] A tagged release installs and runs on a machine that has never seen this repository.
- [ ] `docs/reviews/2026-10-06-objective-review.md`'s findings F1–F12 are each closed, or each is
      recorded as accepted with a reason.

---

## 6. What could still go wrong

- **The decision in §1 is not made**, and the project optimises the scoreboard instead of the
  product — the exact failure this plan exists to prevent.
- **The battery queue becomes the bottleneck.** Serialise deliberately; batch doc-only changes into
  one battery rather than one per file.
- **The matrices are treated as a to-do list.** Section A of the DSH matrix is 70 rows of Electron;
  porting them is not the objective, and saying so is a scope decision, not laziness.
- **The human items slip again.** They have slipped every round so far, because an agent cannot do
  them. That is a scheduling failure, not a technical one, and it is the single most likely reason
  this plan ends where the last one did.
- **A machine fault eats the round.** Two of M316's four batteries were lost to the environment
  (a pruned build tree, a blocked test binary). Budget for it; it is not a code failure.
