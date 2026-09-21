# M109 — `@` completion stops offering files the tools would refuse

## What

Typing `@` in the composer lists workspace files. That list was built by a walker with
its own hardcoded skip list — `.git`, `target`, `node_modules`, `dist`, `.epsilon` — and
no knowledge of `.gitignore`. The tools (`read`, `grep`, `find`, `edit`) walk the same
workspace through `epsilon_core::ignore`, so on any repository with a `.gitignore` the two
disagreed: `@` offered files the model could not read, and the harness's own
[`audit/11`](../../audit/11-current-assessment.md) row for M99 claimed one shared answer
while only half of it was true.

The fix is that the completion walk now asks `epsilon_core::ignore` — the same `IgnoreSet`
the tools load, the same `skip_dir` list — and keeps its own traversal only because the two
walks genuinely want different things. A tool call must see everything it was asked about;
a completion popup is racing a keystroke, so it collects a bounded number of rows over a
bounded depth and stops early.

**The failing test taught me something the audit only half-knew.** My first version of the
walker copied the tool walk's hidden-directory check,
`name.starts_with('.') && !ignore.is_ignored(..)`, along with its comment promising that a
`.gitignore` negation re-includes a hidden directory. The test I wrote for that promise
failed, and the reason is that the promise is unkeepable:

```rust
// IgnoreSet::is_ignored, crates/epsilon-core/src/ignore.rs
for depth in 1..=parts.len() {          // resolve each ancestor in order
    ...
    if let Some(d) = decision { ignored = d; }
    if ignored { return true; }         // nothing below an ignored component is
}                                       // re-includable by a deeper rule
false
```

With `.hidden/` followed by `!.hidden/`, the first rule sets the decision to "ignored" and
the second clears it, so `is_ignored(".hidden", true)` answers **false** — the same answer
it gives for a directory no rule mentions at all. A walk therefore cannot distinguish
"re-included" from "never mentioned", and the extra clause can only differ from "always
skip" in the case where the very next check skips the entry anyway. It was dead logic
carrying a promise, in the TUI walk and in the tool walk both. The tool walk's *test* had
already recorded the limitation honestly ("NOT asserted, because it does not work yet"),
while its inline comment and the doc comment on `walk_files` claimed the feature — the code
was honest in one place and false in two.

## Key decisions

- **Share the ignore answer, not the traversal.** Two walks with different budgets is a real
  difference; two answers to "is this file part of the workspace" is a bug. If a third
  consumer of the traversal appears, the walk belongs in `epsilon-core` beside `IgnoreSet`,
  and the module doc says so.
- **Do not copy dead logic, and do not copy the comment that explains it.** The new walk
  skips hidden directories unconditionally and its comment states the limitation with the
  reason. A test pins the limitation by name
  (`a_hidden_directory_is_skipped_even_when_a_negation_names_it`) so that the day a matcher
  query can tell the two cases apart, the test is what fails.
- **Fix the tool walker's lies in the same slice.** The clause is deleted from
  `epsilon_tools::util::walk_inner`, the inline comment and `walk_files`'s doc comment are
  rewritten to state the limitation, and the existing test note is left as the durable
  record. Behaviour is unchanged — the clause had no effect — so no test count moves.
- **Sorted directory entries.** The tool walk sorts; the old TUI walk did not. Unstable
  order makes the popup jump while typing and makes a test flaky rather than wrong.

## Gotchas

- **A test can fail because the feature it asserts is impossible, not because the code is
  wrong.** The instinct is to "fix" the walker until the test passes; the honest move is to
  find out which of the two is lying. Here the matcher's own doc comment (`dir/` then
  `!dir/`) documents real git semantics for *matching*, and the walker's comment had
  over-generalized it into a capability the API cannot express.
- **`is_ignored` is a boolean about a path, not about provenance.** Any caller that needs
  "why" rather than "whether" cannot use it, and that is a design signal rather than a bug.

## Verification

Battery `m109a` (`target/gate-m109a.log`, copied to
[`../../audit/evidence/gate-m109a.log`](../../audit/evidence/gate-m109a.log)): **all five
gates exit 0, 1,025 passed / 0 failed / 1 ignored across 59 test binaries** — four more
than M106a's 1,021, which is exactly the four new `at_files` tests. The tool walk's change
moves no count, because deleting a clause that could not change the outcome cannot change
a test.

Focused runs before the battery: `epsilon-tools` 97 unit + 45 integration, `epsilon-tui`
221 (four new `at_files` tests; the first version of the hidden-directory test failed and
is what found the dead logic).

Reinstall: **TUI behaviour changed** (`@` now lists what the tools can actually read), so
the global `epsilon.exe` is reinstalled from this round's tree.

## Limits

- **The escape hatch is still not real.** This slice stops the code and the docs from
  claiming it, and names what a fix needs: a matcher query that distinguishes "re-included"
  from "never mentioned" — `IgnoreSet::is_ignored` cannot, because it folds its ancestor
  walk into one boolean.
- **The two walks can still drift** on what they skip, since the traversal is written twice.
  They agree today by inspection, not by construction; only sharing the traversal would make
  that structural, and that is deferred until a third consumer exists.
- **`@` completion is not covered end-to-end.** The four new tests drive the walk directly;
  no test opens the TUI popup and asserts the rendered rows, so a wiring mistake between
  `at_file_rows` and `at_files::walk` would show up only in a real session.
