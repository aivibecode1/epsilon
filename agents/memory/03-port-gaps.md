# Port gaps: the work that is left, and what to do first

**Status:** open. M196 has landed the G7 markdown work and the `tree()` foundation;
see "What M196 already landed" at the bottom before starting anything here.

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
| ~~G7~~ | markdown tables/task lists/strikethrough | **CLOSED in M196** |
| ~~G5~~ | a thinking/reasoning toggle pi exposes | **CLOSED — the note was wrong** |
| ~~G6~~ | viewport paging | **CLOSED — the note was wrong** |
| G4 | the session tree surface itself | large — the 24-missing cluster |

### M196 corrected two rows of this table by reading the code

`03-port-gaps.md` listed G5 (a thinking/reasoning toggle) and G6 (viewport paging) as
open. **Both are already implemented**, and the port study that produced the table is
what is wrong, not the code:

- **G5:** `app.thinking.cycle` is bound in `crates/epsilon-tui/src/app/keys.rs:928` and
  cycles `agent.thinking_level().next()`. `/thinking` also takes an explicit level.
- **G6:** `PageUp`/`PageDown` are bound in the same file (`keys.rs:89`), and the footer
  names the key that scrolls.

The lesson is the repo's own: **an inherited audit is a claim, not a measurement.**
Reading the two lines above took a minute and disproved two rows. Re-verify a gap against
the code before building it — a "missing feature" that is already there costs a
duplicate implementation and a reviewer who believes the audit.

G7 is the one a user notices daily: a table in a model response currently renders as
run-together text, and a *setting that claims to be on* does nothing — which is the
`app.clear` defect again, in a different costume.


## What M196 already landed (do not redo these)

**G7 markdown is done.** `crates/epsilon-tui/src/markdown.rs` now renders GFM tables,
task lists, and strikethrough, with 14 tests. Findings worth keeping:

- The parser had `ENABLE_STRIKETHROUGH` **on with no match arm** — the `app.clear` defect
  in a different costume, and exactly what the port study predicted.
- Three bugs surfaced only because tests looked at *multi-item* output:
  1. `flush` pushed an empty line at every cell boundary, so tables arrived preceded by
     four blank lines.
  2. Row state was never reset per row, so only the first body row survived.
  3. **Pre-existing:** there was no `Event::End(TagEnd::Item)` arm at all, so every bullet
     list rendered as one line (`- a - b`). Any single-item list hid it.
- pi's table layout is `markdown.ts:842`; the property worth porting is the **narrow
  fallback** — a table too wide for columns degrades to readable `a | b` rows rather than
  one-character columns.

**`SessionStore::tree()` is done, and the duplicate is gone.** The store builds the
parentage forest. `rpc_mode.rs` used to carry a **second, hand-written copy** of the same
builder (same three root cases, same timestamp sort, same recursion) — now a thin
projection onto pi's wire shape calling `epsilon_session::build_forest_from_entries`.
The test that holds them together is `the_wire_tree_and_the_store_tree_agree`: it compares
the two **to each other**, which is the only comparison whose expected value does not come
from the implementation.

**Still open for the tree:** pi's labels (`label` / `labelTimestamp` are still `null` on the
wire), and the TUI surface that navigates it.


## G4 measured, not guessed (M196)

The port study calls G4 "the session tree surface" and estimates it large. Reading the
code says the *data* is done and the *surface* is the whole gap:

- `/tree` and `/timeline` both call `open_timeline_picker`
  (`crates/epsilon-tui/src/app/commands.rs:827`), which reads **`branch_entries()`** — the
  active lane only. `SessionStore::tree()` now exists and is not used there, so the
  picker still cannot see an abandoned branch.
- It renders a **flat** list: `timeline_entries: Vec<(seq, kind, snippet)>`. There is no
  depth, no folding, no branch marker.
- pi's `tree-selector.ts` is 1,427 lines and offers: fold/unfold, four filters
  (`user-only`, `no-tools`, `labeled-only`, `all`), search, label editing
  (`updateNodeLabel`), copy, and navigation that goes through `navigateTree`.

**Build order that follows from this** — the store is ready, so each step is TUI-only.
**Steps 1, 2 and 3 are DONE in M196**; what remains is step 4 and the search box.

1. ~~`open_timeline_picker` reads `tree()` and renders depth + branch markers.~~ **Done.**
2. ~~Label editing through `SessionStore::set_label`.~~ **Done** — `Shift+L`, per
   `keybindings.ts:159` (a bare `l` was my first guess and would have collided with a
   future "navigate left" chord).
3. ~~Filters and folding.~~ **Done** — `f` cycles pi's five modes; `Ctrl+Left` folds-or-up
   and `Ctrl+Right` unfolds-or-down, per `keybindings.ts:150-157`.
4. Search, and `copySelected` (`tree-selector.ts:627`).

### Two things the filter and fold got wrong first time, worth keeping

- **Filtering a tree is not `filter().collect()`.** pi filters a flat list and then
  *recalculates the indent from what survived* (`tree-selector.ts:408`), so a row whose
  parent was filtered out is **promoted to a root** rather than dropped or left
  indented under something invisible. The first implementation returned the original
  depth; a test caught it.
- **A fold is transitive.** pi's second pass marks a row hidden when its parent is folded
  *or is itself marked* (`tree-selector.ts:400-407`) — that second clause is what hides a
  whole subtree instead of one level.
- Both the filter and the two fold chords re-derive the view, so they share one
  `refilter(app)` helper. Written three times, the copies drift in their selection
  handling, and a fold that moves the highlight onto another row makes a view feel
  haunted.

**Do not** re-derive the store side. `build_forest_from_entries` is public, the wire tree
projects it, and `the_wire_tree_and_the_store_tree_agree` holds the two together.


## The keybinding chords to match, read from `keybindings.ts`

Do not guess these. `references/pi-0.85.1/packages/coding-agent/src/core/keybindings.ts`:

| Action | Default | Line |
|---|---|---|
| fold, else move up | `ctrl+left`, `alt+left` | 150 |
| unfold, else move down | `ctrl+right`, `alt+right` | 154 |
| edit label | `shift+l` | 159 |
| filter: default | `ctrl+d` | 210 |
| filter: hide tool results | `ctrl+t` | 216 |
| filter: user messages only | `ctrl+u` | 220 |

Epsilon binds the filter to a bare `f` and the label to `shift+l`; folding is on the
Ctrl+arrow chords. A bare `l` was tried and rejected — see above.


## M196 commit plan (one battery, six commits)

A single green battery covers all of this; the commits are logical units, not
separate builds. Staging by explicit path — never `-A`.

| # | Commit | Files |
|---|---|---|
| 1 | cache percent stops claiming a full cache | `epsilon-tui/src/ui/format.rs`, `ui/metrics.rs` |
| 2 | a reporting provider is not a silent one | `epsilon-core/src/cache_stats.rs`, `tests/cache_attribution.rs` |
| 3 | GFM markdown: tables, task lists, strikethrough | `epsilon-tui/src/markdown.rs` |
| 4 | the session forest has one builder | `epsilon-session/src/store.rs`, `lib.rs`, `tests/session_store.rs`, `epsilon-cli/src/rpc_mode.rs`, `epsilon-contracts/src/message.rs` |
| 5 | branch labels: a type with no writer now has one | `epsilon-session/src/store.rs` (overlap — stage by line, see AGENTS.md) |
| 6 | `/tree` sees abandoned branches, labels, filters, folds | `epsilon-tui/src/app{.rs,/commands.rs,/keys.rs,/panels.rs,/session.rs,/view.rs,/tests.rs}` |

Commits 4 and 5 both touch `store.rs`. **Stage that file by line**
(`git hash-object -w` + `git update-index --cacheinfo`) and prove the staged blob
against HEAD that only the label lines are in the second commit — the shared-file
rule in AGENTS.md, and the only thing here that is genuinely fiddly.


## M196: a deadlock that HUNG instead of failing (worth remembering)

Adding the re-projection after a label write, I wrote:

```rust
let Ok(mut st) = store.lock() else { return false };
st.set_label(&id, label)?;            // write under the lock
...
agent.reproject_current_branch()      // locks the SAME store again -> deadlock
```

**The failure mode was worse than a compile error: the test hung for 60+ seconds.**
Nothing failed; the run simply stopped making progress, which reads as a slow machine
rather than a bug. I found it only because `cargo test` prints
`test ... has been running for over 60 seconds`, and I read that line instead of
skimming it.

Three things worth keeping:

- **A hang is a result.** When a test does not finish, that is the finding. Do not
  wait longer — kill it and read the line the harness printed about the stalled test.
- **Scope the lock to the operation that needs it.** A write and a read of its result
  that must be consistent belong in **one store method**, not in a longer-held lock at
  the call site.
- **This is what the "what nothing exercises" discipline buys.** The label editor was
  new code with a new test; the test exercised the real key path, and the deadlock was
  in code that compiled cleanly and passed clippy.
