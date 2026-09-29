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

## STILL OPEN, and it is a size decision not a task: branch summaries on navigate

Re-measured at the end of M196. `SessionStore::branch_with_summary` is correct,
tested, and still has **no production caller** — the original
"faithful port, no production caller". `/tree`'s Enter navigates and does not
write a summary.

**Why it is still open, honestly: pi's version is a PROVIDER CALL.** The recipe
at `agent-session.ts:3114-3268` is
`collectEntriesForBranchSummary` -> `generateBranchSummary(...)` ->
`branchWithSummary(newLeafId, text, ...)`. The text is *model-generated*. Epsilon's
key handler is synchronous, so producing it means an async round-trip wired into
the key path, plus a decision about what happens when the provider call fails
mid-navigation — pi has that at `:3271-3273` and it is not a small branch.

Writing `branch_with_summary(target, "left this branch", None, None)` would close
the wiring in an afternoon and would be **worse than nothing**: a fixed string
pretending to be a summary of a conversation it never read. This is the case
where leaving a capability visibly absent beats shipping a fake one.

**Also still open, and worth naming:** the summary is attached at the
navigation TARGET, not the abandoned branch (`agent-session.ts:3254`), and
navigating to a *user* message puts that message's text back in the editor so it
can be edited and resent (`:3243`). Epsilon has neither behaviour.

## (superseded heading kept for the recipe) First target: give `branch_with_summary` a production caller

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


## The settings gap is really a SKILLS gap (M196, measured)

The port study calls it "a settings selector". Reading pi's `config-selector.ts`
(942 lines) says otherwise: it is not a toggles panel at all, it is a **resource
enable/disable browser** over `extensions | skills | prompts | themes`, scoped
`user | project`, writing `+pattern` / `-pattern` into the settings arrays
(`config-selector.ts:16755+`).

Epsilon's equivalent today:

- `/settings` is a **notice dump** -- paths, current model, thinking level, recents.
  Deliberate, with a comment naming the deferral.
- `/skills` prints `names.join(", ")` -- a comma-joined list of NAMES, read-only.

So the honest gap is: **there is no way to enable or disable a skill from the
TUI, and no way to see what a skill does without leaving it.** That is one
bounded feature, not a 30-toggle settings panel, and it is worth more than
building a selector for settings Epsilon does not have.

`/templates` already prints descriptions -- that is the pattern to follow for
`/skills`, and the model for a row that carries a summary rather than a name.

**Do not** build a generic settings panel to close this row of a table. Build the
skill toggle, and read the shape off the real screen.


## A test that paid for itself immediately (M196)

`crates/epsilon-cli/src/runtime.rs:2726` —
`every_milestone_has_a_row_somewhere_in_the_history` parses `docs/history.md`, takes
every `| M<digits>` row, and asserts the set is **contiguous up to its newest entry**.

Adding the M196 row made 196 the newest and the test immediately reported:

```
docs/history.md has no row for milestones [194, 195] (newest is M196)
```

So **M194 and M195 were committed without history rows** — the exact drift the test
exists to prevent, sitting undetected in the tree until something newer forced the
check. It was written and it worked; it just had no trigger until now.

Worth remembering as a shape: a test can be green for a long time and still be the
right test. This one was not wrong, it was un-exercised by a gap that had already
accumulated. Filling the rows is the fix; there is no code change.


## A GREEN BATTERY HIDED FIVE DELETED TESTS (M196) — the most important note here

While reworking one test in `crates/epsilon-tui/src/app/tests.rs` I replaced a
**line range** rather than matched text. The range was stale — `cargo fmt` had
reformatted it between my read and my write — so the slice took **five neighbouring
tests with it**, including `the_tree_picker_shows_an_abandoned_branch`,
`the_label_editor_writes_through_the_store_and_clears_on_empty`,
`a_filter_hides_entries_and_promotes_the_ones_left`, `cycling_the_filter_is_a_round_trip`
and `folding_hides_the_whole_subtree_not_just_one_level`.

**The battery came back `GATE PASS`.** 1,516 tests, 0 failed — green, and shipping five
tests that no longer existed. Nothing in the suite can catch this: a deleted test is not
a failing test, and a whole *category* of behaviour lost its only coverage silently.

It was caught only because the number moved: **1,520 on the previous green run, 1,516
now.** Four fewer. That is the entire detection mechanism, and it is a human comparing
two runs.

### The rules that follow

1. **Never edit a test file by line range.** Match on the test's own name and re-read
   the current text immediately before writing. The format-on-write step means the text
   you matched against may no longer exist.
2. **After any change to a test file, diff the test NAMES against HEAD before running
   anything:**
   ```
   git show HEAD:<file> | grep -B1 '^fn ' | grep -o 'fn [a-z_]*' | sort > /tmp/before
   grep -B1 '^fn ' <file>              | grep -o 'fn [a-z_]*' | sort > /tmp/after
   diff /tmp/before /tmp/after
   ```
   Lost names are deleted coverage, and the suite will not complain.
3. **A test count that goes DOWN between two green runs is a finding, not noise.**
   Write the number down every run. I only noticed because the totals were in the log.
4. Recovery, when it happens: `git checkout -- <test file>`, then re-apply the intended
   change by NAME. That is what I did, and it is faster than trying to reconstruct five
   tests.


### The guard test needed THREE attempts, and that is the real lesson

`no_source_file_lost_a_test_since_head` exists to make the deletion above
impossible to repeat silently. It took three tries before it actually worked, and
each failure is the same shape: **the test passed while doing nothing.**

1. **Version 1** counted removed `#[test]` attributes against added ones. The
   control PASSED — because the same diff that deleted one test also *added* two
   new ones, so 1 <= 2. A net count cannot see one name vanishing among three
   arriving. **Compare NAMES, not counts.**
2. **Version 2** compared names, and still passed vacuously: cargo sets a test's
   cwd to the **crate directory**, where `git diff -- crates/` matches nothing
   (`crates/` is not under `crates/epsilon-cli/`), exits 0, and prints zero
   bytes. **Run `git -C <repo root>`**, and assert that a dirty tree actually
   produces diff output, so "inspected nothing" can never read as "found nothing".
3. **Version 3** fires, and names the deleted test exactly.

The general rule, and it is the one this repo keeps relearning: **a control that
passes is a control you have not yet proven works.** Both earlier versions passed
*while the defect was live in the tree*. Neither was broken by a typo -- each was
structurally incapable of seeing the thing it was written to see.


## `git checkout -- <file>` DESTROYED uncommitted work (M196, again)

A brace-balance edit to `crates/epsilon-tui/src/app/commands.rs` left the file
unparseable. The reflex was `git checkout -- <file>` to get back to a known
state — which is safe on committed work and **catastrophic on everything
else**. It threw away the entire tree-search implementation and the `/skills`
change, both uncommitted, in the same file as the edit.

It was recoverable because the design was known, but it cost four rounds.

**The rule: commit or stash BEFORE a surgical edit, never after.** By the time a
brace is unbalanced the interesting work is already uncommitted, and the tool
that is supposed to help is the one that removes it.

Two habits that would have prevented the whole afternoon:

1. `git stash push -- <file>` before hand-editing a large file by hand, and
   `git stash pop` when it compiles. Cheap, and the stash *is* the backup.
2. Check brace depth programmatically after a hand edit rather than by eye:
   ```python
   d = sum(l.count("{") - l.count("}") for l in text.split("\n"))
   assert d == 0, d
   ```
   Two of the three "unexpected closing delimiter" rounds were this check run
   too late — after the file was already written.


## Search and the skills toggle are DONE (M196, second half)

**`/tree` search** — pi's exact rule (`tree-selector.ts:337-397`):

- lowercase the query, split on whitespace, **every** token must be a substring
  (`searchTokens.every`, so AND not OR);
- searched text is **label + kind + snippet**, which is what
  `getSearchableText` (`:560-600`) builds — the message *role* pi also pushes is
  the `kind` tag here;
- **the current leaf is never hidden** (`:342-343`) and a textless assistant
  turn is hidden unless it errored (`:345-355`).

The whole visibility pipeline is one function, `visible_rows`: filter -> search
-> mandatory -> folds. **Two callers** open the picker and `refilter`, and two
copies would drift invisibly.

**Skills on/off** — the re-scoped gap, now closed end to end:

- `[skills] disabled` in config, written with the `toggle_favorite_model_in`
  idiom (parse to a table, change ONE key, write back). A **disable-list, not
  an enable-list**: an enable-list would silently drop every skill the day the
  config gains a section it did not have.
- `/skill <name> on|off`, routed as a MUTATING command, not a view one.
- The handler **checks the name against what is discovered first**. Writing a
  disable for a skill nothing provides gives a setting that does nothing AND
  reads back as a disabled skill the user cannot find to re-enable.
- `/skills` now prints descriptions and marks `[off]`, and says how to change
  one.

### Three tests that already existed, and caught my own work

- `the_parser_does_not_know_a_command_the_list_does_not` — a **reverse-drift**
  check. It failed naming exactly what I had missed: `skill` parsed but was not
  in `SLASH_ITEMS`, so the palette would not offer it. A command the palette
  omits is one a user must guess the name of.
- The config round-trip tests — the load-bearing one is that writing a skill
  setting **leaves every other key alone**, including a table this build knows
  nothing about. That is what makes writing from a TUI safe at all.

### A test I changed instead of the code

I asserted the disable-list kept insertion order. It does not: `retain`-then-
`push` moves a re-disabled skill to the end. **Nothing reads that order**, so
"fixing" correct code to satisfy a property no caller depends on is the wrong
trade. The test now asserts each name appears **exactly once** — the bug `retain`
actually exists to prevent.


## M196 closed the tree's remaining adjacents

**Fork at a selected user message** (`k` in the `/tree` picker). `/fork` forks at
the LEAF, so before the tree existed there was no way to say "keep everything up
to here and try something else" — and `fork_branch_at`, correct since M192 with an
RPC caller, had **no TUI path at all**. Seventh occurrence of the repo's own defect
class.

Two deliberate choices:
- **only user messages.** A fork of an assistant turn or a tool result leaves a
  session ending mid-answer: a file pi can produce, a user cannot use. The refusal
  says which kind was selected.
- **the session file's own parent directory**, matching `/fork`. I first guessed a
  `sessions_dir()` that does not exist; had that compiled, forks would have landed
  somewhere the session browser does not list.

**Jump to previous/next semantic prompt** (`ctrl+up` / `ctrl+down`, pi's chords at
`tui/src/keybindings.ts:184-192`). pi finds prompts by matching OSC-133 markers in
rendered rows (`tui-alt-screen.ts:489`); Epsilon emits none, so the same idea is
`LineKind::User` — which is what a prompt already IS in the transcript's data.

The reason it earns a key: a long session is mostly tool results, so paging by ten
rows stops on things nobody was reading.

Three properties the test pins, each a way a naive version fails:
1. starts **one row away** (`scrollTop + direction`, `tui-alt-screen.ts:488`) — a
   prompt the reader is already looking at is not a jump;
2. **skips non-user rows**;
3. returns `None` at the ends rather than **clamping** — clamping is how "you are
   already at the top" becomes a key that looks broken.

## The port study, re-measured rather than trusted (M196)

All 84 rows checked against the code. **Ten rows it reported open are closed** —
six by M196, three by M194, one PARTIAL already effectively DONE. **29 MISSING
becomes 21.** `/settings` stays open deliberately, with a comment naming the
deferral, and the addendum in the study says so.

The finding cuts BOTH ways, and that is the part worth keeping: the study also
reported G5 and G6 as MISSING when they were implemented. **A study written
against a tree that is still moving is wrong in both directions**, and the rows it
gets wrong in your favour are the expensive ones — building them produces a
duplicate implementation and a reviewer who believes the table.

## The deletion guard has a false positive, and a fix that was itself wrong twice

`no_source_file_lost_a_test_since_head` fired on `fmt_tokens` after I made it
`pub` for the compaction row. The diff is `-fn fmt_tokens(` / `+pub fn fmt_tokens(`,
and a `fn`-only match reads the first as a deletion with no replacement.

**The first fix did not work**, and this is the part worth keeping. It stripped
`"pub fn "` and *then* tried `strip_prefix("fn ")` — but `"pub fn "` already
contains `"fn "`, so nothing was left to match and the name never formed. It
shipped as "fixed" because the code compiled. The second fix stripped both and
went one step further wrong by shadowing `rest` into itself.

The working shape is **one strip loop over `["pub(crate) fn ", "pub fn ", "fn "]`,
breaking on the first match, with the `else` doing the skip** — the visibility
test has to happen *before* anything is consumed.

**A guard needs a control that is real.** My first control was a test name I
had invented (`a_half_page_is_half_the_measured_viewport`), which did not exist
at HEAD — so deleting it changed nothing and the guard correctly passed. A
control that never exercised the guard proved nothing. The real control deleted
`the_label_prompt_and_jump_notice_draw` (a test that *is* at HEAD): the guard
named it, which is the whole property.

**And restoring from a backup taken after the damage is not restoring.** The
`shutil.copy2` that "restored" the file copied the already-damaged version, and
`AGENTS.md`'s mtime rule applied to it too. The block was recovered from
`git show HEAD:<file>` by name, and the file then compared equal to HEAD's test
set (113 tests, none missing) before the suite was run again.

**Count reconciliation:** 429 declared at HEAD, 433 compiled now, 4 added
(`a_compaction_row_reports_the_same_tokens_it_records`,
`a_prompt_jump_lands_on_a_user_turn_and_reports_the_ends`,
`forking_from_the_tree_writes_only_the_chosen_ancestry`,
`the_copy_key_reaches_the_clipboard_path`). A delta of 1 from an expected 434
was the *control's* damage, not a lost test — which is why the named comparison
runs against the compiled list, not against a remembered number.

## The `!` shell escape, and where it had to run (M196)

pi: `!cmd` runs a shell command, `!!cmd` runs one whose output stays out of the
model's context (`interactive-mode.ts:3105-3119`, `handleBashCommand` `:6476`).
Epsilon now has both.

**The decision that mattered: the TUI does not own a shell runner.** The obvious
implementation -- `epsilon-tui` depends on `epsilon-tools`, builds a `BashTool` --
is wrong twice. It is a new edge in the dependency graph, and the tool would carry
a **default** sandbox rather than the session's configured one, so a user could run
through `!` something the agent is not permitted to run.

What it does instead: `epsilon_core::exec::run_local(&Arc<Agent>, name, args)`,
public, which mirrors `execute_batch`'s sequence exactly -- registry lookup,
`basic_validate`, `resource_key`/mode, then `gate.authorize` with the same
`PermissionRequested`/`PermissionDecided` events. A `!` command can therefore
raise an approval prompt, which is correct: the user is running it.

`epsilon-tui` keeps its downward edge; `epsilon-tools` is a **dev-dependency** of
the TUI, for the test's registry only.

**Async, deliberately.** The key handler is synchronous and the tool's cap is 120s.
Blocking would reproduce the "TUI looks dead" symptom this repo has chased since
M151.

### Three ways those two tests were wrong before they were right

All the same shape this repository keeps meeting -- a test reporting success while
the thing you wanted did not happen:

1. **The empty registry.** `test_app()` registers nothing, so `run_local` answers
   "no tool named bash", and a test asserting on notice text accepts that as a
   successful run. Fixed with `test_app_with_bash`, which also hands back the
   `ChannelApprovalUi` **clone** (a second `new()` is a different queue, so answers
   go nowhere).
2. **The unanswered gate.** With a real `bash` registered and nobody answering,
   `run_local` blocks forever and the test sees nothing. That is correct product
   behaviour; the *test* was wrong. It now answers the ask, as a user would.
3. **The drain that broke on the first timeout** after 50ms, holding only the
   `running:` line. Then breaking on the *marker* -- which `running: echo <marker>`
   contains. Both assertions now require the **labelled output line**:
   `shell: ...` vs `shell (kept out of context): ...`.

**The control.** With the `!` arm's prefix made unmatchable, both tests fail. The
first two control attempts **did not compile** (a multi-codepoint char literal),
which proved nothing -- a control that does not build is not a control.

### And the line-range trap, three times in one session

Restoring a mangled test file by **line range** corrupted it again: once truncating
`park_ask` mid-body in `tests_support.rs`, once landing a probe block on the *imports*
at line 10 of `tests.rs`. `git show HEAD:<file>` then re-apply **by test name** is
the only version of this that worked. `AGENTS.md` states the rule; I watched it fail
three times before treating it as load-bearing.
