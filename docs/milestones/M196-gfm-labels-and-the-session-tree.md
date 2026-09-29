# M196 — GFM markdown, one session forest, branch labels, and a `/tree` that sees branches

## What

Six gaps in the pi port closed, all in the "the capability is half there" class rather
than missing outright. Each one had a definition, a serialization, or a test somewhere
— and nothing on the path a user actually takes.

| # | Change | Where |
|---|---|---|
| 1 | Cache percentage no longer rounds a partial hit to a full one | `epsilon-tui/src/ui/format.rs` |
| 2 | "This provider reports no cache traffic" no longer blames a provider for an un-hit cache | `epsilon-core/src/cache_stats.rs` |
| 3 | GFM tables, task lists and strikethrough render | `epsilon-tui/src/markdown.rs` |
| 4 | One session-forest builder, not two | `epsilon-session/src/store.rs`, `epsilon-cli/src/rpc_mode.rs` |
| 5 | `EntryKind::Label` has a writer and a reader | `epsilon-session/src/store.rs`, `epsilon-tui/src/app/keys.rs` |
| 6 | `/tree` shows abandoned branches, and can label, filter and fold | `epsilon-tui/src/app/{,commands,keys,panels,session,view,tests}.rs` |
| 7 | `/tree` search, with pi's AND semantics and the mandatory-row rule | `epsilon-tui/src/app/commands.rs` |
| 8 | a skill can be switched off from the TUI and shows its description | `epsilon-core/src/config.rs`, `epsilon-tui/src/{picker,app/{,commands}}.rs` |
| 9 | editor jump-to-character, on pi's chords | `epsilon-tui/src/app{.rs,/editing.rs,/keys.rs}` |

## Key decisions

**A setting with no handler is a lie in the source.** `ENABLE_STRIKETHROUGH` was on and
no match arm existed, so `~~gone~~` printed with visible tildes and no strike. This is
the `app.clear` defect in a different costume, and the port study predicted it.

**Filtering a tree is not `filter().collect()`.** pi filters a flat list and then
*recalculates the indent from what survived* (`tree-selector.ts:408`), so a row whose
parent was filtered out is **promoted to a root** rather than dropped or left indented
under something invisible. My first implementation returned the original depth.

**A fold is transitive.** pi's second pass marks a row hidden when its parent is folded
*or is itself marked* (`tree-selector.ts:400-407`). Without the second clause you hide
one level, not a subtree.

**Two questions, not one.** "Does the provider report cache fields?" and "has anything
hit the cache?" are different facts; conflating them is how a healthy cache on a
short session gets reported as a provider without caching. `moved_cache_traffic` and
`provider_reports_cache` now answer them separately.

**Chords come from the reference, not from taste.** `shift+l` for the label, `ctrl+←` /
`ctrl+→` for fold-or-move — `keybindings.ts:150-159`. A bare `l` was tried and rejected:
it would collide with a future "navigate left" and does not match pi's muscle memory.

**A tree's visible rows are ONE pipeline.** `visible_rows` = filter -> search ->
mandatory -> folds, with two callers (opening the picker, and `refilter` after a
chord). Two copies would drift invisibly: the picker opens showing rows the chord
handler believes are hidden.

**Search ANDs its tokens, and never hides the leaf.** pi lowercases, splits on
whitespace, and requires **every** token (`tree-selector.ts:393`); the searched text
is label + kind + snippet. The current leaf is visible unconditionally
(`:342-343`) and a textless assistant turn is hidden unless it errored
(`:345-355`) — an error IS text the reader is looking for.

**A setting is a disable-list, not an enable-list.** An enable-list would silently
drop every skill the day a config file gained a `[skills]` section it did not have
before. A disable-list is inert until someone writes to it.

**Match arms are ordered, and a looser guard first shadows a stricter one.** The
`Ctrl+Alt+]` arm has to precede `Ctrl+]`, because the plain guard is also satisfied
by the ALT chord — written the other way round, backward was unreachable and the key
silently did the other thing.

**Matching the reference beats intuition.** I documented and tested that labelling does
*not* move the leaf. It does, and so does pi: `_appendEntry` sets `leafId = entry.id`
unconditionally (`session-manager.ts:1058-1063`). The code was right; the assertion
was wrong.

## Bugs this milestone's tests caught

1. **Four blank lines before every table** — `flush` pushed an empty line at every cell
   boundary.
2. **Only the first body row survived** — row state was never reset per row.
3. **Every bullet list rendered on one line** (`- a - b`) — there was **no
   `Event::End(TagEnd::Item)` arm at all**. Pre-existing; any single-item list hid it.
4. **A silent no-op in the tree picker** — selection looked the seq up inside
   `branch_entries()` (the *active lane*) and jumped to whatever it found there, so
   choosing an off-lane row found nothing and did nothing. Now navigates by id.
5. **A self-deadlock in the label editor** — the store lock was held across
   `reproject_current_branch()`, which locks the same store. See below.
6. **Two tests passing for the wrong reason** — `a_provider_that_never_mentions_caching...`
   used a fake provider that hardcoded `cache_reported: true`, so it never tested a
   silent provider at all.

## The deadlock, because a hang is worse than a failure

```rust
let Ok(mut st) = store.lock() else { return false };
st.set_label(&id, label)?;            // write under the lock
agent.reproject_current_branch()      // locks the SAME store again  -> hang
```

Nothing failed. The test simply stopped making progress — **60+ seconds**, which reads
as a slow machine rather than a bug. It was found only because `cargo test` prints
`test ... has been running for over 60 seconds` and that line was read rather than
skimmed. Fixed by scoping the lock to the write.

## Test-caught design corrections

- **A global flag cannot vary test behaviour here.** The tests in one binary run
  concurrently on separate tasks in one process, so a `static` switch made one test
  poison another. A `Drop` guard did not help; the shared state was the mistake. The
  knob belongs **on the thing the test constructs** — now a field on the fake provider.
- **A test must not hardcode a fixture's shape.** The wire-label test indexed
  `tree[0]["children"]`, but its target entry is a *root*, so the lookup missed. It now
  searches the tree.
- **Negative controls, run and recorded.** Removing the tree promotion and the
  `End(Item)` arm each made the naming test fail for exactly that reason; both restored
  and byte-verified.

## What nothing exercises

- **The search field, the fold/filter glyphs and the jump-mode notice are asserted
  on STATE, never drawn.** `apply_search`, `visible_rows` and `jump_to_char` are
  pure and tested; that the caret renders, that `▸`/`▾` differ on screen, and that
  the `/` field appears in the panel are not. A panel that fails to *render* — wrong
  area, title unpainted, glyph outside the frame — passes every test here.
- ~~**`Ctrl+]` and `Ctrl+Alt+]` are exercised by the mode and the algorithm, never
  through `handle_key`.**~~ **Closed.** `the_backward_jump_chord_arms_backward_not_forward`
  drives both chords and asserts the direction each arms. The arm ORDER is the
  property that made backward unreachable — the plain guard is also satisfied by the
  ALT chord — and a test fails the moment the arms are swapped, where a comment was
  only a promise.
- **The terminal integration suite never opens `/tree`, never arms a jump, and never
  runs `/skill`.** It drives the real binary, but not these paths.
- **A skill toggle takes effect "on the next turn" — nothing tests that.** The write
  and the read-back are tested; the claim that the model stops seeing the skill is
  asserted by nothing, because it depends on re-running discovery.
- **Fold and filter state is per-picker-open.** Reopening `/tree` rebuilds from the
  file, so a fold or filter from a previous picker session is not restored. Nothing
  asserts either way, because nothing observes it.
- **`/settings` is still a notice dump**, deliberately, with a comment naming the
  deferral. Reading pi's `config-selector.ts` showed the reference is a resource
  enable/disable browser rather than a toggles panel, which is why the bounded
  skill toggle was built instead.

## Verification

**`m196-green3` — GATE PASS, 1,520 tests passed, 0 failed** (63 test binaries).
Evidence: [`audit/evidence/gate-m196-green3-PASS.log`](../../audit/evidence/gate-m196-green3-PASS.log),
committed with the milestone that it gates.

Per-binary counts for the crates this milestone touched:

| Crate | Passed |
|---|---|
| `epsilon-tui` (lib) | 419 |
| `epsilon-session` (lib + `session_store`) | 69 |
| `epsilon-core` (`cache_attribution`) | 5 |

Baseline for comparison: 1,495 at the start of the session, so **+25 tests**, all of
them asserting behaviour that did not exist before. The suite total moved from 1,495 to
1,520 with no test removed or weakened — two pre-existing cache tests had their
*assertions corrected* (they were pinned to wording the fix made more precise), and both
corrections are described in the commit messages.

Every one of the following was observed rather than assumed, and each negative control
was run and recorded:

| Claim | How it was checked |
|---|---|
| a partial cache hit is not a full one | control: old rounding fails the 0.996 case naming it |
| a reporting provider is not a silent one | two tests, one per direction |
| tables/task lists/strikethrough render | probe printed the actual rendered rows |
| list items are separate lines | control: removing `End(TagEnd::Item)` prints `• alpha • beta • gamma` on one line |
| a filter promotes an orphaned row | control: removing the promotion fails the test that names it |
| an abandoned branch is visible | asserted present AND marked off-lane, so a flat dump cannot pass |
| the label reaches the file | close/reopen, so a map cannot pass |
| a label lands on its TARGET | asserted against the leaf, so "some label exists" cannot pass |

## Limits

- The TUI-visible consequences of a moved leaf after labelling are asserted through
  `handle_key`, not measured on a real console.
- Filters use the row's resolved `kind` tag rather than re-deriving from the entry
  enum. One representation is deliberate — two would be free to disagree — but it means
  a new entry kind needs its tag added in `timeline_kind_tag` before a filter can see it.
