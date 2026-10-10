# Plan 026 — the convergence plan

**What this is.** The re-ranked, falsifiable plan that follows from the investigation sweep of
2026-10-09 ([`../../audit/evidence/investigation-sweep-2026-10-09.md`](../../audit/evidence/investigation-sweep-2026-10-09.md)).
It answers one question: *now that the tree has been audited fresh, what is actually worth
doing, in what order?* It is not a restart of the program — plan 025 did its work, and most of
it landed. This plan says what survived that, what the fresh audit found, and what goes next.

**How it relates to what exists.**

| Document | Status after this plan takes effect |
|---|---|
| [`025-completion-plan.md`](025-completion-plan.md) | **Superseded, not discarded.** Its disposition is §1 below, workstream by workstream. Its standing instructions (one build owner, never re-use a battery label, the human items cannot be closed by an agent) remain in force and are re-stated here only where they bite. |
| [`../parity/pi-tui.md`](../parity/pi-tui.md), [`../parity/dsh-desktop.md`](../parity/dsh-desktop.md) | The scoreboard, unchanged in role. Measured healthy this week: 471 citations resolve, both count checkers exit 0, all 119 test names claimed by `done` rows exist in the tree. |
| [`../STATUS.md`](../STATUS.md) | The record. Every round lands here with its battery. |

---

## 1. What plan 025 left, dispositioned

| Workstream | Disposition | Evidence |
|---|---|---|
| A — land the work that exists only on disk | **Landed.** | Tree is clean at HEAD (`git status` 0 entries at sweep time); matrices are in git. |
| B — make the scoreboard trustworthy | **Landed and still passing.** | `check-parity-citations` is a gate leg; totals re-counted from the matrices this week match `STATUS.md` exactly. |
| C — the window's usable floor | **Landed.** | 36 of 37 heads answered, `/editor` refused by name; presets engine + `/preset` (rounds 66–68). |
| D — command coverage by construction | **Landed.** | Verb rows derived from `SUPPORTED` (round 11). |
| E — the credential contradiction | **CARRIED — still open.** §6 of the locked file and the tree still disagree: credentials sit in `<config dir>/auth.json`; proposal 7 is unanswered. R6 below. |
| F — the terminal's user-visible Pi rows | **Largely landed.** | The rows F named (P-237, P-250, P-260, P-294) and the five-investigation queue all closed by rounds 70–103. What remains is R2 below. |
| G — evidence integrity | **Landed.** | Three more checkers are gate legs; `gate.ps1` copies its own log. |
| H — witness the accessibility | **CARRIED — untouched.** No screen-reader transcript, no human keyboard-only pass, either front end. R5 below, and it is the item that has slipped every round. |
| I — release | **Half-landed.** | Preview install verified (round 73); no tag, no artifact. R7. |

**The one structural re-rank.** Plan 025's own §4 says *"No fixes to the desktop's
presentation"* because presentation was a machine fault. That fault was cut around (offscreen
rasteriser, M305; twelve window states rendered, M313; frames re-rendered per round since).
Presentation work is no longer blocked, so it returns to the plan as R3.

---

## 2. The ranked items

Order is by value, not by size. Each item's done-when is falsifiable; each names the evidence
it leaves and the trap that makes it longer than it looks.

### R1 — The permission gate yields Enter/Escape to the button the reader aimed at (D-073, I-104)

**Why first.** The only finding of the sweep that is a live safety defect with a
wrong-direction failure mode: a focused **Refuse** button receives an Enter that has already
been consumed by the global `approve_pending` and resolves **AllowOnce**
(`crates/epsilon-desktop/src/app.rs:4605`, `:8711`; the consuming take is
`crates/epsilon-desktop/src/keys.rs:225`; the chords are `epsilon-contracts/src/keybind.rs:503-504`).
The outcomes are right and the disambiguation is wrong — the exact thing the reference's row
exists to prevent (`packages/client/ui-approval/src/client/ApprovalPanel.tsx:51`).

**What.** Before the global take, check whether an approval button holds keyboard focus; if it
does, the widget owns Enter/Escape for that frame. The global chords remain the no-focus path.

**Done-when.** A test driving a real unmodified Enter through the real draw path with focus on
Refuse resolves **Deny** and with focus on Allow resolves **Allow**; the no-focus behaviour the
existing tests pin is unchanged; the ask strip's `[enter] allow [esc] refuse` hint still
describes what actually happens. Both matrix rows move — `done` if a frame shows it, `partial`
with the half named if not.

**Evidence.** The RED-first test, its negative control, the battery log, the matrix diff.

**Trap.** The fix must consult the buttons' *real* egui focus ids. If the ask strip draws its
buttons as painter rects rather than widgets, the Enter ownership must move into the widget —
do not paper over it with a second global read.

**Landed, round 105 (battery `port-r105c`, green first time — the disk and PDB machine faults are in the STATUS row).** Both rows are `done`. The buttons are real
widgets, so the fix is the one the trap names: the gate records the buttons' ids as it draws
them (`app.rs:7848-7856`) and the global read stands aside when one holds focus, leaving
egui's own focused-widget Enter handling to activate it
(`egui-0.36.2/src/context.rs:1466-1477` — verified in the vendored source, not assumed).
Pinned by `enter_aimed_at_the_focused_refuse_button_refuses_the_ask`, which asserts the
**decision** (`refused`, and not `allowed`) because both outcomes close the gate; its negative
control — the guard stubbed to `false` — printed `allowed` and a completed run. **The round
also closed the honest-ask half that verification found**: the `ask_is_display_honest` rule
gated the Allow *button* but not the Enter *path*, so a `U+202E` ask could still be approved
from the keyboard (M261's rule, hole in this front end). One pure predicate,
`enter_still_allows`, now carries both rules; the dishonesty half is unit-pinned
(`enter_still_allows_only_for_an_honest_ask_no_button_owns`) because the draw path cannot
produce a hostile ask, and the focused half end-to-end because a focus state cannot be
reached from outside the draw. The no-focus path is unchanged and still pinned by the
pre-existing gate tests.

### R2 — The TUI transcript's remaining partials, as one presentation round

**Why.** The philosophy's central TUI complaint — *presentation, not functionality* — now has
only four rows left standing, and all four are in one area of the screen:

| Row | The half that is open |
|---|---|
| P-058 | No fixed `Error: `/`Warning: ` prefix convention (the `SystemErr` row itself landed, round 97) |
| P-128 | Same prefix half; the errored-reply rendering landed |
| P-054 / P-183 | A compaction rule (`state.rs:1445`) but no summary card and no collapsible card |
| P-121 | No `outputPad` setting |

**What.** Decide the prefix convention (adopt Pi's, or record deliberately different with the
reason) and land the compaction card: collapse to the token line, expand to a header plus
summary.

**Done-when.** Each row is `done` with a frame test at 80×24 and 100×30, or `deliberately
different` with both sides cited.

**Round 106, landed in two halves.** The prefix convention is **decided — deliberately
different**, with both sides cited in the rows (severity already carried four proven ways;
a fifth channel for one fact). The compaction card landed on the **live** path: the wire now
carries the summary and the pre-compaction token count, the row is one expandable-event row
through the card machinery's own selection/nesting/affordance, and the collapsed form hides
the digest whole behind `… N more line (ctrl+x to open)` — the affordance's direction and
singular were wrong on the first look and were fixed by looking. P-054 and P-183 stay
`partial`: **the replay half and the markdown half are named in the row as open** — a resumed
session hydrates from messages while the digest lives in the store's compaction entry. The
chord was the shared `ctrl+x` (the trap's worry resolved: no new binding was needed).

**Trap.** The prefix decision is a *design* decision with a wording cost across nine command
paths — write the convention down before touching the path count. And a card that expands needs
a chord that is not already taken (`/expand` exists; Ctrl+X is cut in text fields).

### R3 — The desktop's design-quality program, continued

**Why.** The owner's standing instruction (rounds 100–103: the window needs a lot of work; make
it good the way DSH-Desktop is good). Four slices landed — raised pane surfaces, the empty
editor offering work, the launch diagnostics as one band, one fact stated once per pane. The
program is the next finding, not a sixth list written in advance.

**What.** Rank the remaining design findings from a fresh pass over the twelve rendered states
(`audit/evidence/desktop-frames-2026-10-05-m313/`) plus the composer/transcript/settings
surfaces the frames do not cover, then land them one round at a time, RED-first, frame-tested.

**Done-when.** Each landed slice has a frame a person read and a STATUS row; the program ends
when a ranked pass finds no finding that is not a recorded deliberate difference.

**Round 107 — the first slice, landed.** The ranked pass read the window's draw code against the
objective's own question (*what requires user attention*) and the top finding was the round-97
defect alive in the other front end: every notice drew `theme.dim` with a blank marker, and the
reducer never read the assistant's `error_message`. Fixed as `RowKind::NoticeFail` — the
`SystemErr` row's window half, the cross the failed tool already owns, the danger colour, the
accessible name — with eleven failure sites routed and the M62/auth-note parity on the terminal's
own predicate. Frame read; pair pinned; control run. **The pass continues:** the next findings are
ranked from the same read (the composer's send affordance and the settings/shortcuts overlays are
the surfaces the twelve frames do not cover).

**Trap.** Decoration is not design. The test for each slice is the information-hierarchy
question from the objective — *what did the agent say, what is it doing, what needs attention* —
never "prettier".

### R4 — S-122: the keybinding recorder

**Why.** The largest single missing desktop surface, and the mechanism it would record into
already exists on both sides (`epsilon-contracts/src/keybind.rs`, the shared `keybindings.json`
loader, the conflict test). Rows behind it: S-122, S-123, S-124, S-125 (write half landed
round 100), S-126.

**Done-when.** A user rebinds a chord by pressing it, the conflict check refuses an occupied
chord naming both commands, and the file write is atomic and survives a restart — all three
pinned through the real draw path.

**Landed, round 108.** All three halves pinned (`the_shortcuts_recorder_captures_a_chord_and_saves_it`,
`the_recorder_refuses_a_chord_another_command_owns_naming_both`,
`escape_cancels_the_recorder_without_writing`), both controls run, frames read. **The trap was
real but the other way round:** the defect found was not the recorder eating its own cancel key
but the *chain order* — the recorder read its keys behind the palette, whose chords are what a
reader rebinds, so recording `ctrl+p` opened the palette. The recorder now owns the keyboard
from the top of the frame's key chain, before even approvals. Remaining in the cluster: S-126's surface — S-124
(restore-all-defaults) landed round 109 on the recorder's file: `ctrl+r` arms, `enter`
applies, `escape` abandons, and the replaced file is kept as `keybindings.json.bak` (§6's
recoverable copy).

**Trap.** A recorder is a modal keyboard owner. It must not eat the Escape that cancels it —
the round-99 confirm-handler defect (frame-consumed `any_key`) is the same shape.

### R5 — The human items (owner action, book first)

**Why.** Unchanged from plan 025 §2.H, and still the most-deferred item in the repository: no
screen reader has ever announced this window (UIA proves the tree reaches the platform; a
reader decides what is *said*), and no dated human keyboard-only pass exists for either front
end. **An agent cannot close this one.** Book the session; everything else proceeds while it
waits.

**Done-when.** A verbatim reader transcript and a dated keyboard-only walkthrough, each with
every defect it found carrying a test that fails without its fix.

**Trap.** Scheduling, not engineering. It slips when it is not booked.

### R6 — The credential contradiction, decided (plan 025 §2.E)

**Why.** The locked §6 says secrets belong in the OS credential store; the tree writes API keys
and OAuth refresh tokens to `auth.json`. One rule, one product — but this needs the owner's
signature and (probably) an ADR plus a dependency decision.

**Done-when.** Either no plaintext credential is on disk after a login, or §6 is amended through
`AGENTS-PROPOSALS.md` to state what the tree does per platform, and ratified.

**Owner decision required.** Do not guess this one.

### R7 — Release, tagged

**Why.** Preview install verified (round 73); no tag, no artifact. The release notes must be
true of the artifact, and the parity matrices are the honest gap list.

**Done-when.** A person who has never seen this repository installs and runs both front ends
from the notes alone, and the commands in the notes were run.

**Trap.** Do not tag while R1 is open.

### R8 — Pinning-quality sampling at scale (the F5 class, done properly)

**Why.** The sweep measured that every `done` row's *named test exists* (119/119). It did not
measure that each test *pins the behaviour its row claims* — the class the 2026-10-06 review
called F5 and found real defects in. The check that exists (citations resolve) cannot see it.

**Done-when.** A sampled audit (every fifth `done` row, both matrices) confirms each named test
moves its row, or downgrades the row with the half named.

**Landed, round 110.** 17 rows sampled (12 of 61 Pi, 5 of 26 DSH), 27 named tests read against
their rows' behaviour cells, **0 downgraded**; the record states what the sample does and does
not prove and names what a continuous version of the check would need. Evidence:
[`../audit/evidence/pinning-sample-2026-10-09.md`](../../audit/evidence/pinning-sample-2026-10-09.md).

---

## 3. Sequence, and the machine rules that shape it

1. **R5 booked first** — longest lead time, zero code dependency.
2. **R1** — safety, small, unblocks the release.
3. **R2** — the last coherent TUI presentation block.
4. **R3** — the desktop design program, ranked pass then one round per finding.
5. **R4, R6, R7** in any order once their owners are satisfied (R6 needs the owner; R7 after R1).
6. **R8** whenever the battery queue has slack.

**Machine rules, restated because they are the real schedule risk.** One build owner at a
time; `CARGO_BUILD_JOBS=1`; build outside the repository; a battery gets a fresh label and a
timeout above 600 s; never edit source while a battery runs. A warm-tree battery is ~40
minutes. **Two workstreams that both need a battery cannot run in the same hour.**

## 4. Definition of done for this plan

- [x] R1 landed with both matrix rows moved and the ask's hint still truthful. (Round 105: `port-r105c`.)
- [x] R4 landed: the recorder, its conflict refusal, the writer's caller, and restore-all-defaults. (Rounds 108–109; S-122, S-123, S-124, S-125 done.)
- [ ] R2 landed or each row is `deliberately different` with both sides cited.
- [ ] R3 has a ranked pass and every finding landed or recorded.
- [ ] R5 booked; a reader transcript and a keyboard-only walkthrough exist with dates.
- [ ] R6 decided in writing by the owner.
- [ ] A tagged release installs and runs from its notes.
- [x] R8's sample is complete and the rows it found stale are corrected. (Round 110: 17 rows, 27 tests, 0 downgraded — [`pinning-sample-2026-10-09.md`](../../audit/evidence/pinning-sample-2026-10-09.md).)
- [ ] `docs/STATUS.md` says what changed and what is still broken; the battery that describes
      the tree is committed and cited.

## 5. Deliberately not in this plan

- **Section A of the DSH matrix (Electron chrome):** 70 rows about a browser, a DOM and window
  management this window does not have and will not acquire (§2.3 of the locked file). They
  stay `missing` as *unportable*, not as gaps.
- **Pi's extension-renderer and decorative rows:** recorded `deliberately different` in the
  matrix, not re-litigated per round.
- **New documentation trees:** `docs/` is large; a finding that is not a row or a plan item is
  a STATUS row, not a new file.
