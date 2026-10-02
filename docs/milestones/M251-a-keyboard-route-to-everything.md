# M251 — a keyboard route to everything

The window had **two** keys. `Enter`, to send, and `Escape`, to abort. Both belonged to the
composer, and nothing else in this application could be reached without the mouse.

Every file was reachable only by clicking down the file tree — and the tree is the *first*
pane to disappear as the window narrows, which is the exact direction in which a reader
needs the keyboard most. A reader who cannot type a name has to already know where the file
is. That is a browser, not an IDE.

All four reference harnesses have this. `opencode`'s palette is built from its command ids
so `/hotkeys` cannot drift from it; DSH's is a strip above the composer with inline editing;
Hermes' carries its whole 102-command registry. This window had none of it, and now has the
part that earns its place.

## `Ctrl+P`

A search box over the window listing **every file in the workspace** and **the window's own
commands** — the three sidebar views and a tree reload. Files first, then commands, because
that is the order a reader means them in: typing a name wants the file.

Four things it does deliberately:

- **`Enter` and `Escape` are read before the composer, not after.** The composer consumes
  both unconditionally — that is how it sends and aborts — so a palette reading them later
  would be reading events already taken. This is the same mistake as the tab strip's in
  M250, one layer down: *read a key before the widget that wanted it, not after.*
- **The search box takes the keyboard explicitly.** M249's rule is that the composer yields
  to whatever already holds focus, so while the palette is open the composer stops
  requesting focus at all. Without that the two would fight over the caret every frame.
- **A row says whether it is a file or a command.** `sessions` the file and `sessions` the
  command are told apart before either is opened.
- **Truncation is marked.** Past forty rows the palette says how many more there are rather
  than stopping as though there were no more — the rule this repository has applied to every
  list it has built since M247.

Four commands is not thirty. There is no command table in this crate to draw on, and three
entries that demonstrably work are worth more than thirty that do not.

## What the tests caught while it was being written

**A chord is not a key press.** The first version's helper sent `Key` events with no
modifiers, so `Ctrl+P` never registered and the test passed for a reason unrelated to the
palette. The helper now takes modifiers, and the four palette tests are about `Ctrl+P`
specifically.

**A test that could match the wrong row.** The comparison is exact and lives in one helper
(`palette_has`) rather than four spellings, because a palette test that passes because
`alpha` matched `alpha.rs.bak` has measured nothing — and the substring version would have
compiled, passed, and proved less.

## Verification

Battery `m251` — **GATE PASS on attempt 1: 1,817 tests across 74 binaries, 0 failed**
(`audit/evidence/gate-m251.log`). Up from 1,813. `epsilon-desktop` is 101 unit + 21 window.

**Negative control:** with the `Ctrl+P` chord disarmed, all four palette tests fail and the
other 97 pass.

**Reinstall:** none. `epsilon-cli` and `epsilon-tui` are untouched.

## What nothing exercises

- **The palette has never been seen.** Every claim is an offscreen-`Context` measurement of
  egui's real draw path — not a GPU rasterizer, not a screenshot. Position, row height, the
  width clamp, and whether 40 rows are legible at all are **unmeasured against a real
  screen**. This is the fifth desktop milestone to say this; it is now the largest single
  untested surface in the application.
- **Arrow keys move the selection; nothing tests that.** `palette_keys` handles
  `ArrowUp`/`ArrowDown` and clamps to the ends, and no test presses either.
- **Filtering is a substring match, case-insensitive** — not fuzzy, not subsequence. A
  reader who types `capp` will not find `app.rs` the way a fuzzy matcher would have.
  Deliberate: fuzzy matching is a second scoring algorithm with its own ranking to argue
  about, and substring search is predictable on day one.
- **The palette lists directories out.** `visible()` returns them and they are filtered, so
  only files are offered — which means a directory cannot be opened to browse, only the
  files inside it, matching the tree's collapse behaviour.
- **Four commands.** There is no command table, so nothing else is reachable from the
  keyboard: not the model picker, not sessions, not a diff review. Widening this is the next
  obvious slice and wants a real command registry rather than more palette entries.