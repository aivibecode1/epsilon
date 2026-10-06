# Plan 024 — finishing the objective

**Status: item 2 is LANDED (M312, ADR-028). The rest is open.**

Written 2026-10-04 at handoff, because the session that produced it ran out of clock rather than
because the list is speculative. **Every item here was verified against the tree at the time of
writing**, and the evidence is named so you can re-check it before you spend a day on it — which you
should, because a finding is a claim about a tree and trees move.

> **Three of this plan's numbers were wrong when this file was written, and two independent
> read-only passes corrected them on the same day.** They are corrected below and the corrections are
> kept in the text rather than quietly applied, because the plan is the artefact a later session
> will trust:
>
> - **§2 is done.** `append` refuses once the lock is not ours, `persist_migration` too, the
>   predicate separates a takeover from an unreadable lock, and the TUI's `/name` — the last
>   production site that discarded a write — now reports it. ADR-028.
> - **§5's denominator is wrong: it is 3 of 37, not 3 of 38.** `COMMAND_HEADS` holds 37 rows;
>   38 is the non-`Unknown` variant count, because `Models`/`Model(String)` are two variants for one
>   head. The numerator was exact. [`audit/evidence/plan024-item5-desktop-commands.md`](../../audit/evidence/plan024-item5-desktop-commands.md)
>   also finds **two** joining tests rather than one, a 7-row palette table nothing reconciles, and
>   two ADR-024 citations pointing at the wrong lines.
> - **§7's counts are wrong in the same direction Class A's were**: the header says 50 items /
>   652 lines; the tables hold **40 rows** plus 15 items in prose. Trust the rows.
>   [`audit/evidence/plan024-item7-class-b-rederived.md`](../../audit/evidence/plan024-item7-class-b-rederived.md).
> - **§3's recipe cannot work as written.** "`evaluate` is already `pub` … so a defaulted method
>   belongs on the `PermissionGate` trait and reuses `evaluate`" is architecturally impossible:
>   `epsilon-safety` depends on `epsilon-core`, not the reverse, so a default body in
>   `epsilon-core` cannot name `PermissionEngine`. The call belongs in an override in
>   `epsilon-safety`. There are also **three** call sites, not two, and **the default ruleset has
>   zero `Deny` rules**, so the feature hides nothing unless a project config or `--plan` supplies
>   one. [`audit/evidence/plan024-item3-denied-tools.md`](../../audit/evidence/plan024-item3-denied-tools.md).
> - **Not in this plan at all, and found while doing §5:** `/theme` advertised a
>   `high-contrast` palette that does not exist. Fixed at M312, and the capability is still
>   missing — see the roadmap in [`../STATUS.md`](../STATUS.md).

Read [`../HANDOFF.md`](../HANDOFF.md) first if you have not. Read [`../../AGENTS.md`](../../AGENTS.md)
before your first change.

---

## How to use this plan

- **Order is value, not dependency.** Item 1 is the largest gap in the objective. Item 2 has a
  prerequisite written into it and will waste your time if you skip that.
- **Each item says how to know you are done.** "Done" here means a battery leg and a test that fails
  for the reason it names, not a diff.
- **Anything you cannot finish goes back into `STATUS.md`'s roadmap, described, not half-built.**
  The rule that has served this repository best: *a working, verified product beats breadth.*
- **Check `STATUS.md`'s Known-broken section before you start.** Some items here have a longer
  treatment there, including the failures that made them hard.

---

## 1. Run a real screen reader against the desktop window

**Why this is first.** It is the largest gap in the objective's definition of done, and M307 moved
the evidence as far as it can go without a human: the accessibility tree is now known to reach
**Windows UI Automation from another process** — 53 nodes, 52 named. UIA is the API NVDA and
Narrator are built on, but it returns the *raw tree*. **A reader decides what to say and in what
order, and nothing has ever made it speak.**

**Where.** `crates/epsilon-desktop/src/app.rs` (the window), `scripts/desktop-a11y-probe.ps1` (the
existing probe), `audit/evidence/desktop-uia-tree-2026-10-04.txt` (the current tree).

**What to do.**

1. Launch the window — offscreen rendering is not needed for this, because accessibility is
   published by the application and not by the compositor, which is *why* the probe works on a
   window that draws nothing (§4 of the handoff).
2. Start Narrator (`Ctrl`+`Win`+`Enter`). Walk the landmarks: transcript, editor, file tree, title
   bar, status bar, goal band, composer.
3. **Record what it actually announces**, verbatim, next to the 2026-10-04 tree so the two are
   comparable. Keep it in `audit/evidence/`.
4. Fix what the reader gets wrong, and add a test that pins the *wording* — `app.rs` already has
   `the_accessibility_names_read_correctly_and_the_filter_is_named` as the model, and it exists
   because reading a tree dump found two bugs no assertion was looking at.

**Done when:** a transcript of a real reader session is in `audit/evidence/`, and any defect it
found has a test that fails without its fix.

**The known first candidate:** one unnamed `ScrollBar`
(`audit/evidence/desktop-uia-tree-2026-10-04.txt`). See item 6 — do not start there.

---

## 2. Make a stolen session lease stop the write — **LANDED (M312, ADR-028)**

**Done, and the two traps below are where the work went.** The lease field is `lease` rather than
`_lease`, `LeaseOwnership` answers `Ours` / `Taken` / `Missing` / `Unreadable`, `append` refuses on
the two that are evidence and not on the one that is not, `persist_migration` refuses as well
(it replaces the file rather than adding to it), and every caller surfaces the error. Four tests,
each with a negative control; the loop test's control prints a completed turn written into a session
file another process owns.

**What the battery added:** five `epsilon-runtime` goal-round tests failed because their fixture
deleted its own session directory at the end of a helper. The refusal was right, the fixture was
wrong, and the message was corrected because it asserted a rival that might not exist.

**Why.** Two writers can still share one session file. The victim keeps appending through an
already-open handle after another process has taken the lock, and nothing detects it.

**The trap, measured, and it is the whole reason this item is long.** The fix looks like two lines:
`SessionLease::lock_is_ours()` already exists (`crates/epsilon-session/src/lease.rs:558`, private,
used only by `Drop`) and is exactly the predicate; add a check in `SessionStore::append` and refuse.
**It is not safe on its own:**

1. **Two production call sites discard the result of `append`**, so a refusal becomes a *silent drop*
   instead of a refusal:
   - `crates/epsilon-core/src/lib.rs` — `record_model_change`, `let _ = guard.append(ModelChange)`
   - `crates/epsilon-core/src/lib.rs` — `attach_child_session`, `let _ = store.append(Custom { task_meta })`

   (M311 changed `record_model_change` to **emit a notice** on failure; re-grep, the lines have
   moved. `attach_child_session` was fixed the same way.)
2. **`lock_is_ours()` answers `false` for an unreadable file as well as a stolen one** — it is
   `read_to_string(..).is_ok_and(..)`. A false negative would refuse a legitimate write and **lose
   the message**. The predicate must distinguish *"someone else owns this"* from *"I cannot read it
   right now"*, and the refusal has to be loud either way.

**Done when:** `append` refuses once the lock is not ours, **every** caller surfaces that refusal,
a test drives a real takeover and fails without the refusal, and an ADR entry records the decision.
Note `AGENTS.md`: a refusal path must leave the file untouched.

**The design decision you owe the ADR.** Whether the *first* refusal is enough, or whether the
session should be re-opened. Re-opening means re-acquiring a lease that another process holds —
which is the thing the design forbids. Say which you chose and why.

---

## 3. Hide denied tools from the model's tool list — **and say so in prose**

**Why.** Offering the model a tool the permission engine will refuse is a lie the UI tells, and it
costs a wasted turn before the refusal. `AGENTS.md` §8 forbids silent failures, and this is the
quieter version: the model is invited, then blocked.

**The recipe is already worked out** (this is `STATUS.md` roadmap item 3, re-measured 2026-10-04):

- The "no denied set to read" half of the old premise is **false as a blocker**: `evaluate` is
  already `pub` (`crates/epsilon-safety/src/engine.rs:83`) and `RulesetGate` owns the engine
  (`gate.rs:52`), so a **defaulted** method belongs on the `PermissionGate` trait
  (`crates/epsilon-core/src/gate.rs:51`) and reuses `evaluate`.
- `session_allows` is an **allow** set with zero production callers. It is not the accessor.
- Two call sites, both in `stream_one_turn`: the stable prompt tier and the request payload
  (`crates/epsilon-core/src/lib.rs`; the tier is fingerprinted, so filtering **changes
  `stable_hash` by design** — that is a stated consequence, not a surprise).
- Filtering reuses the existing precedent `ToolRegistry::subset` (`registry.rs:59`).
- **It must tell the model in prose that the tool is refused.** Hiding it alone answers
  *"Tool 'bash' not found"*, which is strictly worse than a policy refusal.

**Done when:** a refused tool is absent from the request payload and the prompt tier, the model is
told why in prose, the fingerprint change is recorded, and a test drives a real denial through the
loop.

---

## 4. `/model` auto-fetch should target the catalogue, not the cursor row

**Why.** Opening the picker fires a live listing request for **whichever stage-1 row the cursor is
on**. Moving the arrow keys fetches repeatedly, and the reader sees notices about a provider they
did not choose. M306 silenced the no-credential case; the targeting is still wrong.

**What the reference actually does** — measured, and the first reading of it was wrong, so check it
yourself: pi **does** refresh at runtime (`references/pi-1.0.0/packages/coding-agent/src/modes/interactive/model-catalog-refresh.ts`),
but `interactive-mode.ts:1136-1143` refreshes the **whole catalogue once at startup**, swallows
failures (`.catch(() => {})`), and bounds the work with a 15-second timeout. **The divergence that
hurt was targeting plus volume, not the existence of the feature.**

**Where.** `crates/epsilon-tui/src/app.rs` — `request_live` and its four call sites; the desktop has
no equivalent.

**Done when:** the auto-fetch no longer depends on cursor position, a frame test pins that opening
the picker does not print a notice naming a provider the reader did not select, and the change is
recorded as a behaviour change.

---

## 5. Raise the desktop's command coverage

**Why.** `crates/epsilon-desktop/src/commands.rs` answered **3 of 37** terminal commands when
this was written. Refusing by name is the right pattern; a small fraction of the surface is the
problem, and the objective asks for the desktop "to the highest standard".

> **Re-measured 2026-10-05: this section was stale on arrival and is now seven of 37.**
> `/tasks`, `/session`, `/clear` and `/theme` landed at M312 (ADR-029), and the denominator was
> wrong even then — `COMMAND_HEADS` holds 37 rows, not 38; 38 was the non-`Unknown` *variant*
> count, because `Models`/`Model(String)` are two variants for one head. What has **not** landed
> is the mechanism: `SUPPORTED` and the match arms remain two statements of one fact held together
> by tests rather than by construction. That is the real work; adding arms was the easy part.

**Where.** `crates/epsilon-desktop/src/commands.rs` (`window_runs`, `SUPPORTED`) and the shared
registry in `crates/epsilon-contracts/src/command.rs`.

**Done when:** the number is up and the *declaration* is derived from the implementation rather than
checked against it. **The current honest residual, written in the code:** `window_runs` steers the
*exported capability answer*, not production dispatch, so the declaration and the implementation are
two statements of one fact joined by one test — **not by construction**. Closing that is the real
work; adding arms is the easy part.

---

## 6. Name the unnamed `ScrollBar`

**Why.** One node a screen reader can only address by its role, in a window that has several.

**Where it comes from.** egui draws it at `scroll_area.rs:1334-1336` with
`WidgetInfo::new(WidgetType::ScrollBar)` — **a role and no label** — at the id
`scroll_area_id.with(axis)`. The four scroll areas here all carry stable id salts
(`epsilon-file-tree`, `epsilon-diff-body`, `epsilon-file-body`, `epsilon-agent-transcript`), so the
id is computable.

**Why it is not a one-liner, and this is the trap.**
`Context::accesskit_node_builder` is **get-or-create** (`egui-0.36.2/src/context.rs:3675-3690`,
whose doc says so). Naming both axes unconditionally **creates** a node for a scroll bar that is not
shown — trading one unnamed node for two phantom ones, which is **worse** for a reader. A correct
fix needs to know whether the bar exists; egui's own `content_is_too_large` is the honest predicate,
and a second copy of it here is the thing Ponytail asks you not to write.

**Done when:** `scripts/desktop-a11y-probe.ps1` exits **0** — it exits 1 on any unnamed node, and it
is deliberately **not** a `gate.ps1` leg while this is open, because a gate that is red for a known
finding gets deleted rather than read.

---

## 7. Class B of the dead-code sweep

**Why.** 50 items, 652 lines, whose *only* callers are tests. **Class A is done** (M308: 19 deleted,
3 kept with written reasons). Class B is not mechanical.

**Why it is a judgement, item by item.** Each deletion takes its test sites with it, so the question
is whether the **test** was worth more than the code it exercises. Some are clearly dead; some are
the only record of a capability (`OAUTH_SYSTEM_IDENTITY` is B2-shaped and was kept for exactly that
reason).

**Where.** `audit/evidence/ponytail-deadcode-2026-10-04.md`, Class B.

**Re-derive the list before acting.** The ledger's own header said "23 items, 175 lines" for Class A
and it has 22 rows summing to 160 — **the counts have been wrong once**, and Class B has not been
re-verified since M303–M311 moved the tree.

**Done when:** every Class B item is either deleted with its test, or kept with a written reason.

---

## 8. Lower-priority, recorded so they are not lost

| Item | Where | Note |
|---|---|---|
| Desktop keyboard-only pass by a person | both front ends | The *automated* coverage is substantial (~16 desktop tests drive real key events through the real draw path; the ConPTY suite types into the real binary). **No record exists of a human doing it.** |
| `/model` durability on the RPC paths | `crates/epsilon-cli/src/rpc_mode.rs:660,722` | M303 moved the session record into `set_model_id` so these are covered. Verify rather than assume. |
| The desktop's `task_meta` notice sink | `crates/epsilon-core/src/lib.rs`, `attach_child_session` | M311 routed its two failures through `AgentEvent::Notice`. Check nothing else on that path still prints to a console a window does not have. |
| `SCROLLBAR` role on the file-tree pane | `crates/epsilon-desktop/src/workspace.rs` | The pane is `Region` and its rows `TreeItem`, and **the nodes are siblings under the root** — flipping the role would change one label and create no structure. Real hierarchy needs a node on the pane `Ui` first, which moves every region's parentage. Behaviour-level; needs a decision. |

---

## What NOT to do

- **Do not "fix" the blank desktop window by changing GPU configuration.** The offscreen path exists
  and works; presentation is a machine fault, and `AGENTS.md` forbids changing security
  configuration from an agent session.
- **Do not make `append` refuse before every caller surfaces the error.** Item 2. Doing it the other
  way round trades a visible interleaving risk for silent data loss.
- **Do not delete a check because it is red for a known finding.** Say so in the check's own doc and
  keep it red, as `desktop-a11y-probe.ps1` does.
- **Do not trust a count in a document over the command that produced it** — including the counts in
  this plan if the tree has moved since 2026-10-04.
