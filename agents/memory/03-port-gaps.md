# Port gaps: the work that is left, and what to do first

**Status:** open. M194 and M195 are committed and gated; this is what comes next.

## Where the port actually stands

Measured against `pi-0.85.1` on 2026-09-28, over **84 TUI/loop capabilities**:

| | Count | Meaning |
|---|---|---|
| DONE | 38 | behaves as pi does |
| PARTIAL | 22 | exists, but one behaviour or edge case is missing |
| MISSING | 24 | no equivalent in Epsilon |

The 24 missing ones **concentrate in a single cause**: everything pi renders through a
`Component` that Epsilon has no equivalent of —

- a **foldable session tree** (pi's `TreeView`; navigate, label, fold, jump)
- a **settings selector**
- an **editor with jump-to-character**

So "24 missing capabilities" is misleading if read as 24 separate projects. It is closer to
three surfaces plus their supporting commands. Evidence and the full table:
[`audit/evidence/port-fidelity-study-2026-09-28.md`](../../audit/evidence/port-fidelity-study-2026-09-28.md).

## First target: give `branch_with_summary` a production caller

**Why this one first:** it is the repo's own documented defect class — *a faithful port with
no production caller*. Named in `AGENTS.md`, and named by the code itself at
`crates/epsilon-tui/src/app/commands.rs:445-448`. It is small, bounded, and it closes a real
behaviour gap: pi **keeps a record of the branch you abandon**; Epsilon silently drops it.

**The primitive is already correct.** `SessionStore::branch_with_summary`
(`crates/epsilon-session/src/store.rs:1397`) matches pi's `branchWithSummary`
(`session-manager.ts:1381-1405`) in both call shapes, and has tests
(`crates/epsilon-session/tests/session_store.rs:2338-2570`). Every caller is a test.

**pi's production caller** is `navigateTree`
(`references/pi-0.85.1/packages/coding-agent/src/core/agent-session.ts:3114`):

1. refuse while streaming (`3117`)
2. no-op when the target is already the leaf (`3124`)
3. `collectEntriesForBranchSummary(sessionManager, oldLeafId, targetId)` (`3139`)
4. `generateBranchSummary(...)` (`3205`)
5. new leaf position depends on the target's **type** (`3240-3251`):
   - `message` with role `user` → leaf = its `parentId`, and the user's text goes **back to
     the editor**
   - `custom_message` → same
   - anything else → leaf = the target itself
6. `branchWithSummary(newLeafId, text, details, fromExtension, usage)` (`3258`)
7. attach a label if one was given (`3268`)

**The two subtle behaviours worth getting right** — both are easy to miss and both are the
kind of thing this repo has shipped wrong before:

- **The summary is attached at the target, not the old branch** (comment at pi `3254`).
- **Navigating to a user message puts that message's text back in the editor** (`3243`), so
  the user can edit and resend. Epsilon has no equivalent of that today.

## Discipline for this work

- The porting rule from `AGENTS.md`: **grep for callers, then check at least one is
  production code.** A definition plus a test is not a feature. This defect class has now
  been found five times here.
- Ship it with a test that drives the **production path**, not the store method directly —
  otherwise the next agent re-finds the same gap.
- Gate it (`pwsh -File scripts/gate.ps1 <label>`), commit with the log path in the message.

## Next candidates after that

From the same study, in the order I would take them:

| Gap | What | Rough size |
|---|---|---|
| G7 | markdown silently drops tables, task lists and links; strikethrough is **enabled** in the options but no match arm handles it | small — a match arm each |
| G5 | a thinking/reasoning toggle pi exposes | small |
| G6 | viewport paging (pi pages long output; Epsilon scrolls only) | medium |
| G4 | the session tree surface itself | large — the 24-missing cluster |

G7 is the one a user notices daily: a table in a model response currently renders as
run-together text, and a *setting that claims to be on* does nothing — which is the
`app.clear` defect again, in a different costume.
