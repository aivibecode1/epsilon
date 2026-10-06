# Epsilon terminal UI — behavioural source inventory

## What this file is

The **our-side** half of the parity matrix [`docs/parity/pi-tui.md`](../../docs/parity/pi-tui.md). The
reference half is [`parity-pi-tui-reference.md`](parity-pi-tui-reference.md), which reads Pi's
interactive terminal mode out of the vendored tree under `references/`. This one reads **our** terminal
UI out of `crates/epsilon-tui` and the command registry out of `crates/epsilon-contracts/src/command.rs`,
and answers the question the matrix asks of our side: *what does this program actually do, where in the
source, and what proves it.*

**Every row cites a line in the tree. No citation, no row.**

| Fact | Value | Where it comes from |
|---|---|---|
| The crate | `epsilon-tui`, ratatui + crossterm | `crates/epsilon-tui/Cargo.toml:1`, `crates/epsilon-tui/src/lib.rs:1-10` |
| Its shape | a pure event reducer (`state::UiState`) plus a thin terminal runtime (`app::run`) | `crates/epsilon-tui/src/lib.rs:3` |
| Rows | 110 ids, `E-001` … `E-110`, no gaps | this file, §4 |
| Rows reconstructed from source | **110 of 110**; none written as `not established` | §4, §5 |
| Rows whose behaviour no test in the tree pins | **8** | §4, each marked `Test: **none found**` |
| Citations | 697 line-anchored, all machine-validated | §9 |
| Slash items offered to a reader | **43** | `crates/epsilon-contracts/src/command.rs:47` |
| Command heads the parser knows | **37** | `crates/epsilon-contracts/src/command.rs:383` |
| Alias spellings | **8** | `crates/epsilon-contracts/src/command.rs:458` |
| `enum Command` variants | **39** | `crates/epsilon-contracts/src/command.rs:252` |
| Keybinding ids with a default chord | **24** | `crates/epsilon-tui/src/keybindings.rs:27` |
| Command-palette entries | **7** | `crates/epsilon-tui/src/picker.rs:541` |
| What the desktop answers of the 37 heads | **7 run, 30 refused by name** | `crates/epsilon-desktop/src/commands.rs:449`, `:410` |

### Why this file was re-derived rather than transcribed

The first investigator's session ended before it wrote its file. A forked agent starts with **no prior
turns**, so the row-level detail was not available to transcribe and every row below was rebuilt by
opening the source. That is the better outcome anyway, and `docs/STATUS.md:241-246` records the same
reasoning for our other half: *a citation nobody re-opened is not evidence*, and the rule this repository
follows is that **a summary of an audit is not the audit**.

The cost of the loss is stated rather than hidden: the area and id-range assignment below came from the
survey and is the one thing in this file that was **not** re-derived, because the ids are an addressing
scheme rather than a claim about the code. Four behaviours that were in the survey's plan had no row slot
left after the re-derivation — they are listed in §5 rather than invented into an id.

### Paths read

- `crates/epsilon-tui/src/` — every `.rs` file, 49 files
- `crates/epsilon-tui/Cargo.toml`
- `crates/epsilon-contracts/src/command.rs` (the shared registry both front ends read)
- `crates/epsilon-desktop/src/commands.rs` (the desktop's capability answer, for §2.4)
- `crates/epsilon-cli/src/main.rs` (mode dispatch, theme resolution, `--demo`)
- `crates/epsilon-cli/tests/terminal.rs` (the real-pty suite)
- `crates/epsilon-core/src/{lib.rs,walk.rs,config.rs}`, `crates/epsilon-safety/src/{gate.rs,channel_ui.rs}`,
  `crates/epsilon-llm/src/auth.rs`, `crates/epsilon-runtime/src/{lib.rs,text_safety.rs}`,
  `crates/epsilon-runtime/tests/goal_round_gates.rs` — where a behaviour genuinely lives outside the TUI
- `docs/STATUS.md:223-352` (the M313 section, checked against source rather than believed)
- `audit/evidence/tui-frames-2026-10-04-m312/*.txt` — 21 captured frames (7 scenarios x 3 sizes), cited
  in §6 as what a reader actually sees

### How a row is shaped

| Field | What it holds |
|---|---|
| **behaviour** | one or two sentences, as a person at the terminal would notice it |
| **evidence** | `path:line` for the logic, and `path:line` for the draw where the draw is a separate path |
| **test** | the name of a test that exists in the tree, its `file:line`, and whether it asserts on the **frame** or on state only |
| **observe** | what a reader sees, or what happens to their keystroke |

`frame` means the test drives the production draw path through `render_rows` / `render_buffer`
(`crates/epsilon-tui/src/app/tests_support.rs:109` and `:119`, which call `draw` itself) and reads the
screen back. `state only` means it asserts on a field. The distinction is load-bearing: it is the
difference between "a user sees this" and "a variable holds this", and this repository has shipped
features that were correct, tested, and never rendered.

Rows marked **`none found`** are behaviours the source establishes and **no test in the tree pins**. They
are not reconstruction failures; they are gaps, and §7 collects them.

---

## 1. Module map

Verified by reading, not by the survey's claim.

### `src/app/` — runtime, key handling, composition

Nine runtime modules plus six test-only ones, declared at `crates/epsilon-tui/src/app.rs:1440-1872`:

| Module | Owns |
|---|---|
| `app.rs` | `App`, the terminal lifecycle, and the single `draw` entry point |
| `event.rs` | the run loop between the first frame and the last (`crates/epsilon-tui/src/app.rs:1437-1441`) |
| `keys.rs` | `handle_key`: one match that routes every keystroke |
| `commands.rs` | slash-command execution |
| `panels.rs` | the draw path for the transcript and every overlay |
| `view.rs` | scrolling, the hint set, the search, the geometry |
| `editing.rs` | cursor motion, insert, delete, undo, kill |
| `completion.rs` | the `/` and `@` popups |
| `editor.rs`, `login.rs`, `session.rs` | `$EDITOR`, the masked key prompt, session files |

Test-only, all `#[cfg(test)]`: `composer_viewport_tests.rs`, `goal_tests.rs`, `startup_tests.rs`,
`tests_support.rs`, `frame_dump.rs`, `golden.rs`, `panic_tests.rs`, `tests.rs`.

### `src/ui/` — presentation

Twelve modules, declared at `crates/epsilon-tui/src/ui/mod.rs:31-41`, with the ownership table in that
file's own module doc (`crates/epsilon-tui/src/ui/mod.rs:13-30`): `theme`, `layout`, `metrics`,
`format`, `header`, `transcript`, `card`, `composer`, `footer`, `overlay`, `shell`, `cost`.

**The discipline is measurable.** The module doc claims "nothing in here reaches for application state"
(`crates/epsilon-tui/src/ui/mod.rs:3-4`). A grep for `crate::app::` across `crates/epsilon-tui/src/ui/*.rs`
returns **no matches**; the only crate-root references are 4 to `crate::keybindings::` (the footer
resolving hints) and 3 to `crate::state::` (the shared line types). No widget can reach `App`.

### Top level — the data layer

`state.rs` (the `UiState` reducer), `picker.rs` (palette, slash items re-export, the session browser),
`keybindings.rs`, `search.rs`, `clipboard.rs`, `markdown.rs`, `wrap.rs`, `undo.rs`, `kill_ring.rs`,
`editor_ops.rs`, `at_files.rs`, `trust_store.rs`, `render_cache.rs`, `text_safety.rs`, `scroll.rs`,
`fuzzy.rs`, `latex.rs`, `git.rs`, `bench.rs`, `renderers/` — declared at
`crates/epsilon-tui/src/lib.rs:12-37`.

### Two structural facts the matrix will lean on

1. **`crates/epsilon-tui/tests/` does not exist.** Every test in this crate is an in-crate
   `#[cfg(test)]` module. A grep counts **534** `#[test]` / `#[tokio::test]` attributes under
   `crates/epsilon-tui/src/`. `epsilon-tui` is the only crate in the workspace with no `tests/`
   directory — the other eleven have one (`epsilon-cli` 5, `epsilon-core` 17, `epsilon-safety` 4,
   `epsilon-tools` 3, `epsilon-llm` 5, `epsilon-ext` 3, `epsilon-runtime` 2, `epsilon-desktop` 2,
   `epsilon-session` 2, `epsilon-mcp` 1, `epsilon-lsp` 1).
2. **`golden.rs` uses no golden files.** There is no `include_str!`, no `include_bytes!`, no file read
   and no `.txt` fixture in `crates/epsilon-tui/src/app/golden.rs`. It drives the production `draw`
   through a `TestBackend` and asserts **structure** — that the frame is exactly the terminal size, that
   a row is never wider than the terminal, that a role is distinguishable without colour — and says why
   at `crates/epsilon-tui/src/app/golden.rs:17-19`: *"a golden test that pins every character fails on
   every deliberate copy change and gets rewritten without being read."*

---

## 2. The shared command registry

The command half of the registry is **not** in the terminal. It lives in `epsilon-contracts`, below both
front ends, and `epsilon-tui` re-exports every item from `picker`
(`crates/epsilon-tui/src/picker.rs:31`) so no call site changed. The move and its reason are the module
doc at `crates/epsilon-contracts/src/command.rs:1-33`.

### 2.1 The four tables, and the three tests that hold them together

| Table | Count | Declared | Guarded by |
|---|---|---|---|
| `SLASH_ITEMS` — what `/help` prints and `/` suggests | 43 | `crates/epsilon-contracts/src/command.rs:47` | `every_offered_command_is_reachable_from_slash_items` (`crates/epsilon-contracts/src/command.rs:606`) |
| `COMMAND_HEADS` — one entry per command | 37 | `crates/epsilon-contracts/src/command.rs:383` | `every_head_in_command_heads_is_parsed` (`crates/epsilon-contracts/src/command.rs:586`) |
| `ALIAS_HEADS` — alternative spellings | 8 | `crates/epsilon-contracts/src/command.rs:458` | `the_parser_does_not_know_a_command_the_list_does_not` (`crates/epsilon-contracts/src/command.rs:632`) |
| `enum Command` — what the parser returns | 39 | `crates/epsilon-contracts/src/command.rs:252` | — |

37 heads + 8 aliases = **45 spellings**, which is the "45" in `docs/STATUS.md:342`.

The reverse-drift check reads `parse_command`'s own body out of the file with `include_str!` and extracts
its quoted tokens (`crates/epsilon-contracts/src/command.rs:644-678`), so a command added to the parser and forgotten in the sweep
fails rather than running untested. It had to be widened once: its token rule originally accepted
lowercase, `_` and `:` only, so `scoped-models` and `?` were invisible to it (`crates/epsilon-contracts/src/command.rs:663-673`).

### 2.2 The 37 command heads

`parse_command` is a hand-written `match`, and that is a recorded decision, not a leftover
(`crates/epsilon-contracts/src/command.rs:26-33`): the grammar has four arities. Desktop column: **run** = the window answers it
(`crates/epsilon-desktop/src/commands.rs:449`); **refused** = refused by name, with a sentence.

| # | Head | `Command` variant | Parser | Terminal? | Desktop |
|---|---|---|---|---|---|
| 1 | `model` | `Models` bare / `Model(String)` with a tail | `crates/epsilon-contracts/src/command.rs:484`, `:494` | yes | refused |
| 2 | `think` | `Think(Option<String>)` | `crates/epsilon-contracts/src/command.rs:495` | yes | refused |
| 3 | `compact` | `Compact` | `crates/epsilon-contracts/src/command.rs:500` | yes | **run** |
| 4 | `theme` | `Theme(Option<String>)` | `crates/epsilon-contracts/src/command.rs:501` | yes | **run** |
| 5 | `branch` | `Timeline` | `crates/epsilon-contracts/src/command.rs:506` | yes | refused |
| 6 | `tree` | `Tree` | `crates/epsilon-contracts/src/command.rs:507` | yes | refused |
| 7 | `goal` | `Goal(Option<String>)` | `crates/epsilon-contracts/src/command.rs:508` | yes | **run** |
| 8 | `fork` | `Fork` | `crates/epsilon-contracts/src/command.rs:513` | yes | refused |
| 9 | `clone` | `Clone` | `crates/epsilon-contracts/src/command.rs:514` | yes | refused |
| 10 | `new` | `New` | `crates/epsilon-contracts/src/command.rs:515` | yes | refused |
| 11 | `resume` | `Resume(String)` | `crates/epsilon-contracts/src/command.rs:516` | yes | refused |
| 12 | `sessions` | `Sessions` | `crates/epsilon-contracts/src/command.rs:517` | yes | **run** |
| 13 | `editor` | `Editor` | `crates/epsilon-contracts/src/command.rs:518` | yes | refused |
| 14 | `session` | `Session` | `crates/epsilon-contracts/src/command.rs:519` | yes | **run** |
| 15 | `trust` | `Trust(Option<String>)` | `crates/epsilon-contracts/src/command.rs:520` | yes | refused |
| 16 | `reload` | `Reload` | `crates/epsilon-contracts/src/command.rs:525` | yes | refused |
| 17 | `export` | `Export(Option<String>)` | `crates/epsilon-contracts/src/command.rs:526` | yes | refused |
| 18 | `import` | `Import(String)` | `crates/epsilon-contracts/src/command.rs:531` | yes | refused |
| 19 | `restore` | `Restore(String)` | `crates/epsilon-contracts/src/command.rs:532` | yes | refused |
| 20 | `share` | `Share` | `crates/epsilon-contracts/src/command.rs:533` | yes | refused |
| 21 | `copy` | `Copy` | `crates/epsilon-contracts/src/command.rs:534` | yes | refused |
| 22 | `name` | `Name(Option<String>)` | `crates/epsilon-contracts/src/command.rs:535` | yes | refused |
| 23 | `changelog` | `Changelog` | `crates/epsilon-contracts/src/command.rs:540` | yes | refused |
| 24 | `tasks` | `Tasks` | `crates/epsilon-contracts/src/command.rs:541` | yes | **run** |
| 25 | `hotkeys` | `Hotkeys` | `crates/epsilon-contracts/src/command.rs:542` | yes | refused |
| 26 | `settings` | `Settings` | `crates/epsilon-contracts/src/command.rs:543` | yes | refused |
| 27 | `scoped_models` | `ScopedModels` | `crates/epsilon-contracts/src/command.rs:544` | yes | refused |
| 28 | `clear` | `Clear` | `crates/epsilon-contracts/src/command.rs:546` | yes | **run** |
| 29 | `expand` | `Expand` | `crates/epsilon-contracts/src/command.rs:547` | yes | refused |
| 30 | `skills` | `Skills` | `crates/epsilon-contracts/src/command.rs:548` | yes | refused |
| 31 | `skill` | `Skill(String)` | `crates/epsilon-contracts/src/command.rs:549` | yes | refused |
| 32 | `templates` | `Templates` | `crates/epsilon-contracts/src/command.rs:550` | yes | refused |
| 33 | `search` | `Search(String)` | `crates/epsilon-contracts/src/command.rs:551` | yes | refused |
| 34 | `logout` | `Logout(String)` | `crates/epsilon-contracts/src/command.rs:575` | yes | refused |
| 35 | `exit` | `Quit` | `crates/epsilon-contracts/src/command.rs:576` | yes | refused |
| 36 | `help` | `Help` | `crates/epsilon-contracts/src/command.rs:545` | yes | refused |
| 37 | `login` | `Login { provider, key }` | `crates/epsilon-contracts/src/command.rs:552` | yes | refused |

**7 run, 30 refused.** `39` variants against `37` heads: `Models` and `Model` are the two arities of one
head, `Login` is a struct variant, and `Unknown(String)` is not a command.

### 2.3 The 8 aliases

| Alias | For | Line |
|---|---|---|
| `models` | `model` | `crates/epsilon-contracts/src/command.rs:459` |
| `timeline` | `branch` | `crates/epsilon-contracts/src/command.rs:460` |
| `auth` | `login` | `crates/epsilon-contracts/src/command.rs:461` |
| `connect` | `login` | `crates/epsilon-contracts/src/command.rs:462` |
| `quit` | `exit` | `crates/epsilon-contracts/src/command.rs:463` |
| `q` | `exit` | `crates/epsilon-contracts/src/command.rs:464` |
| `scoped-models` | `scoped_models` | `crates/epsilon-contracts/src/command.rs:467` |
| `?` | `help` | `crates/epsilon-contracts/src/command.rs:468` |

Aliases are deliberately absent from `COMMAND_HEADS` (one entry per command) and are driven by the
all-commands sweep through the command they alias (`crates/epsilon-contracts/src/command.rs:449-450`).

### 2.4 The desktop's answer, and why it is derived

`refused_heads()` is `COMMAND_HEADS` minus `window_runs(&command)` (`crates/epsilon-desktop/src/commands.rs:410-418`):
there is no second list to keep in step, and the subtraction is a consequence rather than a count written
by hand. `SUPPORTED` is the only hand-written statement of the window's capability in the tree
(`crates/epsilon-desktop/src/commands.rs:423`), holding exactly seven heads — `clear`, `compact`, `goal`,
`session`, `sessions`, `tasks`, `theme` (`crates/epsilon-desktop/src/commands.rs:449-451`).

Every refusal is by name, for both spellings of every head, asserted through the same `refusal_for` the
window calls: `every_refused_head_is_refused_by_name` (`crates/epsilon-desktop/src/commands.rs:471`). Zero
heads are silently dropped.

---

## 3. Keybindings

`keybindings::DEFAULTS` (`crates/epsilon-tui/src/keybindings.rs:27`) holds **24** ids. Every one has a
`keybindings.matches(...)` arm in `app/keys.rs`, and two more ids are handled **without** a default
chord. The survey's claim that every id except `app.interrupt` had a handler was true before M313 and is
**no longer true** — see §8.

| id | default chord(s) | declared | handler |
|---|---|---|---|
| `app.interrupt` | `escape` | `crates/epsilon-tui/src/keybindings.rs:28` | `crates/epsilon-tui/src/app/keys.rs:1346` (table-read since M313) |
| `app.exit` | `ctrl+q` | `crates/epsilon-tui/src/keybindings.rs:37` | `crates/epsilon-tui/src/app/keys.rs:653` |
| `app.clear` | `ctrl+c` | `crates/epsilon-tui/src/keybindings.rs:38` | `crates/epsilon-tui/src/app/keys.rs:649` |
| `app.model.select` | `ctrl+m` | `crates/epsilon-tui/src/keybindings.rs:39` | `crates/epsilon-tui/src/app/keys.rs:1424` |
| `app.palette` | `ctrl+p` | `crates/epsilon-tui/src/keybindings.rs:40` | `crates/epsilon-tui/src/app/keys.rs:1442` |
| `app.thinking.cycle` | `ctrl+t` | `crates/epsilon-tui/src/keybindings.rs:41` | `crates/epsilon-tui/src/app/keys.rs:1435` |
| `app.model.cycleForward` | `f2` | `crates/epsilon-tui/src/keybindings.rs:42` | `crates/epsilon-tui/src/app/keys.rs:1459` |
| `app.model.cycleBackward` | `shift+f2` | `crates/epsilon-tui/src/keybindings.rs:43` | `crates/epsilon-tui/src/app/keys.rs:1460`, `:1473` |
| `app.tools.expand` | `ctrl+x` | `crates/epsilon-tui/src/keybindings.rs:44` | `crates/epsilon-tui/src/app/keys.rs:1804` |
| `app.transcript.pageUp` | `pageup` | `crates/epsilon-tui/src/keybindings.rs:50` | `crates/epsilon-tui/src/app/keys.rs:1642` |
| `app.transcript.pageDown` | `pagedown` | `crates/epsilon-tui/src/keybindings.rs:51` | `crates/epsilon-tui/src/app/keys.rs:1646` |
| `app.transcript.prevPrompt` | `ctrl+up`, `ctrl+shift+up` | `crates/epsilon-tui/src/keybindings.rs:58` | `crates/epsilon-tui/src/app/keys.rs:1660` |
| `app.transcript.nextPrompt` | `ctrl+down`, `ctrl+shift+down` | `crates/epsilon-tui/src/keybindings.rs:59-62` | `crates/epsilon-tui/src/app/keys.rs:1707` |
| `app.transcript.live` | `G`, `g` | `crates/epsilon-tui/src/keybindings.rs:73` | `crates/epsilon-tui/src/app/keys.rs:1819` |
| `app.transcript.top` | `ctrl+home`, `ctrl+shift+home` | `crates/epsilon-tui/src/keybindings.rs:123` | `crates/epsilon-tui/src/app/keys.rs:1699` |
| `app.transcript.bottom` | `ctrl+end`, `ctrl+shift+end` | `crates/epsilon-tui/src/keybindings.rs:124` | `crates/epsilon-tui/src/app/keys.rs:1703` |
| `app.message.copy` | `f4` | `crates/epsilon-tui/src/keybindings.rs:140` | `crates/epsilon-tui/src/app/keys.rs:1004`, `:1678` |
| `app.live.refresh` | `ctrl+g` | `crates/epsilon-tui/src/keybindings.rs:141` | `crates/epsilon-tui/src/app/keys.rs:1317` |
| `tui.editor.undo` | `ctrl+z` | `crates/epsilon-tui/src/keybindings.rs:142` | `crates/epsilon-tui/src/app/keys.rs:1493` |
| `tui.editor.redo` | `ctrl+.` | `crates/epsilon-tui/src/keybindings.rs:143` | `crates/epsilon-tui/src/app/keys.rs:1499` |
| `tui.input.newLine` | `ctrl+j` | `crates/epsilon-tui/src/keybindings.rs:144` | `crates/epsilon-tui/src/app/keys.rs:1776` |
| `tui.input.submit` | `enter` | `crates/epsilon-tui/src/keybindings.rs:145` | `crates/epsilon-tui/src/app/keys.rs:1410` (table-read since M313) |
| `app.clipboard.paste` | `ctrl+v`, `shift+insert` | `crates/epsilon-tui/src/keybindings.rs:146` | `crates/epsilon-tui/src/app/keys.rs:1755` |
| `app.editor.external` | `ctrl+o`, `alt+e` | `crates/epsilon-tui/src/keybindings.rs:149` | `crates/epsilon-tui/src/app/keys.rs:1783` |
| `app.transcript.halfUp` | **none on purpose** | — | `crates/epsilon-tui/src/app/keys.rs:1686` |
| `app.transcript.halfDown` | **none on purpose** | — | `crates/epsilon-tui/src/app/keys.rs:1691` |

**The two ids with a handler and no default** are recorded at `crates/epsilon-tui/src/keybindings.rs:93-122`: three chords were
tried for a half page and each is already the composer's — bare letters are text, `ctrl+u`/`ctrl+d` are
kill-to-start and delete-forward, `ctrl+j` is newline. pi leaves the same key unbound for the same reason.
The invariant that drives it is at `crates/epsilon-tui/src/keybindings.rs:110-114`: every id in `DEFAULTS` must have a printable
chord, because the footer prints chords and a binding it cannot name is a binding a user cannot find.
Pinned by `a_hint_label_parses_back_to_the_chord_it_names` (`crates/epsilon-tui/src/keybindings.rs:493`).

**One chord names one action.** `no_chord_is_bound_to_two_ids_by_default`
(`crates/epsilon-tui/src/app/tests.rs:1704`) walks the *effective* table, so a user's own
`keybindings.json` is covered too.

---

## 4. Behaviour rows

### 4.1 Layout — `E-001` … `E-007`

**E-001 — Five stacked bands, one rule, below the composer.**
Behaviour: every frame is split in a fixed order — a one-row header, the transcript (which absorbs all
slack), the composer, exactly one full-width rule, and a one-row footer. There is no divider above the
transcript; the surviving rule sits *below* the composer.
Evidence: `crates/epsilon-tui/src/ui/layout.rs:84` (the vertical `Layout`, five constraints);
draw `crates/epsilon-tui/src/ui/shell.rs:92` (`draw_transcript(f, regions.transcript);`).
Test: `the_shell_shows_a_header_a_body_and_a_footer_at_every_supported_size` —
`crates/epsilon-tui/src/ui/shell.rs:194` (frame). Also `the_one_rule_sits_between_the_transcript_and_the_composer`
— `crates/epsilon-tui/src/app/golden.rs:1451` (frame), and `regions_tile_the_frame_without_gaps_or_overlap`
— `crates/epsilon-tui/src/ui/layout.rs:131`.
Observe: one status row, the prompt, one row of `─`, the key hints. Resizing never adds a second rule.
Captured: `audit/evidence/tui-frames-2026-10-04-m312/idle-with-content.80x24.txt:26`.

**E-002 — The composer is capped at 8 rows and a third of the screen.**
Behaviour: the composer takes `1..=min(8, height/3)` text rows, so a pasted document can never claim the
conversation's space.
Evidence: `crates/epsilon-tui/src/ui/layout.rs:81` (`let composer = composer_rows.clamp(1, Self::composer_cap(area.height));`);
draw `crates/epsilon-tui/src/ui/shell.rs:99`.
Test: `the_composer_never_takes_the_screen` — `crates/epsilon-tui/src/ui/layout.rs:186` (state only);
frame companion `a_long_transcript_never_pushes_the_composer_off_the_screen` — `crates/epsilon-tui/src/app/golden.rs:449`.
Observe: paste 400 lines; the prompt grows to at most 8 rows (4 on a 12-row window) and the transcript keeps the rest.

**E-003 — The transcript is the last region to lose rows.**
Behaviour: the transcript is the only band with a floor — `Constraint::Min(1)` rather than `Fill`, the
reason the comment at `crates/epsilon-tui/src/ui/layout.rs:82-83` gives is that `Fill` hands the body a zero-height rectangle on a
short screen and the body silently disappears. Chrome yields before content.
Evidence: `crates/epsilon-tui/src/ui/layout.rs:86` (`Constraint::Min(1),`); draw `crates/epsilon-tui/src/ui/shell.rs:84`.
Test: `the_transcript_keeps_the_space_it_needs_at_every_supported_size` — `crates/epsilon-tui/src/ui/layout.rs:174`.
Observe: squash the window; the composer's spare rows go first and the conversation is still readable.

**E-004 — Below 44x10 the frame stops being a layout and says so.**
Behaviour: under `MIN_WIDTH` (44) or `MIN_HEIGHT` (10) the whole frame is replaced by one centred
warning naming both the required and the current size. No header, no composer, no partial transcript.
A pending approval is still named, so a blocked run is not mistaken for a hang.
Evidence: `crates/epsilon-tui/src/ui/layout.rs:112` (`area.width < MIN_WIDTH || area.height < MIN_HEIGHT`);
draw `crates/epsilon-tui/src/ui/shell.rs:120`; the approval line `crates/epsilon-tui/src/app.rs:1605`
and its advice `:1611`.
Test: `a_screen_below_the_floor_says_so_and_draws_nothing_else` — `crates/epsilon-tui/src/ui/shell.rs:229` (frame);
`below_the_floor_the_frame_says_so_instead_of_drawing_a_broken_layout` — `crates/epsilon-tui/src/app/golden.rs:474` (frame).
Observe: drag to 40x8 and everything becomes `terminal too small — widen to 44x10 (now 40x8)`.

**E-005 — The draft is wrapped to the width the composer actually draws at.**
Behaviour: the composer's text width is the frame width minus its own gutter, computed once and used both
to wrap the draft and to place the caret, so the caret sits in the column the typed text occupies.
Evidence: `crates/epsilon-tui/src/ui/shell.rs:133`; draw `crates/epsilon-tui/src/app.rs:1622`.
Test: `the_composer_width_matches_what_the_composer_draws` — `crates/epsilon-tui/src/ui/shell.rs:238` (frame).
Observe: type a line that exactly fills the row at 44, 80 and 120 columns; the last character lands in the final column.

**E-006 — Chrome yields by whole part, never by fragment.**
Behaviour: as the terminal narrows the header drops whole measurements from the tail (the model name and
its `offline` qualifier are pinned and go last) and the footer keeps the workspace location and drops
whole hints. No half-badge ever reaches the screen.
Evidence: `crates/epsilon-tui/src/ui/header.rs:160` (`match keep.iter().rposition(|p| !p.pinned) {`);
draw `crates/epsilon-tui/src/ui/footer.rs:425`.
Test: `a_narrow_terminal_drops_whole_gauges_rather_than_cutting_one` — `crates/epsilon-tui/src/ui/header.rs:377` (frame);
`no_footer_row_ever_prints_half_a_hint_or_touches_the_location` — `crates/epsilon-tui/src/ui/footer.rs:672` (frame);
`the_position_badge_is_never_half_written_at_any_width` — `crates/epsilon-tui/src/ui/footer.rs:607` (frame).
Observe: gauges disappear one at a time. Captured: `audit/evidence/tui-frames-2026-10-04-m312/brand-new.60x20.txt:23`
reads `insert · enter send · pageup · ctrl+j ...` — whole hints, dropped from the far end.

**E-007 — A status row is re-measured and trimmed on the way out.**
Behaviour: both status rows are measured a second time as they leave the widget and trimmed at a span
boundary if they still exceed the width — a backstop for a width table that disagrees with ratatui's.
Evidence: `crates/epsilon-tui/src/ui/layout.rs:407`; draw `crates/epsilon-tui/src/ui/layout.rs:411`.
Test: `nothing_is_drawn_past_the_row_width` — `crates/epsilon-tui/src/ui/footer.rs:840` (frame);
the same clamp on the header at `crates/epsilon-tui/src/ui/header.rs:73`.
Observe: drag to 24 and 44 columns; the rows shorten in place and the conversation never shifts.

### 4.2 Transcript — `E-008` … `E-010`

**E-008 — A two-column gutter on every transcript row, banded to the edge.**
Behaviour: every row opens with a two-column gutter carrying that row's role glyph, so all bodies align at
column 2. On a terminal with surfaces a banded row's fill runs to the right edge; an unbanded row is
deliberately left unpadded.
Evidence: `crates/epsilon-tui/src/ui/transcript.rs:149`; draw `crates/epsilon-tui/src/app/panels.rs:412`.
Test: `a_user_turn_carries_its_gutter_and_band_on_a_real_frame` — `crates/epsilon-tui/src/app/golden.rs:558` (frame);
`a_wrapped_banded_message_keeps_every_row_inside_the_frame` — `crates/epsilon-tui/src/app/golden.rs:621` (frame).
Observe: your own turns read as a filled band reaching the right edge; under `NO_COLOR` the fill and the
trailing blanks go, the two-column margin does not.

**E-009 — Role is carried by the gutter glyph, never by colour alone.**
Behaviour: under `NO_COLOR` or on a 16-colour terminal all seven roles still have distinct left margins —
including the assistant's answer versus the model's reasoning trace, and a pending tool call versus a notice.
Evidence: `crates/epsilon-tui/src/ui/transcript.rs:70`; draw `crates/epsilon-tui/src/ui/transcript.rs:142`.
Test: `an_answer_and_its_reasoning_are_distinguishable_on_a_real_frame_without_colour` — `crates/epsilon-tui/src/app/golden.rs:766` (frame);
`every_role_is_distinguishable_without_colour` — `crates/epsilon-tui/src/ui/transcript.rs:268`.
Observe: run in a plain console or with `NO_COLOR=1`; the reasoning trace and the answer still begin with
different characters. This is `AGENTS.md` §8's "never rely on colour alone", met in a terminal.

**E-010 — A blank transcript is replaced by the first-run screen.**
Behaviour: with nothing to show, the transcript region carries the welcome panel — wordmark, one
instruction, the three prefixes the composer reads, and the session name only when there is one — and it
disappears the moment a real conversation exists. It never restates the header's model or context.
Evidence: `crates/epsilon-tui/src/ui/transcript.rs:234`; draw `crates/epsilon-tui/src/ui/shell.rs:90`.
Test: `an_empty_session_gets_the_first_run_screen_inside_a_well_formed_frame` — `crates/epsilon-tui/src/app/golden.rs:509` (frame).
Observe: on first launch you read `E P S I L O N`, `Describe a task, then press enter.` and
`@ files · ! shell · / commands`. Captured: `audit/evidence/tui-frames-2026-10-04-m312/brand-new.60x20.txt:6-10`.

### 4.3 Render — `E-011` … `E-012`

**E-011 — One frame is assembled in one place, dialog last.**
Behaviour: `draw` measures the transcript's rectangle *before* painting the chrome, so the footer's
position badge and the rows the body paints come from one arithmetic; the shell then paints header, body,
composer, rule and footer; overlays go on top and the approval dialog is painted last, because it is the
one dialog that can answer keys. Below the layout floor the draw refuses entirely and clears the record
of which ask was painted, so no key can authorise work the user never saw.
Evidence: `crates/epsilon-tui/src/app.rs:1634`; draw `crates/epsilon-tui/src/app.rs:1670`, `:1719`.
Test: `a_full_frame_is_exactly_the_terminal_size_at_every_supported_width` — `crates/epsilon-tui/src/app/golden.rs:371` (frame).
Observe: at 44x10, 60x20, 80x24, 100x30 and 160x48 the screen is exactly `height` rows of exactly `width` columns.

**E-012 — Each line is wrapped once; a frame clones only the visible window.**
Behaviour: every transcript line is parsed, card-rendered and wrapped exactly once per width, keyed by its
monotonic id; row offsets are prefix sums, so a frame binary-searches for the window and clones only those
rows. Cost per frame is O(visible), independent of transcript length. A width, palette or expand-toggle
change invalidates everything; a named restale invalidates only that card.
Evidence: `crates/epsilon-tui/src/render_cache.rs:377`; draw `crates/epsilon-tui/src/app/panels.rs:396`.
Test: `width_change_invalidates_everything` — `crates/epsilon-tui/src/render_cache.rs:730`;
`a_restaled_card_is_rebuilt_when_named_and_kept_when_not` — `crates/epsilon-tui/src/render_cache.rs:805`.
Observe: drag the window edge mid-session; every row is re-wrapped and still fits exactly.

### 4.4 Cards — `E-013` … `E-020`

**E-013 — A tool call is one card: a title line, then its body.**
Behaviour: a tool call opens a card whose first row is the title — the tool's name, its headline, its
status as a word — and every row beneath is the body. A result settles the call's own card rather than
adding a second one.
Evidence: `crates/epsilon-tui/src/ui/card.rs:213`; draw `crates/epsilon-tui/src/render_cache.rs:141`.
Test: `a_tool_result_is_a_card_whose_title_carries_name_status_and_duration` — `crates/epsilon-tui/src/app/golden.rs:1784` (frame).
Observe: `bash $ cargo test · pending` then the output on the rows beneath. Captured:
`audit/evidence/tui-frames-2026-10-04-m312/tool-cards.80x24.txt:6-10`.

**E-014 — ctrl+x flips the global expand-all toggle *and* records the newest card's own state.**
Behaviour: a card is open when the user decided that card's state by hand, and otherwise when the global
toggle is on. `ctrl+x` does both, so a hand-opened card stays open after the toggle goes back off. The key
path and the draw path read the same rule rather than each holding a copy.
Evidence: `crates/epsilon-tui/src/render_cache.rs:72` (`self.explicit(id).unwrap_or(self.expand_all)`);
draw `crates/epsilon-tui/src/app/view.rs:71` (`.is_open(id, has_body)`).
Test: `ctrl_x_opens_the_newest_card_and_leaves_the_others_alone` — `crates/epsilon-tui/src/app/golden.rs:2485` (frame);
`the_drawn_and_recorded_answers_to_an_open_card_are_the_same_answer` — `crates/epsilon-tui/src/app/tests.rs:1918` (sweeps every input the rule depends on).
Observe: one ctrl+x removes only the newest card's `… N more lines` row; a second press puts the toggle back
off and the hand-opened card is *still* open, which a single boolean cannot express.

**E-015 — A collapsed body is capped at twelve rows and the cap names the key that opens it.**
Behaviour: a collapsed card's body is cut to twelve visible rows, followed by one extra row reading
`… 29 more lines (ctrl+x to open)` — the true count and the live text of the binding. An expanded card is
not capped, so it never prints an affordance telling you to press the key you just pressed.
Evidence: `crates/epsilon-tui/src/ui/card.rs:246`; draw `crates/epsilon-tui/src/ui/card.rs:290`;
the constant `crates/epsilon-tui/src/ui/card.rs:70` (`pub const BODY_ROWS: usize = 12;`).
Test: `a_long_diff_is_capped_and_says_which_key_opens_it` — `crates/epsilon-tui/src/app/golden.rs:2423` (frame).
Observe: a 41-row diff draws twelve body rows plus the marker; `new line 19` appears only after ctrl+x.

**E-016 — An edit's diff is rendered as a diff, never parsed as markdown.**
Behaviour: a result carrying `details.diff` becomes a parsed diff card. Each `@@` header is printed as it
arrived in the accent colour, an added line gets `+` and a removed line `-` before its line number, and a
line the format does not define (`\ No newline at end of file`) is kept and printed rather than dropped.
Below 120 body columns the diff is unified; at or above it the two sides share one row.
Evidence: `crates/epsilon-tui/src/state.rs:847` (`Some(diff) => CardBody::Diff(parse_diff(&diff)),`);
draw `crates/epsilon-tui/src/ui/card.rs:367`.
Test: `an_edits_diff_is_drawn_with_a_coloured_gutter_not_as_prose` — `crates/epsilon-tui/src/app/golden.rs:1846` (frame);
`the_diff_is_not_run_through_the_markdown_parser` — `crates/epsilon-tui/src/app/golden.rs:1980` (frame);
`the_split_diff_draws_both_sides_at_a_wide_frame` — `crates/epsilon-tui/src/app/golden.rs:2125` (frame).
Observe: `+ 5 ` and `- 5 ` precede the two sides of a one-line replacement, in the success and danger colours.

**E-017 — `ui/card.rs`'s `sanitize` normalises tabs and CRs; it is not the security stripper.**
Behaviour: before a body row is measured, every tab becomes three spaces and every carriage return is
deleted, because a tab is one `char` and up to eight columns and a width computed over raw text wraps rows.
This is a width normalisation. The stripper that removes bidi and control characters is `Card::sanitize` in
the reducer (`crates/epsilon-tui/src/state.rs:215`, which calls `text_safety::display_safe`).
Evidence: `crates/epsilon-tui/src/ui/card.rs:782` (`text.replace('\t', "   ").replace('\r', "")`);
draw `crates/epsilon-tui/src/ui/card.rs:210`.
Test: **none found** — `crates/epsilon-tui/src/ui/card.rs`'s `mod tests` holds three tests
(`:822`, `:840`, `:859`), none touching it, and no test string anywhere in the crate contains a literal tab.
Observe: a tool whose output contains a tab draws it as three spaces and the columns stay aligned.
See §7 for a stale comment four lines above this function that credits it with the *other* function's job.

**E-018 — Each tool's headline is produced by a per-tool renderer.**
Behaviour: what a card says depends on which tool made it: `shell`/`bash`/`run` open with `$ <command>`
(a multi-line `bash -c` script collapses to its first line), `grep` prints `/pattern/ in path`, and the
path-taking tools print their path. The headline is computed once at commit and kept as data, because the
draw path is cached and anything derived per frame would freeze.
Evidence: `crates/epsilon-tui/src/renderers/mod.rs:39`; draw `crates/epsilon-tui/src/state.rs:793`.
Test: `a_completed_tool_call_is_drawn_once_on_a_real_frame` — `crates/epsilon-tui/src/app/golden.rs:1707` (frame);
per-tool forms in `crates/epsilon-tui/src/renderers/tests.rs`, e.g. `a_shell_card_reads_like_a_command_line` at `:78`.
Observe: the row reads `bash $ cargo test · pending` rather than a JSON dump of the arguments.

**E-019 — A missing `details` field degrades the headline; it is never invented.**
Behaviour: a headline may know more than the call did, but only from what the tool reported. A `read`
lacking `first_line`/`last_line` degrades to the bare path; a `grep` with no `truncated` flag says nothing
about truncation. A tool with nothing sayable falls back to its name plus the first non-blank output line.
Evidence: `crates/epsilon-tui/src/renderers/mod.rs:79`; draw `crates/epsilon-tui/src/ui/card.rs:114`.
Test: `a_tool_with_nothing_sayable_falls_back_to_the_first_output_line` — `crates/epsilon-tui/src/renderers/tests.rs:139`;
`a_card_with_no_headline_still_renders_its_name` — `crates/epsilon-tui/src/renderers/tests.rs:176`.
Observe: `src/main.rs:10–50 of 300` when the tool reported a range, `src/main.rs` when it did not.

**E-020 — A duration appears only on a finished card.**
Behaviour: a card prints `· 412ms` only once its result has settled, as `ended - started`. Sub-second work
reads as `412ms` rather than `0s`. A running call prints no number on the card at all, because a transcript
row is cached and a live figure would freeze; its live time is in the header instead.
Evidence: `crates/epsilon-tui/src/state.rs:265` (`let end = self.ended_at_ms?;`); draw `crates/epsilon-tui/src/ui/card.rs:120`.
Test: `a_running_call_shows_its_time_in_the_header_not_on_the_card` — `crates/epsilon-tui/src/app/golden.rs:2740` (frame);
`only_a_finished_card_carries_a_duration` — `crates/epsilon-tui/src/state.rs:2835`.
Observe: during a `sleep 30` the header reads `running 12s` while the card row carries no time figure at all.

### 4.5 Streaming — `E-021` … `E-023`

**E-021 — The live tail is painted every frame, below the cached transcript.**
Behaviour: assistant text and reasoning that have not been committed are wrapped and painted as ordinary
transcript rows on every frame, after the cached rows and offset past the cached row count. The tail is not
in the render cache, so it is wrapped and sliced fresh each frame.
Evidence: `crates/epsilon-tui/src/app/panels.rs:401`; draw `crates/epsilon-tui/src/app/panels.rs:412`.
Test: `a_bidi_override_in_streaming_model_output_never_reaches_the_frame` — `crates/epsilon-tui/src/app/tests.rs:5600` (frame).
Observe: after one `MessageUpdate` and no `MessageEnd`, the frame already shows the in-flight words. The
tail's height is measured into the viewport at `crates/epsilon-tui/src/app/view.rs:635` so the position
badge does not jump each time the model wraps a line.

**E-022 — The streaming fields are replaced, not appended, stripped as they enter, and the status reads `streaming…`.**
Behaviour: each `MessageUpdate` replaces the whole in-flight text and thinking fields — the event carries
the message so far, not a delta — and sets the status line. Both fields pass through `display_safe` on the
way in, so the stored tail carries no bidi override or control character. (This is M313; see §8.)
Evidence: `crates/epsilon-tui/src/state.rs:1194-1195` (`self.streaming_text = crate::text_safety::display_safe(&a.text()).0.into_owned();`);
draw `crates/epsilon-tui/src/app/view.rs:151`.
Test: `streaming_updates_replace_then_commit_on_end` — `crates/epsilon-tui/src/state.rs:2202`;
frame assertion `a_bidi_override_in_streaming_model_output_never_reaches_the_frame` — `crates/epsilon-tui/src/app/tests.rs:5600`.
Observe: the header carries `streaming…` for the whole time the answer is arriving.

**E-023 — `MessageEnd` commits the tail into transcript rows and clears it.**
Behaviour: when the message ends, the reasoning taken out of the tail is pushed as a `Thinking` row, the
streaming text is cleared, and the final assistant text is committed as one markdown-carrying row. From
that moment the answer is a cached transcript row rather than a live tail.
Evidence: `crates/epsilon-tui/src/state.rs:1208` (`self.streaming_text.clear();`); draw `crates/epsilon-tui/src/render_cache.rs:159`.
Test: `streaming_updates_replace_then_commit_on_end` — `crates/epsilon-tui/src/state.rs:2202`;
frame companion `an_answer_and_its_reasoning_are_distinguishable_on_a_real_frame_without_colour` — `crates/epsilon-tui/src/app/golden.rs:766`.
Observe: the tail disappears and the same words reappear as a scrollable row that survives later turns,
with the reasoning row above it.

### 4.6 Composer — `E-024` … `E-033`

**E-024 — Left/Right arrows and Ctrl+B/Ctrl+F move the caret one grapheme cluster.**
Behaviour: each moves the caret exactly one extended grapheme cluster, never one scalar, so a ZWJ emoji
family, a flag or an accented letter moves as one step.
Evidence: `crates/epsilon-tui/src/app/keys.rs:1590`; draw `crates/epsilon-tui/src/app.rs:1655` (`caret: (caret_row, caret_col),`).
Test: `ctrl_b_ctrl_f_move_cursor_by_char` — `crates/epsilon-tui/src/app/tests.rs:2739` (state only).
Observe: pressing Right through `a[family]b` moves once per glyph and the caret never parks inside a composed character.

**E-025 — Word motion steps a whole word, with paste markers as one unit.**
Behaviour: Ctrl+Left / Ctrl+Right and Alt+B / Alt+F move to word boundaries; a run of punctuation moves as
one unit, and an atomic `[paste #N …]` marker is one word.
Evidence: `crates/epsilon-tui/src/app/keys.rs:1581`; draw `crates/epsilon-tui/src/app.rs:1655`.
Test: `word_nav_respects_boundaries_and_unicode` — `crates/epsilon-tui/src/editor_ops.rs:306`;
`word_nav_treats_marker_as_one_unit` — `crates/epsilon-tui/src/kill_ring.rs:221`. No test drives Alt+B or Ctrl+Left through `handle_key`.
Observe: hold Ctrl+Left through `alpha beta_gamma  δέλτα` and the caret lands on each word start, Greek included.

**E-026 — Ctrl+A goes to line start, Ctrl+E to line end; Home/End are absolute.**
Behaviour: Ctrl+A and Ctrl+E stop at `\n` in a multi-line draft; Home and End jump to the very start and
very end of the draft.
Evidence: `crates/epsilon-tui/src/app/keys.rs:1523`; draw `crates/epsilon-tui/src/app.rs:1655`.
Test: `ctrl_a_ctrl_e_home_end_line_semantics` — `crates/epsilon-tui/src/app/tests.rs:2672`;
`word_and_line_nav_snaps_off_cluster_interior` — `crates/epsilon-tui/src/editor_ops.rs:372`.
Observe: in a three-line draft, Ctrl+A drops the caret to column zero of the *current* line while Home drops it to the very first.

**E-027 — Backspace, Delete and Ctrl+D each remove a whole grapheme cluster.**
Behaviour: Backspace deletes the cluster before the caret; Delete and Ctrl+D the cluster under it. Both
forward forms are guarded past the end of the draft.
Evidence: `crates/epsilon-tui/src/app/editing.rs:85`; draw `crates/epsilon-tui/src/ui/composer.rs:105`.
Test: `ctrl_d_forward_deletes_and_alt_backspace_kills_word` — `crates/epsilon-tui/src/app/tests.rs:2704`;
`clusters_move_and_delete_whole` — `crates/epsilon-tui/src/editor_ops.rs:335`;
rendered by `a_draft_replaces_the_placeholder` — `crates/epsilon-tui/src/ui/composer.rs:203` (frame).
Observe: one Backspace removes the whole family emoji, and the row is re-wrapped with the caret following.

**E-028 — Ctrl+Z undoes a backspace or a kill; Ctrl+. re-applies it.**
Behaviour: a snapshot of the draft is pushed before each destructive edit; a fresh edit after an undo
discards the redo rail.
Evidence: `crates/epsilon-tui/src/app/editing.rs:189`; draw `crates/epsilon-tui/src/app.rs:1655`.
Test: `ctrl_z_undoes_backspace_and_kills` — `crates/epsilon-tui/src/app/tests.rs:2161`;
`ctrl_dot_redoes_what_ctrl_z_undid` — `crates/epsilon-tui/src/app/tests.rs:2725`;
rail semantics `redo_rail_semantics` — `crates/epsilon-tui/src/undo.rs:118`.
Observe: Ctrl+W then Ctrl+Z brings the word back with the caret where it was. Undo depth is capped at 200 snapshots (`crates/epsilon-tui/src/undo.rs:6`).

**E-029 — Ctrl+U / Ctrl+K / Ctrl+W kill to the ring and Ctrl+Y yanks it back, cycling on repeat.**
Behaviour: the killed text goes on a 16-entry kill ring; consecutive kills accumulate into one entry, and
repeated Ctrl+Y cycles backwards through older entries.
Evidence: `crates/epsilon-tui/src/app/keys.rs:1739`; draw `crates/epsilon-tui/src/app.rs:1655`.
Test: `ctrl_w_then_ctrl_y_round_trips_text_through_the_kill_ring` — `crates/epsilon-tui/src/app/tests.rs:2062`;
`consecutive_ctrl_w_accumulates_into_one_yank_entry` — `crates/epsilon-tui/src/app/tests.rs:2098`;
ring cap and rotation `kill_ring_pushes_accumulates_and_caps_depth` — `crates/epsilon-tui/src/kill_ring.rs:162`,
`kill_ring_rotate_cycles_entries` — `crates/epsilon-tui/src/kill_ring.rs:178`.
Observe: Ctrl+W three times then one Ctrl+Y brings all three words back in order.

**E-030 — Ctrl+J inserts a newline so a draft can span several lines.**
Behaviour: Ctrl+J (`tui.input.newLine`) inserts a literal `\n` instead of sending; each logical line gets
its own screen row and Enter still submits the whole buffer as one message.
Evidence: `crates/epsilon-tui/src/app/keys.rs:1777` (`app.insert_char('\n');`); draw `crates/epsilon-tui/src/app.rs:1838`;
the prompt glyph only on the first row at `crates/epsilon-tui/src/ui/composer.rs:92`.
Test: `ctrl_j_inserts_newline_and_enter_still_submits` — `crates/epsilon-tui/src/app/tests.rs:2076`;
multi-row rendering `editing_start_of_long_draft_keeps_caret_line_visible` — `crates/epsilon-tui/src/app/composer_viewport_tests.rs:166` (frame).
Observe: two rows, only the first marked `❯`, and Enter sends one message containing the line break.

**E-031 — A draft wider than the composer soft-wraps, and the view scrolls to keep the caret's row on screen.**
Behaviour: the draft is wrapped to the composer's own text width before drawing, one grapheme per
decision, preserving typed spaces; when the caret is below the composer the earlier rows scroll off.
Evidence: `crates/epsilon-tui/src/app.rs:1830` (`if !newline && col + cells > width {`); draw `crates/epsilon-tui/src/ui/composer.rs:75`.
Test: `wrapped_draft_grows_composer_at_supported_widths` — `crates/epsilon-tui/src/app/composer_viewport_tests.rs:85` (frame);
`a_long_draft_scrolls_to_keep_the_caret_visible` — `crates/epsilon-tui/src/ui/composer.rs:210` (frame).
Observe: type a paragraph wider than the window; it wraps, nothing is dropped, and the caret never disappears.
Composer wrapping does **not** use `crates/epsilon-tui/src/wrap.rs` — that module is transcript-only.

**E-032 — The caret is the terminal's own cursor, positioned in display columns.**
Behaviour: when the composer is focused the terminal cursor is placed on the caret's row and column, offset
by the prompt gutter; when an overlay owns the keyboard no cursor is placed at all. No cursor shape or
blink rate is set by the app, so the caret blinks as the terminal blinks — `crates/epsilon-tui/src/ui/composer.rs:112-114`
gives the reason ("a drawn block cannot blink").
Evidence: `crates/epsilon-tui/src/ui/composer.rs:118` (`f.set_cursor_position((x, y));`); gutter from `:115` and `:128`.
Test: `long_unicode_draft_wraps_and_places_caret_in_display_columns` — `crates/epsilon-tui/src/app/composer_viewport_tests.rs:123` (frame).
Observe: in a draft of CJK ideographs the cursor sits immediately after the character it will extend.

**E-033 — The composer grows with a long draft but stops at 8 rows or a third of the frame.**
Behaviour: the composer's height is the draft's wrapped row count clamped to `min(8, height/3)`. The draft
text itself is never truncated at any length — there is no length cap anywhere in the crate.
Evidence: `crates/epsilon-tui/src/ui/layout.rs:81`; draw `crates/epsilon-tui/src/ui/layout.rs:45` (`pub const COMPOSER_MAX_ROWS: u16 = 8;`).
Test: `the_composer_never_takes_the_screen` — `crates/epsilon-tui/src/ui/layout.rs:186`;
`a_long_transcript_never_pushes_the_composer_off_the_screen` — `crates/epsilon-tui/src/app/golden.rs:449` (frame).
Observe: paste a document; the composer grows for a few rows then stops and the transcript keeps its rows.

### 4.7 Submit — `E-034` … `E-037`

**E-034 — Submitting empties the draft, resets the caret, and expands paste markers.**
Behaviour: whatever key the `tui.input.submit` binding names takes the draft out, leaves it empty with the
caret at 0, ends any in-progress yank, and expands every `[paste #N …]` marker back to its stored text.
Evidence: `crates/epsilon-tui/src/app.rs:775` (`let raw = std::mem::take(&mut self.input);`); draw `crates/epsilon-tui/src/app.rs:1654`.
Test: `a_rebound_submit_key_is_the_key_that_submits` — `crates/epsilon-tui/src/app/tests.rs:1875`;
`paste_marker_expands_to_payload_on_submit` — `crates/epsilon-tui/src/app/tests.rs:2118`.
Observe: press the submit key and the prompt is empty with the caret at column zero.

**E-035 — Submit on an empty or whitespace-only draft does nothing.**
Behaviour: after the draft is taken and its paste markers expanded, text that is empty or only whitespace
returns immediately: no command runs, no shell command runs, no user message is sent, nothing is written
to the transcript or to history.
Evidence: `crates/epsilon-tui/src/app.rs:780` (`if text.trim().is_empty() {`); draw — none; the function
returns before any transcript line is pushed, so the frame is byte-identical to the one before the keypress.
Test: **none found** — `submit_editor` was read in full (`crates/epsilon-tui/src/app.rs:774-887`) and
every Enter press in the suite is made with non-empty draft text or inside an overlay arm.
Observe: pressing submit on a fresh screen produces no echo, no spinner and no notice.

**E-036 — A `/command` draft runs a command and never reaches the model; `!cmd` runs a shell command.**
Behaviour: a draft that parses as a slash command is handed to the command runner and returns there. A
draft starting with `!` runs a shell command through the real `BashTool` (`!!` keeps its output out of
context). Anything else becomes a user message.
Evidence: `crates/epsilon-tui/src/app.rs:856` (`if let Some(command) = crate::picker::parse_command(&text) {`);
draw `crates/epsilon-tui/src/app/editing.rs:349` (`self.ui.push_local_input(&text);`).
Test: `unknown_slash_word_errors_instead_of_prompting_the_model` — `crates/epsilon-tui/src/app/tests.rs:1422`;
`x_toggles_expand_and_quit_requests_exit` — `crates/epsilon-tui/src/app/tests.rs:2042`;
`enter_with_popup_on_exact_text_still_edits_before_running` — `crates/epsilon-tui/src/app/tests.rs:2608`.
Observe: `/models` opens the picker and starts no run; `!ls -la` lists without consulting the model.

**E-037 — Text typed while a run is active is queued as steering, with a notice saying so.**
Behaviour: after the message is built, whether a run is active decides the destination — during a run the
message is queued onto the agent as steering and the transcript says so; when idle it starts a new run.
Evidence: `crates/epsilon-tui/src/app.rs:881` (`agent.steer(msg);`); draw — none; the queued state is
announced only as a transcript notice at `crates/epsilon-tui/src/app.rs:882`.
Test: **none found** — a grep for `steering queued`, `agent.steer(` and `fn steer` across
`crates/epsilon-tui/src` returns only the two production lines at `crates/epsilon-tui/src/app.rs:881-882`.
Observe: with an answer streaming, type a correction and press submit; your words appear at once and are
delivered at the end of the current turn rather than starting a second one.

### 4.8 Completion — `E-038` … `E-039`

**E-038 — `/` opens a fuzzy command popup above the composer; arrows move it, Tab/Enter apply, Esc dismisses.**
Behaviour: typing `/` opens an eight-row popup titled `commands` listing slash candidates ranked by fuzzy
score with descriptions right-aligned. Tab applies the highlighted candidate and closes the popup without
submitting; Enter applies it and then falls through to submit, so `/hotk` + Enter runs `/hotkeys`; Esc
dismisses and keeps the text.
Evidence: `crates/epsilon-tui/src/app/completion.rs:23` (`let candidates = crate::picker::slash_items(&self.input);`),
`:161` (`self.input = item.value.to_string();`); draw `crates/epsilon-tui/src/app/panels.rs:771` (`title: "commands",`), `:796`.
Test: `slash_popup_opens_and_arrows_move_selection` — `crates/epsilon-tui/src/app/tests.rs:2568` (state only).
No frame test asserts the popup's text: `an_ascii_frame_draws_no_character_a_legacy_console_cannot` —
`crates/epsilon-tui/src/app/golden.rs:270` (frame) paints it but asserts only the CP437 repertoire.
Observe: a box listing `/models`, `/model `, `/think ` with dim descriptions; the hint row reads `↑↓ pick · tab apply · esc dismiss`.

**E-039 — `@` opens a workspace-file popup that fuzzy-matches the `@`-word and inserts the path.**
Behaviour: `@` plus word characters opens a popup titled `files` listing workspace-relative paths that
agree with the tools' ignore rules, fuzzy-filtered, capped at 200 rows, with the walk cached for 5 s so
typing does not re-walk per keystroke. Tab inserts the path with no trailing space, Enter the path plus a space.
Evidence: `crates/epsilon-tui/src/app/completion.rs:42`, `:129`; draw `crates/epsilon-tui/src/app/panels.rs:521` (`title: "files",`), `:555`.
Test: `at_file_popup_opens_and_tab_inserts_path` — `crates/epsilon-tui/src/app/tests.rs:3076` (state only);
`the_list_agrees_with_the_ignore_matcher_the_tools_use` — `crates/epsilon-tui/src/at_files.rs:80`.
No frame test: `crates/epsilon-tui/src/app/golden.rs:287-303` paints only panels 0-3, so `draw_at_file` is never rendered in an assertion.
Observe: `fix @src/a` opens a narrow `files` box containing `src/app.rs`; hint row `tab insert · enter insert + space · esc close`.

### 4.9 Model picker — `E-040` … `E-045`

**E-040 — Ctrl+M (`app.model.select`) and `/models` open the picker and stash the draft.**
Behaviour: opening clears the composer and repurposes the editor line as the picker's filter; a draft typed
before opening is stashed and restored on close. Opening also starts a silent live listing fetch.
Evidence: `crates/epsilon-tui/src/app/keys.rs:1429` (`app.stash_draft();`), `:1431` (`app.request_live(false);`);
`crates/epsilon-tui/src/app/commands.rs:529`; draw `crates/epsilon-tui/src/app/panels.rs:80`.
Test: `ctrl_m_opens_picker_without_typing_slash` — `crates/epsilon-tui/src/app/tests.rs:1726`;
`slash_models_opens_picker_via_real_key_path` — `crates/epsilon-tui/src/app/tests.rs:727`.
Observe: a half-typed prompt plus Ctrl+M opens `models — filter: ''` and empties the composer; Esc restores the prompt (`crates/epsilon-tui/src/app/editing.rs:168`).

**E-041 — The picker is two stages: providers first, then that provider's models.**
Behaviour: stage 1 is one row per distinct provider (Popular first, then auth-ready, then alphabetical),
each with a model count and an auth marker. Enter descends to stage 2 — that provider's models, bundled
plus live-listed, deduped — resetting the highlight and clearing the filter.
Evidence: `crates/epsilon-tui/src/app/keys.rs:1176`; `crates/epsilon-tui/src/picker.rs:379`; draw `crates/epsilon-tui/src/app/panels.rs:58`.
Test: `two_stage_picker_descends_and_applies_model` — `crates/epsilon-tui/src/app/tests.rs:2181`;
`stage2_merges_live_ids_for_its_provider_only` — `crates/epsilon-tui/src/picker.rs:966`.
Observe: stage 1 rows read `●✓ opencode` with `42 models` right-aligned; after Enter the title becomes `models — deepseek: filter ''`.

**E-042 — Arrows step one row; PageUp/PageDown step exactly the window the picker last drew.**
Behaviour: arrows move by one and clamp at both ends. PageUp/PageDown move by `picker_capacity`, the row
count the paint measured on the last frame, so a page grows on a tall terminal and shrinks on a short one.
Evidence: `crates/epsilon-tui/src/app/keys.rs:1228` (`let page = app.picker_capacity.max(1);`), `:1238`;
draw `crates/epsilon-tui/src/app/panels.rs:79` (`app.picker_capacity = height.saturating_sub(3).max(1) as usize;`).
Test: `a_page_key_moves_by_the_window_the_list_actually_drew` — `crates/epsilon-tui/src/app/tests.rs:5488` (frame — renders 100x44 and 100x14 and reads the capacity off the painted picker).
Observe: on 44 rows PageDown jumps by the whole visible list; at 14 rows about a third as far.

**E-043 — Typing filters stage 2; Esc steps back a stage, then closes and unstashes.**
Behaviour: a printable character typed while the picker is open is appended to the editor buffer, which the
picker reads as its filter, and the highlight resets. Esc in stage 2 returns to the provider list with the
filter cleared; Esc again closes, clears the filter and restores the stashed draft.
Evidence: `crates/epsilon-tui/src/app/keys.rs:1325`, `:1151`; draw `crates/epsilon-tui/src/app/panels.rs:80`.
Test: `two_stage_picker_descends_and_applies_model` — `crates/epsilon-tui/src/app/tests.rs:2181` (covers both Esc steps);
`filtered_results_are_ranked_by_fuzzy_score_best_first` — `crates/epsilon-tui/src/picker.rs:1083`.
Observe: the title tracks the filter verbatim; first Esc returns to providers, second closes and the pre-open draft reappears.

**E-044 — Enter applies the highlighted model; Ctrl+F stars it; the old `unreachable!()` interception is gone.**
Behaviour: Enter in stage 2 closes the picker and calls `apply_model`, which switches the session model,
hot-swaps off faux when the new model has credentials, persists the id and pushes a notice. Ctrl+F toggles
the favourite star on the highlighted row. The `Ctrl+F`/`Ctrl+Up`/`Ctrl+Down` `unreachable!()` arm that used
to intercept these chords **no longer exists**; the rationale is at `crates/epsilon-tui/src/app/keys.rs:1186-1199` — "A guard in front of
the code it guards is not a guard" — and the first match now ends at `crates/epsilon-tui/src/app/keys.rs:1200` with `_ => {}`.
Evidence: `crates/epsilon-tui/src/app/keys.rs:1200` (`_ => {}`), `:1170` (`app.apply_model(agent, id);`), `:1301` (`Some(id) => app.toggle_favorite(agent, &id),`);
draw `crates/epsilon-tui/src/app/panels.rs:294` (`let star = if row.is_favorite {`).
Test: `ctrl_f_in_the_model_picker_toggles_a_favorite_instead_of_panicking` — `crates/epsilon-tui/src/app/tests.rs:1778`;
`slash_model_direct_applies_instantly` — `crates/epsilon-tui/src/app/tests.rs:1402`.
Observe: the highlighted row gains a `★` with a `★ <id> favorited at think:…` notice; before the fix all three chords panicked.

**E-045 — The picker draws a titled, auth-marked list over a hint row; Ctrl+G forces a live refresh.**
Behaviour: the panel's title names the filter, one row per item with the active item marked `●`, an
auth/key badge column, the star column in stage 2, the model count or id right-aligned, and a hint row
naming the keys that work for the current stage. Ctrl+G forces a re-fetch, retrying a provider that
already answered; the automatic fetch on open stays silent.
Evidence: `crates/epsilon-tui/src/app/keys.rs:1320` (`app.request_live(true);`), `crates/epsilon-tui/src/app.rs:1092`;
draw `crates/epsilon-tui/src/app/panels.rs:244`, `:315`.
Test: `a_panel_row_is_stripped_before_it_is_drawn` — `crates/epsilon-tui/src/app/golden.rs:2660` (frame);
`an_ascii_frame_draws_no_character_a_legacy_console_cannot` — `crates/epsilon-tui/src/app/golden.rs:270` (frame).
Gap: Ctrl+G is never pressed by any test — `app.live.refresh` appears only at `crates/epsilon-tui/src/keybindings.rs:141` and `crates/epsilon-tui/src/app/keys.rs:1317`.
Observe: stage 2 reads `models — deepseek: filter 'pro'` over `type filters (fuzzy) · enter apply · backspace/esc back`.

### 4.10 Slash — `E-046` … `E-050`

**E-046 — A submitted `/word` is parsed and routed to a themed executor, and its answer lands in the transcript.**
Behaviour: a submitted line starting with `/` is parsed by `parse_command`, which returns `None` only for
text not starting with `/` — every slash line is a command attempt. The resulting `Command` routes by name
to provider / session / view / transfer executors, everything else to the workspace executor.
Evidence: `crates/epsilon-tui/src/app.rs:856`; `crates/epsilon-tui/src/app/commands.rs:471`, `:517`.
Draw: none separate — a command's reply is a transcript notice pushed with `ui.push_notice`.
Test: `no_slash_command_panics_when_typed` — `crates/epsilon-tui/src/app/tests.rs:3671` (drives every `COMMAND_HEADS` entry through the real key path);
`commands_parse_with_optional_args_and_case` — `crates/epsilon-contracts/src/command.rs:693`.
Observe: `/help` fills the transcript immediately; `/quit` ends the session.

**E-047 — An unknown `/word` is corrected in the transcript and never sent to the model.**
Behaviour: `parse_command` has no catch-all `None` for slash input; every unrecognised head returns
`Command::Unknown(head)`. The executor pushes one notice naming the word, appending `— did you mean '/x'?`
when a fuzzy guess exists. No message is constructed, so no run starts.
Evidence: `crates/epsilon-contracts/src/command.rs:577` (`other => Some(Command::Unknown(other.to_string())),`);
`crates/epsilon-tui/src/app/commands.rs:1598` (`let hint = crate::picker::suggest_slash(&word)`).
Test: `unknown_slash_word_errors_instead_of_prompting_the_model` — `crates/epsilon-tui/src/app/tests.rs:1422`;
`suggest_slash_finds_best_fuzzy_hint_for_unknown` — `crates/epsilon-tui/src/picker.rs:1140`.
Observe: `/modl` + Enter gives one line, `unknown command '/modl' — did you mean '/model'? — /help lists commands`; the status bar stays idle.

**E-048 — `/help` prints the whole command reference from `SLASH_ITEMS` plus a usage guide.**
Behaviour: `/help` (and `/?`) walks `SLASH_ITEMS` pushing one notice per entry, then a guide: auth lines,
the `/connect` and `/auth` aliases, thinking keys, the metrics legend, goal verbs, and a key line.
Evidence: `crates/epsilon-tui/src/app/commands.rs:1829` (`for item in crate::picker::SLASH_ITEMS {`);
`crates/epsilon-contracts/src/command.rs:47`.
Test: `no_slash_command_panics_when_typed` — `crates/epsilon-tui/src/app/tests.rs:3671` (requires at least one new notice);
`every_offered_command_is_reachable_from_slash_items` — `crates/epsilon-contracts/src/command.rs:606`.
Observe: `/help` lists `/models — switch model (picker)` and one line per command. No test asserts its rendered frame text.

**E-049 — `/hotkeys` prints the keyboard summary and the bindings actually in effect.**
Behaviour: four summary lines, then — when the manager reports any effective bindings — a sorted
`bindings (effective — ~/.config/epsilon/keybindings.json):` header and one line per action id with chords
joined by `|`. A user who rebinds a key sees their chord here, not the default.
Evidence: `crates/epsilon-tui/src/app/commands.rs:1978`, `:1990`.
Test: `no_slash_command_panics_when_typed` — `crates/epsilon-tui/src/app/tests.rs:3671`;
`an_unbound_key_produces_a_visible_response` — `crates/epsilon-tui/src/app/tests.rs:1517`.
Observe: reads `hotkeys: Enter submit · Esc aborts run (quits when idle) · Ctrl+X expands tool output`, then `  app.model.select: ctrl+m` lines.

**E-050 — `/settings` reports where configuration lives and what is currently set.**
Behaviour: `/settings` opens no overlay; it pushes the resolved global and project config paths, the current
model id and thinking level, the first five recents and favourites (`(none — Ctrl+F to star)` when empty),
and a closing line telling the reader to edit the TOML and run `/reload`.
Evidence: `crates/epsilon-tui/src/app/commands.rs:2004`, `:2018`, `:2025`.
Draw: none — deliberately guidance text, not a form (the `ponytail:` note is at `crates/epsilon-tui/src/app/commands.rs:1997`).
Test: `no_slash_command_panics_when_typed` — `crates/epsilon-tui/src/app/tests.rs:3671`.
Observe: `settings: global … · project …`, then `  current model: … · think: …`, then the `/reload` instruction.

### 4.11 Permissions — `E-051` … `E-057`

**E-051 — An approval ask reaches the screen as a modal painted over everything.**
Behaviour: the run loop polls the approval queue as one arm of a biased `select!`; an ask sets
`app.pending_ask` and forces a redraw, and the very next frame paints a centred `permission required`
dialog over the transcript and every other overlay.
Evidence: `crates/epsilon-tui/src/app/event.rs:281` (`app.pending_ask = Some(pending);`);
draw `crates/epsilon-tui/src/app.rs:1719` (`draw_dialog(f, app, f.area());`).
Test: `a_pending_approval_is_drawn_over_the_timeline` — `crates/epsilon-tui/src/app/tests.rs:520` (frame).
Observe: a tool the agent wants to run makes the dialog appear over whatever was on screen, framed in the
danger colour (`crates/epsilon-tui/src/app/panels.rs:838`). Captured:
`audit/evidence/tui-frames-2026-10-04-m312/permission.80x24.txt:13-18`.

**E-052 — The dialog names the capability and one summarised "what" line, not the raw JSON.**
Behaviour: the body is exactly two labelled rows — `cap` and `what`, the latter taken from the first of
`command`/`path`/`file`/`query`/`url` in the request's detail, truncated to 96 columns. The `pattern` field
is deliberately dropped: it is the first whitespace token and restated what the detail line shows.
Evidence: `crates/epsilon-tui/src/app.rs:153` (`crate::wrap::fit_width(&epsilon_runtime::summarize_request(detail), 96)`);
draw `crates/epsilon-tui/src/app/panels.rs:870` (`field("what", what.into_owned()),`); the preference order at `crates/epsilon-runtime/src/lib.rs:4148`.
Test: `an_ordinary_ask_still_offers_the_usual_keys` — `crates/epsilon-tui/src/app/tests.rs:5962` (frame).
Observe: the frame reads `cap  bash` / `what rm -rf target`.

**E-053 — `y` allows once, `a` always (session), `n` or Esc denies, `e` denies with guidance.**
Behaviour: four decision keys, all handled in one place before any other overlay's keys. `e` stashes the
composer draft and switches the dialog into guidance mode, where Enter sends the typed guidance as
`DenyWithGuidance`.
Evidence: `crates/epsilon-tui/src/app/keys.rs:486` (`ask.reply(PermissionDecision::Deny);`);
draw `crates/epsilon-tui/src/app/panels.rs:923`.
Test: `an_ordinary_ask_still_offers_the_usual_keys` — `crates/epsilon-tui/src/app/tests.rs:5962` (frame);
`guidance_mode_is_drawn_where_the_keys_are_and_returns_the_draft` — `crates/epsilon-tui/src/app/tests.rs:320` (frame).
Observe: the footer and the handler cannot disagree — `e` replaces the answer-key lines with a
`guidance — the keyboard is here now` line rather than appending to them (`crates/epsilon-tui/src/app/panels.rs:888`).

**E-054 — A denial is monotonic: an "always allow" memory can never resurrect a denied call.**
Behaviour: the gate evaluates a deny rule *before* consulting the session "always allow" memory, so a
policy that tightens mid-session takes effect immediately. Separately, an ask whose responder is dropped
without an answer resolves to `Deny`, so a UI that disappears cannot leave work hanging.
Evidence: `crates/epsilon-safety/src/gate.rs:85` (`if self.engine.evaluate(&req.capability, &req.pattern) == Action::Deny {`);
the fail-closed drop at `crates/epsilon-safety/src/channel_ui.rs:128` (`rx.await.unwrap_or(PermissionDecision::Deny)`).
Draw: none — decided in the gate before any decision reaches the dialog.
Test: `memory_promotion_cannot_override_a_fresh_deny` — `crates/epsilon-safety/src/gate.rs:261`;
`dropped_ask_denies_fail_closed` — `crates/epsilon-safety/src/channel_ui.rs:186`;
`bash_defaults_to_ask_and_deny_carries_no_access` — `crates/epsilon-safety/src/gate.rs:229`.
Observe: nothing appears for this; the reader notices it as the *absence* of a dialog.

**E-055 — An unbound key on a permission ask produces a notice, not silence.**
Behaviour: any key the dialog does not act on is refused with a notice naming both the fact and the
alternatives, and the ask stays parked.
Evidence: `crates/epsilon-tui/src/app/keys.rs:512` (`"{other} is not bound on a permission ask — \`);
draw `crates/epsilon-tui/src/app/panels.rs:928`.
Test: `an_unbound_key_on_a_permission_ask_says_so_and_names_the_ones_that_work` — `crates/epsilon-tui/src/app/tests.rs:280`.
Observe: press Tab and the notice reads `Tab is not bound on a permission ask — y allows once, a always, n denies, e denies with guidance`; the tool stays parked.

**E-056 — The permission dialog owns the keyboard ahead of the model picker, the palette and search.**
Behaviour: with an ask pending, `handle_key` routes every key to the approval handler before the model
picker (`crates/epsilon-tui/src/app/keys.rs:1136`), the palette, the search (`crates/epsilon-tui/src/app/keys.rs:553`) or the masked `/login` prompt
(`crates/epsilon-tui/src/app/keys.rs:557`), and the only precedence is the `app.exit` binding. Nothing in that branch reads run state,
so the dialog stays answerable mid-turn.
Evidence: `crates/epsilon-tui/src/app/keys.rs:550` (`if app.pending_ask.is_some() && !app.keybindings.matches("app.exit", &key) {`);
draw `crates/epsilon-tui/src/app/view.rs:445` (`|| self.pending_ask.is_some()`).
Test: `approval_preempts_palette_and_picker_without_changing_filter` — `crates/epsilon-tui/src/app/tests.rs:159`;
`a_pending_approval_is_drawn_over_the_timeline` — `crates/epsilon-tui/src/app/tests.rs:520` (frame);
`a_pending_ask_outranks_an_open_search` — `crates/epsilon-tui/src/app/tests.rs:4935`.
Observe: open the picker or palette, let an ask arrive, press `n`: the ask is denied and the picker stays
open with its filter untouched. **The comment at `crates/epsilon-tui/src/app/keys.rs:1134-1135` says the opposite** — see §7.

**E-057 — After the dialog resolves, the next queued ask is seated but unanswerable until a frame paints it.**
Behaviour: answering takes the ask out and sends the decision down its private reply channel; the run loop's
`next_ask()` arm then pops the next parked ask in FIFO order. That fresh ask is seated immediately but its
keys are refused until a frame has actually drawn it — and the very same key answers it the moment one has.
Evidence: `crates/epsilon-safety/src/channel_ui.rs:74` (`if let Some(id) = st.order.pop_front()`);
draw `crates/epsilon-tui/src/app.rs:1714`.
Test: `a_second_parked_ask_is_not_answerable_before_a_frame_paints_it` — `crates/epsilon-tui/src/app/tests.rs:565`;
`ask_parks_and_next_ask_replies_in_fifo_order` — `crates/epsilon-safety/src/channel_ui.rs:146`.
Observe: with two calls parked, pressing `y` in the gap prints `approval is not visible yet — it appears on the next frame…` and nothing runs.

### 4.12 Auth — `E-058` … `E-061`

**E-058 — `/login <provider> <key>` stores a key without ever opening a prompt.**
Behaviour: the tail after `login` is split on the first space into provider and key; a non-empty key is
written straight to the store and a notice names the provider and the file written. The line is routed as a
command, so it is never echoed into the transcript.
Evidence: `crates/epsilon-contracts/src/command.rs:563` (`} else if let Some((prov, key)) = tail.split_once(' ') {`);
notice `crates/epsilon-tui/src/app/commands.rs:615`.
Test: `login_oauth_never_falls_through_to_the_key_store` — `crates/epsilon-tui/src/app/tests.rs:1376`.
Gap: **no test types `/login <provider> <key>` with a real key** — it would write the developer's real
store, which is the decision recorded at `docs/milestones/M162-slash-commands-by-subject.md:104`.
Observe: `saved key for 'anthropic' → <path>  (next request uses it)`.

**E-059 — `/login <provider>` with no key opens a masked prompt instead of storing anything.**
Behaviour: an empty key segment opens a centred modal titled `API key · <provider>` that renders one bullet
glyph per character and never the key itself; typing, bracketed paste, Ctrl+V and Shift+Insert all go into
the masked buffer. Enter saves, Esc cancels.
Evidence: `crates/epsilon-tui/src/app/commands.rs:598` (`self.auth_prompt_open = true;`);
draw `crates/epsilon-tui/src/app/panels.rs:1028` (`let masked: String = theme.glyphs.bullet.repeat(masked_len);`), `:1041`.
Test: `oauth_need_code_opens_the_overlay_and_done_closes_it` — `crates/epsilon-tui/src/app/tests.rs:795`;
`ctrl_o_in_the_key_prompt_switches_to_the_oauth_ceremony` — `crates/epsilon-tui/src/app/tests.rs:898`.
Gap: **no test renders this panel** — nothing calls `render_rows` with `auth_prompt_open` set.
Observe: the panel reads `the key is never shown, here or in the transcript` above a row of `•`.

**E-060 — `/logout <provider>` removes the stored key and says which of three things happened.**
Behaviour: a bare `/logout` prints a usage line naming an example. With a provider, the store is asked to
remove the entry and the terminal reports removed, nothing-to-remove, or an error — so "nothing happened"
is never reported as success.
Evidence: `crates/epsilon-contracts/src/command.rs:575`; report `crates/epsilon-tui/src/app/commands.rs:635`;
usage `crates/epsilon-tui/src/app/commands.rs:629`.
Test: `no_slash_command_panics_when_typed` — `crates/epsilon-tui/src/app/tests.rs:3671` (covers only the bare usage line);
removal semantics one layer down at `save_load_remove_round_trip_preserving_other_providers` — `crates/epsilon-llm/src/auth.rs:973`.
Gap: **no test types `/logout <provider>` in the TUI**, because it would delete the developer's real stored key.
Observe: `removed stored key for 'openai'`, or `no stored key for 'openai' (nothing removed)`.

**E-061 — The key is written to a JSON file in the config dir, not the OS credential store.**
Behaviour: a saved key lands in `<global config dir>/auth.json` as `{"<provider>": {"type": "api", "key": "…"}}`,
owner-read/write on unix. The screen shows bullets and the path, never the value.
Evidence: `crates/epsilon-llm/src/auth.rs:6` (the module doc naming the path); draw `crates/epsilon-tui/src/app/panels.rs:1041`.
Test: `auth_file_is_owner_only_on_unix` — `crates/epsilon-llm/src/auth.rs:1135`;
`cancelling_a_key_entry_says_what_was_kept` — `crates/epsilon-tui/src/app/tests.rs:822`.
Observe: `saved key for 'openai' → <path>` and never the key. **This contradicts `AGENTS.md` §6**
("Secrets belong in the OS credential store"); see §7 — reported, not reconciled.

### 4.13 Sessions — `E-062` … `E-070`

**E-062 — `/sessions` lists this workspace's sessions, and Enter resumes the picked one.**
Behaviour: `/sessions` (and bare `/resume`) opens an overlay listing session files newest-first; each row
carries a title, a right-hand meta string of `age · size · [task child] · [in use] · <file name>`, and a
"you are here" mark on the attached file. Enter closes the overlay and swaps the app onto that file.
Evidence: `crates/epsilon-tui/src/picker.rs:751` (`meta: format!("{meta_head} · {file}"),`);
draw `crates/epsilon-tui/src/app/panels.rs:204` (`let marker = if row.is_current { active_marker } else { "  " };`).
Test: `session_rows_list_newest_first_and_mark_the_current_file` — `crates/epsilon-tui/src/picker.rs:776`;
`sessions_command_browses_and_enter_switches_the_session` — `crates/epsilon-tui/src/app/tests.rs:1039`.
Observe: the attached session's title leads with the dot glyph; after Enter the panel is gone and the
transcript shows the other conversation plus `resumed N messages from … (history restored, store swapped)`.

**E-063 — `/new` starts a new session *file*, and keeps the old one.**
Behaviour: `/new` refuses while a run is in flight; otherwise it creates a brand-new `.jsonl` beside the
existing sessions, attaches it, and says so. Nothing is carried over because the new file has no history,
rather than because rows were cleared.
Evidence: `crates/epsilon-tui/src/app/commands.rs:1341`; notice `:1343`.
Test: `new_command_starts_a_new_session_file_and_keeps_the_old_one` — `crates/epsilon-tui/src/app/tests.rs:1105`;
`new_command_refuses_while_a_run_is_in_flight` — `crates/epsilon-tui/src/app/tests.rs:1171`.
Observe: the transcript is empty rather than scrolled, the notice names the new file, and `/sessions` now lists two files.

**E-064 — `/fork` and `/clone` each write a distinct session, never a file copy.**
Behaviour: both refuse on an empty session or a missing store, then call `fork_branch_at` on the current
leaf, writing a new session file with a **fresh header id** and only the active branch's ancestry.
`/clone` differs only in the word it uses.
Evidence: `crates/epsilon-tui/src/app/commands.rs:1161`; notice `:1169`.
Test: `fork_creates_a_distinct_session_and_not_a_file_copy` — `crates/epsilon-tui/src/app/tests.rs:1472`.
Gap: no test drives `/clone`; `a_command_needing_the_session_store_says_so_instead_of_crashing` — `crates/epsilon-tui/src/app/tests.rs:3602` — pins only the missing-store refusal.
Observe: a notice naming the new path and the `--session <path>` that opens it; `.epsilon/sessions` gains exactly one `.jsonl` whose header id differs.

**E-065 — `/export` writes HTML by default; `json`/`jsonl` picks the JSON writer.**
Behaviour: bare `/export` writes HTML to `<workspace>/.epsilon/exports/session-<id>.html`. An explicit
leading `html`/`json`/`jsonl` selects the format, a bare path infers it from a `.json`/`.jsonl` suffix, and
a relative path resolves against the workspace root.
Evidence: `crates/epsilon-tui/src/app/commands.rs:2186`; notice `:2215`.
Test: `export_writes_html_and_json` — `crates/epsilon-tui/src/app/tests.rs:3114`, which asserts the two
writers directly and **not** the command's format inference or default destination.
Observe: `exported html → .epsilon/exports/session-….html (N entries)`.

**E-066 — `/resume` bare opens the browser, `/resume <path>` swaps live; `/import` is the same swap worded as an import.**
Behaviour: with a path it refuses while a run is in flight, checks the file exists (relative paths resolve
against the workspace root), then swaps the store, re-hydrates the transcript and pushes the typed path into
shell history. `/import <path>` runs the identical switch.
Evidence: `crates/epsilon-tui/src/app/commands.rs:1357`; notice `:1377`; the browser panel at `crates/epsilon-tui/src/app.rs:1676`.
Test: **none found** — no test in the tree types `/resume` or `/import` at all. The shared switch is pinned
only through the browser's Enter by `sessions_command_browses_and_enter_switches_the_session` — `crates/epsilon-tui/src/app/tests.rs:1039`.
Observe: bare `/resume` produces the same panel as `/sessions`; the path form prints `resumed 41 messages from …` or `resume: no file at '…' — check the path`.

**E-067 — `/session` prints what session this is, plus where the tokens and the money went.**
Behaviour: it prints the file, the leaf id, the branch entry count, live token totals and the model —
prefixed with the `/name` display name when one is set — then cache-attribution lines, a cost breakdown and
any repairs the last resume had to make. With no durable store it says so instead of printing a session.
Evidence: `crates/epsilon-tui/src/app/commands.rs:1075`; draw — notices at `:1062`, `:1092`, `:1099`.
Test: `session_command_reports_cache_attribution_beside_the_totals` — `crates/epsilon-tui/src/app/tests.rs:1211`;
`session_command_shows_where_the_money_went_not_just_how_much` — `crates/epsilon-tui/src/app/tests.rs:1231`.
Observe: a block of `session: …` / `cache: …` / `cost: …` lines; with no store it reads `session: no durable session file attached · tokens …/… · model …`.

**E-068 — `/timeline` and `/tree` open one picker; Enter jumps to that branch and re-reads the transcript.**
Behaviour: both build the same picker from `SessionStore::tree()` (the whole forest, not just the active
lane), land the highlight on the leaf, and mark off-lane rows with a `·`. Enter navigates the store to **the
selected row's own id**, clears the transcript and re-projects it from that branch.
Evidence: `crates/epsilon-tui/src/app/keys.rs:797`; draw `crates/epsilon-tui/src/app/panels.rs:642`.
Test: `timeline_picker_opens_and_enter_jumps` — `crates/epsilon-tui/src/app/tests.rs:2408`;
`tree_and_timeline_open_one_picker_and_say_which_one_ran` — `crates/epsilon-tui/src/app/tests.rs:3446`.
Observe: a `timeline · <filter>` panel of `#[seq] [kind] snippet` rows; the current lane has a blank gutter, abandoned branches a `·`.

**E-069 — `/name <display name>` names the session and persists it into the file.**
Behaviour: it sets the display name in memory and appends a non-LLM `session_name` entry to the session
file; the notice distinguishes a persisted write from one refused (e.g. the file is leased). Bare `/name`
prints the current name — read back from the file when the in-memory value is unset.
Evidence: `crates/epsilon-tui/src/app/commands.rs:1281`; draw `crates/epsilon-tui/src/ui/transcript.rs:238`.
Test: **none found** — no test types `/name`. `a_command_needing_the_session_store_says_so_instead_of_crashing`
— `crates/epsilon-tui/src/app/tests.rs:3602` — pins only the missing-store refusal; the name is re-read on
resume at `crates/epsilon-tui/src/app/session.rs:275`.
Observe: `session name set to 'api work'`, and on an empty transcript the welcome panel gains a `session  api work` row.

**E-070 — `/restore` previews first; only a trailing `yes` overwrites.**
Behaviour: `/restore <path>` describes the checkpoint and changes nothing; `/restore <path> yes` — the
confirmation must be the **last** whitespace-separated word, so a file literally named `yes` is still a file —
reverts. A bare `/restore` prints the grammar, a busy run is refused, and a file that changed since the
capture is refused with the file untouched.
Evidence: `crates/epsilon-tui/src/app/commands.rs:1719` (`let answer = if confirmed {`); notices at `:1725`.
Test: `a_confirmed_restore_reverts_the_file_and_the_frame_says_so` — `crates/epsilon-tui/src/app/commands.rs:2529` (frame, at 80x24 and 100x30).
Observe: the first press prints `nothing has changed yet … DELETE … /restore src/new.rs yes` and the file is still there.

### 4.14 Files — `E-071` … `E-073`

**E-071 — `@` opens a file popup over the composer; Tab inserts the path, Enter inserts it plus a space.**
Behaviour: the popup is re-derived from the editor buffer on every edit, so it appears and disappears with
the text. Tab replaces the `@fragment` with the highlighted path and no trailing space, Enter the same path
plus a space; Esc closes without inserting.
Evidence: `crates/epsilon-tui/src/app/completion.rs:58` (`self.at_open = has_rows;`); draw `crates/epsilon-tui/src/app/panels.rs:554`.
Test: `at_file_popup_opens_and_tab_inserts_path` — `crates/epsilon-tui/src/app/tests.rs:3076`;
`the_at_popup_walks_the_workspace_once_not_once_per_keystroke` — `crates/epsilon-tui/src/app/tests.rs:2990`.
Observe: a narrow `files` panel above the input listing e.g. `src/app.rs`, with `tab insert · enter insert + space · esc close`.

**E-072 — Every offered `@` path is workspace-relative, forward-slashed, and never reached through a link.**
Behaviour: the list is built by one walk shared with the tools, so it agrees with the permission layer about
which files exist. Every entry is `strip_prefix(root)` and rewritten to forward slashes, and any entry that
is a symlink or a Windows junction is neither listed nor descended into.
Evidence: `crates/epsilon-core/src/walk.rs:110` (`if crate::paths::is_link(&path) {`); draw `crates/epsilon-tui/src/app/panels.rs:534`.
Test: `a_link_out_of_the_workspace_is_neither_listed_nor_descended` — `crates/epsilon-core/src/walk.rs:248`;
`the_list_agrees_with_the_ignore_matcher_the_tools_use` — `crates/epsilon-tui/src/at_files.rs:80`.
Observe: a gitignored file, `build/`, `target/` and `.hidden/` never appear; a symlinked-out `escape/secret.txt` is absent.

**E-073 — A typed `@` path that matches nothing simply leaves the popup closed.**
Behaviour: when the fuzzy filter returns no candidates the popup state is cleared and the draw returns before
building a panel, so no `files` panel appears — there is no "not found" row and no notice. The text stays in
the editor and is submitted verbatim.
Evidence: `crates/epsilon-tui/src/app/completion.rs:53`; draw `crates/epsilon-tui/src/app/panels.rs:495` (`if !app.at_open {`).
Test: **none found** — the nearest is `a_missing_workspace_is_an_empty_list_not_a_panic` — `crates/epsilon-tui/src/at_files.rs:141`,
which pins an unreadable workspace producing an empty list, not a fragment that matches no row.
Observe: typing `@nonexistent.rs` leaves the composer as typed with no panel above it; nothing on screen says the file is missing.

### 4.15 Search — `E-074` … `E-075`

**E-074 — `/search <query>` searches the rendered transcript; n/Down/Enter step forward, p/Up/PageUp back, both wrap.**
Behaviour: it searches the text currently in the transcript, not the session log on disk. A blank query
prints a usage notice, a query with no hit prints `no match for '…' in this transcript`, and neither leaves a
search open. A hit opens the search and jumps to the **first** match. `Esc` or `q` closes and restores the
scroll offset the reader had.
Evidence: `crates/epsilon-tui/src/app/commands.rs:1523`; `crates/epsilon-tui/src/app/view.rs:536`, `:539`, `:542`, `:617`;
`crates/epsilon-tui/src/search.rs:72`, `:93`, `:102`; arms `crates/epsilon-tui/src/app/keys.rs:135`, `:139`.
Draw: none separate — navigation mutates `scroll_from_bottom` and the jump is deferred to the next measure
(`crates/epsilon-tui/src/app/view.rs:572`).
Test: `an_open_search_reports_itself_and_the_matches_it_found` — `crates/epsilon-tui/src/app/golden.rs:1606` (frame, asserts both the matches and `1 of 2` on the screen);
`stepping_through_matches_moves_what_is_on_screen` — `crates/epsilon-tui/src/app/golden.rs:1642` (frame, with the jump disabled as a control);
`next_and_previous_wrap_rather_than_stop` — `crates/epsilon-tui/src/search.rs:173`;
`search_command_now_searches_the_transcript_and_not_the_session_log` — `crates/epsilon-tui/src/app/tests.rs:96`.
Observe: the transcript scrolls to the hit and a centred `search` panel appears over it; pressing `n` visibly jumps.

**E-075 — the open search draws a centred `search` panel: `find <query>`, a match counter, and `n next · p previous · esc close`.**
Behaviour: while a search is open a focused, centred overlay is painted with the title `search`, two body
rows (`find  <query>` and the counter, `3 of 7` or `no matches` in red), and a bottom hint row. It is drawn
last, after the permission dialog, and only when no approval dialog is up.
Evidence: `crates/epsilon-tui/src/app.rs:1720`, `:1742`, `:1743`, `:1747`, `:1754`, `:1762`, `:1781`;
draw `crates/epsilon-tui/src/app.rs:1744`, `:1746`, `:1771`; sizing `crates/epsilon-tui/src/ui/overlay.rs:53`, border `:127`.
Test: `an_open_search_reports_itself_and_the_matches_it_found` — `crates/epsilon-tui/src/app/golden.rs:1606` (frame);
`a_pending_ask_outranks_an_open_search` — `crates/epsilon-tui/src/app/tests.rs:4935`.
Observe: an accent-framed box in the middle of the transcript whose bottom row names the three keys that work.

### 4.16 Clipboard — `E-076`

**E-076 — `f4` / `/copy` copies the last assistant message; over SSH it sends OSC 52 and says "sent", not "copied".**
Behaviour: it takes the most recent `LineKind::Assistant` row — preferring its full body over the summary — and
puts it on the clipboard. Locally with a working clipboard the notice reads `copied N chars to the clipboard`.
Over SSH, when there is no local clipboard, **or even when the native write succeeded**, it writes an OSC 52
sequence (`ESC ] 52 ; c ; <base64> BEL`, wrapped in a tmux/screen passthrough when detected) and the notice
reads `copied N chars — sent via the terminal (OSC 52)`. With no assistant row it prints
`copy: no assistant message to copy` and puts nothing on the wire — an empty OSC 52 payload would *clear* the
user's clipboard.
Evidence: `crates/epsilon-tui/src/app/commands.rs:2351`, `:2352`, `:2364`, `:2375`, `:2377`;
`crates/epsilon-tui/src/clipboard.rs:215`, `:162` (the SSH env vars), `:141` (the sequence), `:149` (the tmux envelope), `:47` (`MAX_ENCODED`), `:258`;
`crates/epsilon-tui/src/app/keys.rs:1684`.
Draw: none — the notice is a `LineKind::System` row, and the OSC 52 bytes are deliberately **not** drawn:
they are parked in `app.osc_pending` (`crates/epsilon-tui/src/app.rs:1500`) and written to the terminal after
the frame is out (`crates/epsilon-tui/src/app/event.rs:246`).
Test: `copy_parks_a_sequence_that_carries_the_message_and_the_notice_names_the_path` — `crates/epsilon-tui/src/app/tests.rs:3193` (branches on whether a sequence was parked, so neither branch passes vacuously);
`the_sequence_is_written_at_the_frame_boundary_and_only_once` — `crates/epsilon-tui/src/app/tests.rs:3273`;
`the_copy_key_reaches_the_clipboard_path` — `crates/epsilon-tui/src/app/tests.rs:5140`;
`a_remote_session_sends_the_sequence_even_when_the_native_write_worked` — `crates/epsilon-tui/src/clipboard.rs:383`,
with `a_local_session_does_not_write_osc_52_when_the_native_path_worked` — `crates/epsilon-tui/src/clipboard.rs:398` — as its control.
Observe: a system row lands in the transcript saying exactly which of the two paths ran.

### 4.17 Themes — `E-077` … `E-080`

**E-077 — bare `/theme` toggles dark to light and announces the switch.**
Behaviour: with no argument it swaps to the other *brightness* and pushes a `palette: dark → light` notice.
It clears the transcript render cache so rows already drawn do not survive in the old colours. It is not
written to config — `EPSILON_THEME` / `[tui] theme` make a default permanent.
Evidence: `crates/epsilon-tui/src/app/commands.rs:1532`; `crates/epsilon-tui/src/ui/theme.rs:174`, `:176`; notice `crates/epsilon-tui/src/app/commands.rs:1553`.
Draw: none separate — the theme resolves roles per frame at `crates/epsilon-tui/src/ui/theme.rs:1062` with no repaint call.
Test: `theme_command_toggles_and_names_the_palette` — `crates/epsilon-tui/src/app/tests.rs:3317`;
`switching_the_palette_redraws_the_transcript_and_not_just_the_chrome` — `crates/epsilon-tui/src/app/golden.rs:2802` (frame — asserts the foreground of a cached transcript row changes and the text survives).
Observe: every colour changes on the next frame and a system row names the old and new palette.

**E-078 — `/theme light|dark|high-contrast` sets by name; an unknown name is refused and changes nothing.**
Behaviour: it is idempotent, accepts `dark`, `light`, `high-contrast` (also `contrast` and `hc`),
case-insensitively, with `_` standing in for `-`. An unrecognised name prints a refusal naming the options
and leaves the palette exactly as it was.
Evidence: `crates/epsilon-tui/src/app/commands.rs:1533`, `:1536`, `:1537`; `crates/epsilon-tui/src/ui/theme.rs:150`, `:153`;
the advertised names `crates/epsilon-contracts/src/command.rs:226`.
Test: `theme_command_toggles_and_names_the_palette` — `crates/epsilon-tui/src/app/tests.rs:3317` (asserts `/theme solarized` leaves the palette alone);
`every_palette_the_description_advertises_is_one_the_product_has` — `crates/epsilon-tui/src/app/tests.rs:3378` — parses the names straight out of the `/theme` description and requires each to resolve;
`the_palette_is_selected_by_name_and_survives_a_bad_one` — `crates/epsilon-tui/src/ui/theme.rs:2106`.
Observe: `/theme high-contrast` repaints to white-on-black and prints `palette: dark → high-contrast`.

**E-079 — the palette is chosen at startup, and colour + glyph capability degrade to `NO_COLOR`, ANSI-16 and the ASCII set.**
Behaviour: the palette resolves most-explicit-first — `--light`, then `EPSILON_THEME`, then `[tui] theme` —
and an unrecognised value falls back to **dark**, not to the next source. There is no `--theme <name>` flag.
`NO_COLOR` (set and non-empty) turns every role into attributes only; otherwise `COLORTERM=truecolor|24bit`,
`WT_SESSION` or any `TERM_PROGRAM` gives 24-bit colour and anything else gets the 16 ANSI slots, where body
text becomes `Color::Reset` and filled bands are withheld entirely rather than approximated. The glyph
repertoire is chosen once by `Glyphs::detect`: `EPSILON_ASCII=1|true|yes` forces seven-bit, Windows without a
UTF-8-capable host gets ASCII, everything else Unicode.
Evidence: `crates/epsilon-cli/src/main.rs:1342`, `:1364`, `:1383`, `:465`; `crates/epsilon-tui/src/ui/theme.rs:102`, `:85`, `:1086`, `:1092`, `:1264`, `:481`, `:464`.
Draw: `crates/epsilon-tui/src/ui/theme.rs:1090` — the one place `NO_COLOR`, the ANSI-16 slots and the `DIM` fallback are decided; `:1279` is what the transcript asks before relying on a band.
Test: `the_light_flag_selects_the_light_palette_regardless_of_the_environment` — `crates/epsilon-cli/src/main.rs:2348`;
`an_unrecognised_theme_value_falls_back_to_dark_not_to_the_config` — `crates/epsilon-cli/src/main.rs:2377`;
`the_repertoire_default_is_ascii_on_a_legacy_windows_console` — `crates/epsilon-tui/src/ui/theme.rs:1339`;
`ascii_glyphs_are_seven_bit_only` — `:1288`; `bands_are_withheld_when_the_terminal_cannot_do_them` — `:1439`;
`no_color_still_distinguishes_roles_by_attribute` — `:1415`; `no_color_keeps_the_roles_apart_in_the_light_palette_too` — `crates/epsilon-tui/src/app/golden.rs:1220` (frame);
`an_ascii_frame_draws_no_character_a_legacy_console_cannot` — `crates/epsilon-tui/src/app/golden.rs:270` (frame).
Observe: `NO_COLOR=1` and the screen loses hue but keeps bold/dim structure; `EPSILON_ASCII=1` and every box-drawing character becomes `+-|`.

**E-080 — `high-contrast` is a real third palette: every role at 7:1 or better on black, nothing dimmed.**
Behaviour: it resolves to a third palette table, not a variant of either other one: pure white body text on
pure black (21:1), every other foreground role measured at 7:1 or better, and the four bands held to the
opposite property. No slot is marked `quiet`, because `Modifier::DIM` would let a terminal render a measured
7:1 role below the floor — secondary is carried by lightness instead. It is reached **by name only**:
`flipped` maps `HighContrast` to `Light`, so bare `/theme` never lands on it.
Evidence: `crates/epsilon-tui/src/ui/theme.rs:936` (`pub(crate) const CONTRAST: Self = Self {`), `:955`, `:1065`, `:176`, `:117`.
Test: `the_high_contrast_palette_clears_the_aaa_floor_on_black` — `crates/epsilon-tui/src/ui/theme.rs:1545` (walks `Role::ALL`, checks each against 7:1 *and* against the dark palette's own ratio, and its control asserts the dark palette's `text_faint` *does* miss the floor);
`the_high_contrast_palette_dims_nothing_and_keeps_the_semantic_hues` — `:1685`;
`the_outcome_roles_are_tellable_apart_without_colour` — `:1642`.
Observe: `/theme high-contrast` repaints to `#ffffff` on `#000000`; bare `/theme` from there goes to **light**, not back to dark.
Captured: `audit/evidence/tui-frames-2026-10-04-m312/high-contrast.80x24.txt`.

### 4.18 Status — `E-081` … `E-084`

**E-081 — the header's left half is the run-state indicator: `idle`, or a spinner and `running <n>s · <phase>`.**
Behaviour: it reads `idle` (faint) when nothing is in flight. While a run is active it reads a spinner frame
in the accent colour followed by `running <elapsed>s`, plus ` · <phase>` where the phase is the live
`UiState` status — `run started`, `turn 0: thinking…`, `streaming…`, `running bash…`, `compacting context…`.
The measurements on the right yield first; the run state is the half that outranks them.
Evidence: `crates/epsilon-tui/src/app/view.rs:151`, `:159`; `crates/epsilon-tui/src/ui/header.rs:38`; `crates/epsilon-tui/src/state.rs:1196`, `:1488`.
Draw: `crates/epsilon-tui/src/ui/header.rs:79` (`RunState::Idle => vec![Span::styled("idle", theme.faint())],`), `:86`, `:93`, `:94`, `:72`.
Test: `a_running_call_shows_its_time_in_the_header_not_on_the_card` — `crates/epsilon-tui/src/app/golden.rs:2740` (frame);
`idle_and_running_read_differently` — `crates/epsilon-tui/src/ui/header.rs:216` (frame, via `header::draw` on a `TestBackend`);
`run_lifecycle_renders_in_order` — `crates/epsilon-tui/src/state.rs:2136`.
Observe: the first column changes between a quiet `idle` and a moving spinner with a seconds counter; `AgentEnd` puts `idle` back.

**E-082 — the footer row is the session's status line: live key hints left, workspace path and branch right.**
Behaviour: the bottom row carries only keys that work right now, resolved from the live binding table, plus
the workspace path and git branch. Which set appears depends on state: `insert` idle, `run` in flight
(`enter queue` rather than `enter send`), the approval keys on a permission ask, the overlay keys for any
overlay, and a position badge when the reader has scrolled off the tail. Hints are budgeted before the layout
runs, so a hint is dropped whole or degraded to its bare key — never cut into a key that does not exist.
Evidence: `crates/epsilon-tui/src/app/view.rs:355`, `:404`, `:415`, `:417`; `crates/epsilon-tui/src/ui/footer.rs:284`, `:285`, `:166`.
Draw: `crates/epsilon-tui/src/ui/footer.rs:411`, `:412`, `:425`.
Test: `the_jump_to_live_key_returns_the_view_to_the_tail` — `crates/epsilon-tui/src/app/golden.rs:1066` (frame);
`idle_hints_name_only_keys_that_do_something` — `crates/epsilon-tui/src/ui/footer.rs:465` (frame);
`what_survives_a_narrow_row_is_the_hint_that_cannot_be_guessed` — `:589` (frame);
`no_footer_row_ever_prints_half_a_hint_or_touches_the_location` — `:672` (frame, sweeps 20..=120);
`the_location_survives_a_narrow_terminal` — `:568` (frame).
Observe: the bottom line always reads `insert · enter send · pageup scroll back · …` with the path right-aligned.

**E-083 — `push_notice` appends a harness-authored system row to the transcript.**
Behaviour: runtime messages — run failures, `/command` outcomes, `no match for '…' in this transcript`,
`copy: no assistant message to copy`, `palette: dark → light` — are pushed as a `LineKind::System` row. They
persist and scroll with the conversation and deliberately carry no filled band, so a notice never reads as a
tool call even on a colourless terminal.
Evidence: `crates/epsilon-tui/src/state.rs:971`, `:972`; the key-path channel `crates/epsilon-tui/src/app.rs:1233`.
Draw: `crates/epsilon-tui/src/ui/transcript.rs:110`, `:111`, `:112`; `crates/epsilon-tui/src/render_cache.rs:203`.
Test: `the_answer_is_not_a_notice_when_there_is_no_band` — `crates/epsilon-tui/src/app/golden.rs:75` (frame, ANSI-16 + ASCII);
`a_tool_call_and_a_notice_are_distinguishable_on_a_real_frame_without_colour` — `crates/epsilon-tui/src/app/golden.rs:674` (frame, `NO_COLOR`);
`retry_notices_land_in_the_transcript_and_the_status_line` — `crates/epsilon-tui/src/state.rs:2113`.
Observe: a `•`-marked dim row appears in the conversation and stays there when you scroll back.

**E-084 — the footer leads with a mode badge: `insert`, `run`, `list`, `guidance`, or `approval`.**
Behaviour: the first thing on the row is a one-word badge naming which keyboard is live. It is drawn in the
accent colour, it leads the row because the same chord means different things per mode, and it is emitted
whole or not at all — `insert` cut to `in` is not a shorter mode, it is a word the UI does not use.
Evidence: `crates/epsilon-tui/src/app/view.rs:362`, `:379`, `:381`, `:395`; `crates/epsilon-tui/src/ui/footer.rs:163`, `:259`, `:271`.
Draw: `crates/epsilon-tui/src/ui/footer.rs:357` (`let span = Span::styled(mode.clone(), theme.accent());`), `:359`.
Test: `the_mode_leads_the_row` — `crates/epsilon-tui/src/ui/footer.rs:537` (frame — asserts `starts_with("insert · ")` and `run.starts_with("run · ")`);
`approval_hints_are_the_approval_keys` — `:559`; `the_approval_footer_never_prints_half_a_hint_either` — `:745` (frame, sweeps 20..=120).
Observe: `run · enter queue` and `insert · enter send` are the same chord and different promises.

### 4.19 Metrics — `E-085` … `E-089`

**E-085 — Header usage gauges: session context share and session token totals.**
Behaviour: the header's right half prints `ctx 42%/200k` — the session context the next request will carry,
as a share of the model's window — beside `↑12k ↓3.4k`. A share that rounds away prints `<1%`, an unmeasured
context prints `?` rather than a zero, and a local estimate carries a `*` after the percentage.
Evidence: `crates/epsilon-tui/src/ui/metrics.rs:272` (`Some(ratio) => format!("ctx {}{mark}/{window}", percent_or_floor(ratio)),`);
draw `crates/epsilon-tui/src/ui/header.rs:67`.
Test: `the_context_figure_is_the_session_and_not_the_last_prompt` — `crates/epsilon-tui/src/app/tests.rs:2815` (frame);
`a_real_frame_draws_the_context_the_agent_reports` — `crates/epsilon-tui/src/app/tests.rs:2872` (frame).
Observe: the top row of the terminal; on a wide window the first turns read `ctx <1%/200k`. Captured:
`audit/evidence/tui-frames-2026-10-04-m312/idle-with-content.80x24.txt:4` (`?/1.0M`).

**E-086 — As the context fills and the row narrows: warn, critical, and whole gauges only.**
Behaviour: at 70% of the window the context gauge changes to the warning style and at 90% to critical. A
narrow terminal drops whole gauges from the tail, never half of one, so a cut `↑0 ↓0` cannot appear; the model
name and its `offline` qualifier are pinned and survive every width.
Evidence: `crates/epsilon-tui/src/ui/metrics.rs:186` (`Some(r) if r >= CONTEXT_WARN => Pressure::Warn,`), thresholds `:29`, `:31`;
draw `crates/epsilon-tui/src/ui/header.rs:160`.
Test: `a_narrow_terminal_keeps_the_model_and_the_context_gauge` — `crates/epsilon-tui/src/ui/header.rs:454` (frame).
Observe: drag the terminal narrower; cost and cache rate leave the row first, the context gauge and the model stay, and the gauge's colour shifts as the context fills.

**E-087 — Compaction: automatic on pressure or on `/compact`, and what the row says.**
Behaviour: the loop compacts on its own once the estimated context passes the window minus its reserve, and
`/compact` does it on demand (refused "compact already running" while one is in flight; queued behind a live
turn). The finished compaction prints a transcript row naming input, output and total tokens re-read, and the
header's gauge reads `ctx ? · compacted` until a reply re-measures it.
Evidence: `crates/epsilon-core/src/lib.rs:2692` (`tokens > limit`); draw `crates/epsilon-tui/src/state.rs:1445`
(`"── context compacted · ↑{} ↓{} · {} re-read ──",`); queueing `crates/epsilon-tui/src/app/commands.rs:1561`; gauge `crates/epsilon-tui/src/ui/metrics.rs:246`.
Test: `a_compaction_row_reports_the_same_tokens_it_records` — `crates/epsilon-tui/src/app/tests.rs:5238`.
Observe: a `── context compacted · ↑84k ↓1.2k · 92k re-read ──` rule appears mid-transcript. Captured:
`audit/evidence/tui-frames-2026-10-04-m312/streaming.80x24.txt:6`.

**E-088 — Cumulative spend: `$0.012`, `$0.012+` for a lower bound, nothing when unpriced.**
Behaviour: the header carries session cost in integer micro-USD priced per model from the catalog; a trailing
`+` marks a lower bound (tokens the registry does not price) and an unpriced model contributes to that mark
instead of costing nothing; a model with no registry row yields no cost fragment at all rather than a
confident `$0`.
Evidence: `crates/epsilon-tui/src/ui/cost.rs:138` (`floor: priced_somewhere && moved_unpriced,`); draw `crates/epsilon-tui/src/ui/metrics.rs:375`;
per-component total `crates/epsilon-tui/src/app/view.rs:313`.
Test: `a_floor_cost_is_marked_rather_than_reported_as_a_total` — `crates/epsilon-tui/src/ui/metrics.rs:1089`.
Observe: the `$…` fragment between the cache gauge and the model name, and the `cost:` line from `/session`.

**E-089 — The footer's position badge: `↑ 12/40`, whole or not at all.**
Behaviour: when the reader scrolls up the transcript stops following the run and the footer says so with a
position badge — the theme's back glyph plus rows-below over total rows — beside the key back to live. It
counts rows *below* the window, and it is emitted whole or dropped.
Evidence: `crates/epsilon-tui/src/ui/footer.rs:77` (`"{} {}/{}",`); draw `crates/epsilon-tui/src/ui/footer.rs:389`.
Test: `the_position_badge_is_never_half_written_at_any_width` — `crates/epsilon-tui/src/ui/footer.rs:607` (frame);
`the_frozen_row_at_sixty_columns_keeps_the_position_and_the_exit` — `crates/epsilon-tui/src/ui/footer.rs` (the measured 60-column row is recorded at `crates/epsilon-tui/src/ui/footer.rs:187-204`).
Observe: PageUp during a stream: the mode word stays `insert`, the badge appears in the warning colour, and at 60 columns beside a realistic path the exit key `g` is what survives.

### 4.20 Goals — `E-090` … `E-093`

**E-090 — `/goal <objective>` creates and arms a goal, spends round 1 at once, badges the footer.**
Behaviour: it writes a goal to the session log, arms continuation in this process, and immediately spends
round 1 rather than parking an objective — create or resume is what starts work, `pause` parks one. The footer
grows `● goal 1/256` (filled mark only while armed and active).
Evidence: `crates/epsilon-tui/src/app/commands.rs:776` (`let starts_work = matches!(`); draw `crates/epsilon-tui/src/ui/footer.rs:377`.
Test: `goal_creates_records_and_renders_the_badge` — `crates/epsilon-tui/src/app/goal_tests.rs:28` (frame).
Observe: the badge sits immediately after the mode word on the footer row and is never cut to `● goal 2/2`.

**E-091 — Reading and steering the goal: bare `/goal`, the verbs, visible refusals.**
Behaviour: bare `/goal` prints phase, rounds of cap and armed state, then the objective on its own line, a
blocker if any, then id and revision; with no goal it says `no goal is set`. `edit`, `pause`, `resume`,
`complete`, `blocked <code> <reason>` and `clear` move it through its phases, and a verb is a verb only when
it is the **whole** argument — `/goal clear the failing gates` creates a goal.
Evidence: `crates/epsilon-runtime/src/lib.rs:4257` (`"edit" => return parse_goal_edit(tail),`); draw `crates/epsilon-tui/src/app/commands.rs:801`.
Test: `the_verbs_move_the_goal_through_its_phases` — `crates/epsilon-tui/src/app/goal_tests.rs:81`;
exhaustive verb coverage `every_verb_reaches_its_operation` — `crates/epsilon-runtime/src/lib.rs:4378`.
Observe: `/goal edit rounds zero` answers `positive number` rather than creating a goal with that name.

**E-092 — What a goal round does: turn-end driver, gate, and the cap.**
Behaviour: at a turn end an armed, active, under-cap goal with an empty composer spends its next round durably
and echoes a harness-authored `Continuing the session goal` prompt into the transcript — never into the recall
ring. Declared gates run first, on their own task, and a failing gate's bounded output **is** the round's
prompt; the round is spent either way. The cap stops the loop and names `/goal edit rounds <n>`.
Evidence: `crates/epsilon-runtime/src/lib.rs:1467` (`for gate in &gates {`); draw `crates/epsilon-tui/src/app/commands.rs:944` (`"goal round {}/{} — continuing",`).
Test: `a_failing_gate_becomes_the_continuation_prompt` — `crates/epsilon-runtime/tests/goal_round_gates.rs:98`.
Observe: with gates set the round line appears only after the gate answers; a failing gate announces itself first, then the prompt names the command and its exit code.

**E-093 — The goal lives in the session log, and aborting a run disarms it.**
Behaviour: a goal is durable state in the session log and nowhere else
(`crates/epsilon-tui/src/app/tests_support.rs:59`), so a session with no store refuses `/goal` outright
instead of claiming the session has no objective. Esc during a run stops the run **and** disarms continuation —
goal, objective, phase and round count untouched, no round spent — and the harness says so.
Evidence: `crates/epsilon-tui/src/app/keys.rs:1369` (`agent.set_goal_armed(false);`); the badge `crates/epsilon-tui/src/app/view.rs:433`.
Test: `aborting_a_run_disarms_the_goal_without_touching_it` — `crates/epsilon-tui/src/app/goal_tests.rs:398`.
Observe: Esc mid-run prints `goal disarmed by the abort — /goal resume continues it`; at the next turn end the footer mark flips `●` to `○` while the round counter stands still.

### 4.21 Trust — `E-094` … `E-095`

**E-094 — answering the trust review writes two files, so the next run trusts without asking.**
Behaviour: answering writes the accepted config hash to `<workspace>/.epsilon/trust.json` and the accepted path
to `<global_dir>/trust.json`; both are read back on later runs, so the same workspace is not asked twice.
Trusting a parent directory covers every child, because `get` walks to the nearest ancestor entry.
Evidence: `crates/epsilon-tui/src/trust_store.rs:132` (`std::fs::rename(&tmp, &self.path)` — atomic publish),
`:44` (`path: dir.join("trust.json"),`), `:55`, `:160`; the hash write `crates/epsilon-core/src/config.rs:230`;
`/trust` reading it back `crates/epsilon-tui/src/app/commands.rs:2113`.
Draw: none — the confirmation is a transcript notice, `crates/epsilon-tui/src/app.rs:807`.
Test: `empty_store_asks_set_get_clear_roundtrip` — `crates/epsilon-tui/src/trust_store.rs:235`;
`nearest_ancestor_wins_and_child_overrides` — `crates/epsilon-tui/src/trust_store.rs:249`.
Observe: typing `trust` while the review holds prints two lines and, on a later launch in the same directory, prints no review at all.

**E-095 — while the review holds, every submit is bounced; `trust` accepts, `trust no` quits.**
Behaviour: when a changed project config is flagged at startup, typing anything at all — a prompt or a slash
command — is refused with a notice telling you to answer the review. Only `trust` / `trust yes` / `y` clears
it; only `trust no` / `n` quits. The palette is refused the same way, and a `--tui` task is staged into the
composer rather than run.
Evidence: `crates/epsilon-tui/src/app.rs:786`; the bounce `:835`, the quit `:831`;
the palette refusal `crates/epsilon-tui/src/app/keys.rs:1447`, `:1449`; the startup wording `crates/epsilon-tui/src/app/event.rs:93`, staging `:110`.
Draw: none — the gate has no widget; it answers in the transcript.
Test: `trust_review_holds_submits_until_answered` — `crates/epsilon-tui/src/app/tests.rs:2256`;
`trust_no_quits_instead_of_running_against_unreviewed_config` — `:2280`;
`trust_gate_blocks_palette_and_stages_initial_task` — `:2290`.
Observe: type a normal question after a config change; nothing reaches the model and the `config review pending` line is all you get.

### 4.22 Reload — `E-096` … `E-100`

**E-096 — `/reload` re-reads skills, agent defs, prompt templates and keybindings from disk with no restart.**
Behaviour: it re-scans the skills roots (global `skills/`, plus `.agents/skills` and `.epsilon/skills`), re-scans
the agent-def roots, re-discovers prompt templates, and re-reads `keybindings.json`, all in the running process.
What is **not** re-read is `config.toml`.
Evidence: `crates/epsilon-tui/src/app/commands.rs:1401` (`let found = epsilon_core::skills::discover_with_notices(&roots);`),
`:1403`, `:1414`, `:1450-1452` (`self.keybindings.reload();`), `:1443`.
Draw: `crates/epsilon-tui/src/app/commands.rs:1458-1459` — the single summary line.
Test: `a_skill_root_reload_cannot_read_is_named_on_the_frame` — `crates/epsilon-tui/src/app/commands.rs:2735` (frame).
Observe: edit a `SKILL.md`, type `/reload`, and the transcript's last line reads
`reload: 1 skill(s) + 1 agent(s) loaded (prompt tier re-read on next turn)` with no restart.

**E-097 — a root `/reload` cannot read is named on the frame, with the reason; what it could read still loads.**
Behaviour: if a skills or agents root cannot be enumerated, the transcript names that root and why rather than
printing a bare count — and the entries that were readable still load.
Evidence: `crates/epsilon-tui/src/app/commands.rs:1409` (`for notice in &found.notices {`), `:1410` (`self.ui.push_notice(notice.clone());`).
Test: `a_skill_root_reload_cannot_read_is_named_on_the_frame` — `crates/epsilon-tui/src/app/commands.rs:2735` (frame, at 80x24 and 100x30, and asserting the readable half at `crates/epsilon-tui/src/app/commands.rs:2780`);
`an_agents_root_reload_cannot_read_is_named_on_the_frame` — `crates/epsilon-tui/src/app/commands.rs:2798` (frame).
Observe: make `.agents/skills` a regular file; the transcript carries `skills: cannot read root <path>: <reason>` above the summary line.

**E-098 — `/skills` prints one line per skill with its description, marks `[off]` and ends with an enabled count.**
Behaviour: it lists every discovered skill as `name — description`, appends `(manual only)` where the model may
not invoke it, appends `[off]` where it has been switched off, and closes with how many of how many are enabled.
Evidence: `crates/epsilon-tui/src/app/commands.rs:321` (`let mut line = format!("{} — {}", s.name, s.description);`),
push `:1878`, empty message `:1872`, closing line `:1882`.
Test: `the_skills_listing_carries_a_description_per_skill` — `crates/epsilon-tui/src/app/tests.rs:4459` (calls `skill_lines` directly, so it pins the row formatter rather than the command's use of it);
`no_slash_command_panics_when_typed` — `:3671` — is what actually types `/skills`.
Observe: each row is a sentence, not a name.

**E-099 — `/skill <name> on|off` switches one skill, and refuses a name that is not installed.**
Behaviour: `/skill <name> off` (or `disable`) disables and `on` (or `enable`) re-enables, answering
`<name> off — takes effect on the next turn`. A name no skill provides is refused, so no dead setting is
written; a missing `on|off` answers with the usage line.
Evidence: `crates/epsilon-tui/src/app/commands.rs:693` (`let known = agent.skills().iter().any(|s| s.name == name);`),
refusal `:696`, usage `:679`, success `:702`.
Test: **none found** — no test in `crates/epsilon-tui` types `/skill`. The closest is the parser test
`a_skill_toggle_takes_a_name_and_an_explicit_on_or_off` — `crates/epsilon-contracts/src/command.rs:764`,
which pins only that `/skill commit off` carries the tail.
Observe: `/skill comit off` reads `no skill named 'comit' — `/skills` lists what is available`, not a saved setting.

**E-100 — `/templates` lists `/name — description (takes hint)`, or names the two directories when none is installed.**
Behaviour: it prints one entry per discovered prompt template as `/name — description`, adding
`(takes <hint>)` when the template declares an argument hint, and lists rather than expands. With no templates
it says which two directories to drop files into.
Evidence: `crates/epsilon-tui/src/app/commands.rs:1900` (`let mut line = format!("/{} — {}", t.name, t.description);`),
`:1907`, empty message `:1895`.
Test: `templates_command_lists_names_descriptions_and_hints` — `crates/epsilon-tui/src/app/tests.rs:3872`;
`templates_command_explains_where_to_put_files_when_empty` — `crates/epsilon-tui/src/app/tests.rs:3911`.
Note: `discover_prompt_templates` skips the project roots while the trust review holds
(`crates/epsilon-tui/src/app.rs:742`), so an untrusted workspace's `/templates` is the user's own list only.
Observe: a run-on line of every template with what it does, not a list of names.

### 4.23 View — `E-101` … `E-104`

**E-101 — PageUp / PageDown move the transcript by a fixed ten rows, and the footer names the chord that actually fires.**
Behaviour: with the composer idle, PageUp moves the transcript ten rows further from the tail and PageDown
ten rows back; both saturate, so PageDown at the tail does nothing and PageUp past the top stays pinned.
The chord is read from the binding table, so a rebound key moves both the behaviour and the footer's text.
Evidence: `crates/epsilon-tui/src/app/keys.rs:1642`, `:1643` (`app.scroll_from_bottom = app.scroll_from_bottom.saturating_add(10);`), `:1647`;
`crates/epsilon-tui/src/keybindings.rs:50`, `:51`; draw `crates/epsilon-tui/src/app/view.rs:523`
(`skip: crate::scroll::transcript_skip(total, visible, self.scroll_from_bottom),`), which clamps rather than
running away at `crates/epsilon-tui/src/scroll.rs:30`.
Test: `the_advertised_scroll_key_scrolls_the_real_frame` — `crates/epsilon-tui/src/app/golden.rs:945` (frame);
`transcript_follow_and_clamp` — `crates/epsilon-tui/src/scroll.rs:57`.
Observe: the footer on a live view prints `pageup scroll back · pagedown scroll forward`
(`crates/epsilon-tui/src/ui/footer.rs:225-226`), and pressing PageUp moves ten transcript rows under a static composer.

**E-102 — Ctrl+Up / Ctrl+Down jump to the previous / next user turn, and say so at the ends.**
Behaviour: Ctrl+Up (or Ctrl+Shift+Up) lands on the nearest *user* row strictly above the current position;
Ctrl+Down the nearest user row below. Tool results and assistant rows are skipped. At either end the key
pushes a notice — `no earlier prompt` / `no later prompt` — rather than clamping silently.
Evidence: `crates/epsilon-tui/src/app/keys.rs:1660`, `:1663`, `:1710`; `crates/epsilon-tui/src/keybindings.rs:58`, `:60`;
draw — none separate; the jump only moves `scroll_from_bottom` and the rows appear through the same window as E-101.
Test: `a_prompt_jump_lands_on_a_user_turn_and_reports_the_ends` — `crates/epsilon-tui/src/app/tests.rs:5055`,
which drives `jump_to_prompt` (`crates/epsilon-tui/src/app.rs:1933`) directly and does **not** go through
`handle_key`, so the two notice strings at `crates/epsilon-tui/src/app/keys.rs:1663` and `:1710` are not pinned by any test.
Observe: one press jumps over a run of tool output straight to the next thing you typed; pressing it past the first turn prints `no earlier prompt`.

**E-103 — Jump to live (`g` / `G`) returns the view to the tail, and only exists while the view is frozen.**
Behaviour: it sets the offset back to zero, which is the same value the follow-scroll uses, so the transcript
resumes tracking the run. The arm is guarded on the view actually being frozen, so while the view is live the
letter types into the composer. An arriving permission ask also snaps the view back to live.
Evidence: `crates/epsilon-tui/src/app/keys.rs:1819` (`_ if app.scroll_from_bottom > 0 && app.keybindings.matches("app.transcript.live", &key) => {`), `:1820`;
`crates/epsilon-tui/src/app.rs:1470`; `crates/epsilon-tui/src/app/event.rs:280`; `crates/epsilon-tui/src/keybindings.rs:73`.
Draw: `crates/epsilon-tui/src/ui/footer.rs:388` and `:76` — the badge appearing and disappearing **is** the
visible half of the jump; `frozen` is derived from the same field at `crates/epsilon-tui/src/app/view.rs:351`.
Test: `the_jump_to_live_key_returns_the_view_to_the_tail` — `crates/epsilon-tui/src/app/golden.rs:1066` (frame);
`a_live_frame_never_advertises_the_frozen_position` — `crates/epsilon-tui/src/app/golden.rs:1117` (frame).
Observe: while frozen the footer reads `insert ↑ 12/40 · g jump to live`; pressing `g` brings the tail rows back and the badge goes with them.

**E-104 — Ctrl+Home / Ctrl+End jump to the two ends of the transcript.**
Behaviour: Ctrl+Home (or Ctrl+Shift+Home) pins the view to the top by setting the offset to the measured
total row count; Ctrl+End sets it to zero, the live tail. Plain `Home` / `End` are **not** these — they drive
the composer caret, and they are matched later in the same handler.
Evidence: `crates/epsilon-tui/src/app/keys.rs:1699`, `:1700`, `:1704`; `crates/epsilon-tui/src/keybindings.rs:123`, `:124`;
`crates/epsilon-tui/src/app/keys.rs:1725` (`KeyCode::Home => {`) — the unguarded caret arm sits *after* the binding guard, so `ctrl+home` reaches the scroll arm and bare `home` does not.
Draw: none separate — the offset goes through the same `transcript_skip` window, and `geometry.total.max(1)`
is deliberately past the last row so `saturating_sub` clamps rather than scrolling into rows that do not exist.
Test: **none found** — a grep for `app.transcript.top` / `app.transcript.bottom` across `crates/epsilon-tui/src`
returns only the two `keybindings.rs` declarations and the two `keys.rs` arms; the only `KeyCode::Home` in the
test tree is `crates/epsilon-tui/src/app/tests.rs:2713`, which exercises the composer caret. Nothing presses
`ctrl+home` or `ctrl+end`, on state or on a frame.
Observe: the transcript jumps to the first / last row and the badge reports the resulting position. **Untested on both the state and the frame.**

### 4.24 Editor — `E-105`

**E-105 — `/editor` and `app.editor.external` (Ctrl+O / Alt+E) hand the draft to `$EDITOR` and read it back.**
Behaviour: both set one request flag; the event loop, not the key handler, performs the hand-off between
frames. The draft is cloned, written to a temp `prompt.md`, the editor is spawned with the terminal handed to
it (alternate screen and raw mode released, then re-initialised and fully cleared on return), and the file is
read back with trailing newlines stripped and installed as the new draft. With no editor configured the flag
is never set and an actionable notice is pushed instead; a non-zero exit leaves the draft exactly as it was.
Evidence: `crates/epsilon-tui/src/app/keys.rs:1783`; `crates/epsilon-tui/src/app/session.rs:218`, `:220`;
`crates/epsilon-tui/src/app/editor.rs:36` (the no-editor sentence, naming `$EDITOR`/`$VISUAL` and the
`[editor] command` config key), `:56` (`editor exited with status {code} — draft kept`), `:61`;
the hand-off at `crates/epsilon-tui/src/app/event.rs:143`; the success notice `crates/epsilon-tui/src/app/editor.rs:129`.
Draw: none — the UI leaves the screen, the editor owns the terminal, and the returning frame is a full
repaint (`crates/epsilon-tui/src/app/editor.rs:106` restores, `:116` clears); the text reaches the screen
through the ordinary composer at `crates/epsilon-tui/src/app/editor.rs:128`.
Test: `external_editor_round_trip_writes_hands_over_and_reads_back` — `crates/epsilon-tui/src/app/tests.rs:979`;
`external_editor_keeps_the_draft_when_the_editor_fails` — `:993`;
`editor_command_prefers_config_then_visual_then_editor` — `:952`;
`editor_keybinding_and_slash_arm_the_same_request` — `:1018`;
`editor_request_refuses_during_a_run_and_reports_a_missing_editor` — `:1001`.
Gap: all five run the injected-spawner seam or the request flag; nothing asserts a frame and nothing drives
`open_external_editor` against a real terminal, so the alternate-screen hand-off is untested.
Observe: Ctrl+O prints `opening vim — the UI returns when the editor exits`, the screen clears to the editor, and on exit the composer is repainted with the edited text and `editor: draft updated`.

### 4.25 Startup — `E-106` … `E-107`

**E-106 — Startup takes the alternate screen and raw mode; a `Drop` guard restores them on every exit, panic or not.**
Behaviour: `run` initialises the ratatui terminal, arms the restore guard *before* anything that can panic, and
installs the panic hook after the guard. On a clean exit bracketed paste is disabled and the guard is dropped;
on a panic the hook runs the same restore steps first and the guard's `Drop` runs again during unwinding. Every
step is `let _ =` and idempotent, so restoring twice is harmless. Only bracketed paste is enabled — mouse
capture stays off so drag-select still copies.
Evidence: `crates/epsilon-tui/src/app.rs:1401` (`let mut terminal = match ratatui::try_init() {`), `:1410`
(`let guard = TerminalGuard;`), `:1412` (the mouse-capture comment), `:1415`, `:1349`, `:1353`, `:1433`
(`drop(guard); // restores; the explicit calls below are then redundant`).
Draw: none — the user-visible effect is that the TUI occupies a full alternate screen and the shell's own
scrollback is untouched, and that the console is usable again afterwards.
Test: `the_binary_starts_a_tui_and_draws_its_chrome` — `crates/epsilon-cli/tests/terminal.rs:327` (frame —
the real-pty test that only passes if the binary entered raw mode and drew into the terminal it was given);
`the_terminal_guard_restore_is_reachable_and_idempotent` — `crates/epsilon-tui/src/app/panic_tests.rs:41`.
Observe: launching replaces the shell prompt with a full-screen UI; quitting returns the prompt with echo and line editing intact.

**E-107 — Before the first prompt the reader gets the welcome panel plus any startup notices, never a blank body.**
Behaviour: the transcript region paints the welcome panel when the transcript and the streaming tails are all
empty, and startup notices (config diagnostics, a lease-wait summary, the trust-review line, and a note about
earlier sessions in this workspace) are pushed before the first frame, so they survive the alternate screen
taking the display. If the project config changed since it was trusted, the first submit is held until
`trust` or `trust no` is typed and the task is staged in the composer instead.
Evidence: `crates/epsilon-tui/src/app/view.rs:452`; `crates/epsilon-tui/src/ui/shell.rs:88`, `:89`;
`crates/epsilon-tui/src/app/event.rs:88`, `:93`; `crates/epsilon-tui/src/app/commands.rs:859`;
`crates/epsilon-cli/src/main.rs:1618`.
Draw: `crates/epsilon-tui/src/app.rs:1664` — the branch that replaces the transcript with the welcome panel
inside `crate::ui::shell::paint`; the welcome text itself at `crates/epsilon-tui/src/ui/transcript.rs:419-424`.
Test: `an_empty_session_gets_the_first_run_screen_inside_a_well_formed_frame` — `crates/epsilon-tui/src/app/golden.rs:509` (frame);
`a_launch_in_a_workspace_with_earlier_sessions_says_so` — `crates/epsilon-tui/src/app/startup_tests.rs:51`,
with siblings at `:66`, `:96` and `:120`.
Note: `the_welcome_screen_is_not_a_blank_void` — `crates/epsilon-tui/src/ui/transcript.rs:416` — renders the
welcome widget directly rather than through `render_rows`, so it does not cover the production composition.
Observe: a fresh launch shows the spaced `E P S I L O N` panel with `Describe a task` and the three composer
prefixes, and any startup notice as the first transcript row above it. Captured: `audit/evidence/tui-frames-2026-10-04-m312/brand-new.60x20.txt:6-10`.

### 4.26 Terminal — `E-108` … `E-110`

**E-108 — Resizing is not a key event: the next frame re-measures and the cache rebuilds on the new width.**
Behaviour: `TermEvent::Resize` is deliberately unmatched — the event handler's catch-all ignores it — because
ratatui's `Terminal::draw` re-queries the size on every frame. Each frame recomputes the transcript body width
and the cache's key includes it, so a resize invalidates every cached row wholesale. Below the layout floor
the draw path stops composing the shell entirely: it clears the "which ask did a frame actually paint"
record, so no key can answer an approval the user never saw, and paints the too-small message plus the pending
ask's capability and pattern — because a silently hidden decision reads as a hang.
Evidence: `crates/epsilon-tui/src/app/keys.rs:1902` (`_ => {}`); `crates/epsilon-tui/src/app.rs:498`, `:1596`, `:1597`;
`crates/epsilon-tui/src/render_cache.rs:292`.
Draw: `crates/epsilon-tui/src/app.rs:1614` (`f.render_widget(`) — the only draw for this case: a centred,
wrapped paragraph replacing the whole frame.
Test: `resizing_mid_session_keeps_every_row_inside_the_new_width` — `crates/epsilon-tui/src/app/golden.rs:489`
(frame — renders 120x30, 50x14, 120x30 and 44x10 on one app and asserts every row is exactly `width` columns each time);
`below_the_floor_the_frame_says_so_instead_of_drawing_a_broken_layout` — `crates/epsilon-tui/src/app/golden.rs:474` (frame);
`a_narrow_terminal_is_survived_rather_than_mangled` — `crates/epsilon-cli/tests/terminal.rs:567` (frame).
Observe: dragging the window edge reflows the transcript with no stale wrapping; shrinking below the floor replaces the whole UI with a `too small` message.

**E-109 — A panic restores the terminal first, then prints two readable lines on a working console.**
Behaviour: the hook runs the same restore steps as the guard before it prints, so the crash message lands on a
console with echo and line editing back rather than disappearing into a raw-mode alternate screen. Every panic
payload shape gets a message (`&str`, `String`, `u32`, and a fallback for anything else), so an `assert_eq!`
payload cannot cause a second crash inside the hook. The hook then chains to the previous one.
Evidence: `crates/epsilon-tui/src/app.rs:1391`, `:1392` (`eprintln!("\nepsilon crashed: {}", payload_message(info.payload()));`), `:1393`, `:1379`, `:1359`;
the probe hook at `crates/epsilon-tui/src/app.rs:1428`, gated on an env var.
Draw: none — there is no frame; the reader sees the panic on the *normal* screen after the alternate screen is torn down.
Test: `the_panic_hook_survives_both_payload_shapes` — `crates/epsilon-tui/src/app/panic_tests.rs:71`;
`the_terminal_guard_restore_is_reachable_and_idempotent` — `crates/epsilon-tui/src/app/panic_tests.rs:41`.
Both are structural — `crates/epsilon-tui/src/app/panic_tests.rs:12-39` says so explicitly — and no frame test
exists; the observable console consequence is pinned only on the ordinary path by the pty test at
`crates/epsilon-cli/tests/terminal.rs:377`.
Observe: after a crash you are left at a normal shell prompt with `epsilon crashed: <message>` above it, rather than a silent console.

**E-110 — Capability degradation is decided once per process: `NO_COLOR` drops to attributes, a legacy Windows console drops to ASCII.**
Behaviour: `ColorMode::detect` returns `None` when `NO_COLOR` is set and non-empty, `Truecolor` for
`COLORTERM=truecolor`/`24bit` or an advertised `WT_SESSION`/`TERM_PROGRAM`, and `Ansi16` otherwise.
`Glyphs::detect` honours an `EPSILON_ASCII=1|0` override in both directions, then falls back to ASCII on Windows
unless Windows Terminal, VS Code's terminal or a VT-capable `TERM` is present. When colour is off,
`theme.has_surfaces()` is false so no filled band is claimed, and roles are carried by gutter glyphs and by
bold/dim/italic instead.
Evidence: `crates/epsilon-tui/src/ui/theme.rs:85`, `:481`, `:415`, `:462`; `crates/epsilon-tui/src/app/golden.rs:706`.
Draw: `crates/epsilon-tui/src/ui/footer.rs:78` — every glyph the chrome draws, including the frozen-view badge,
is read from the theme rather than written as a literal, which is what makes one `Glyphs` table the whole switch.
Test: `an_ascii_frame_draws_no_character_a_legacy_console_cannot` — `crates/epsilon-tui/src/app/golden.rs:270`
(frame — paints 12 frames, 3 sizes x plain frame / picker / palette / slash popup, with the ASCII table);
`a_tool_call_and_a_notice_are_distinguishable_on_a_real_frame_without_colour` — `crates/epsilon-tui/src/app/golden.rs:674` (frame);
`an_answer_and_its_reasoning_are_distinguishable_on_a_real_frame_without_colour` — `crates/epsilon-tui/src/app/golden.rs:766` (frame);
`the_repertoire_default_is_ascii_on_a_legacy_windows_console` — `crates/epsilon-tui/src/ui/theme.rs:1339`;
`ascii_glyphs_are_seven_bit_only` — `crates/epsilon-tui/src/ui/theme.rs:1288`.
Observe: on `NO_COLOR=1` the same session renders with no hues and no filled bands, and a pending tool call is still distinguishable from a notice by its gutter character.

---

## 5. Rows not reconstructed

**None.** All 110 ids were established from source in this pass, and every one carries at least one
`file:line`. Where the behaviour exists but **no test pins it**, the row says `none found` and names what was
read; those are collected in §7 rather than hidden here.

Two rules were honoured rather than bent:

- **No id was filled with a plausible sentence.** The instruction allowed a `not established` row; none was
  needed, because each area's source carried more distinct user-noticeable rules than its id budget.
- **No test was named that does not exist.** Every test name in §4 was read at its `fn` declaration line in the
  tree. Where the survey's row would have named a test, this file either names the real one or writes `none found`.

### Behaviours the source establishes that no id covered

Recorded rather than dropped, because losing them is how a rule goes unowned:

| Behaviour | Where | Test |
|---|---|---|
| The cache hit-rate gauge `R18k W2.0k CH 75%`, and the distinction between `CH n/a`, `CH —` and a measured `0%` | `crates/epsilon-tui/src/ui/metrics.rs:291` | `a_provider_that_cannot_report_cache_never_prints_a_percentage` — `crates/epsilon-tui/src/ui/metrics.rs:843`; `cache_rate_is_about_the_latest_prompt_not_the_session` — `:979` |
| Ctrl+] / Ctrl+Alt+] arm a jump-to-character that consumes one typed character and then ends | `crates/epsilon-tui/src/app/keys.rs:1511`, `:1518` | none found |
| `/changelog`, `/share`, `/scoped-models`, `/expand` | `crates/epsilon-tui/src/app/commands.rs` | driven only by `no_slash_command_panics_when_typed` — `crates/epsilon-tui/src/app/tests.rs:3671` |
| `/tree` is a second spelling of `/timeline`, not a separate behaviour | `crates/epsilon-contracts/src/command.rs:507` | `tree_and_timeline_open_one_picker_and_say_which_one_ran` — `crates/epsilon-tui/src/app/tests.rs:3446` |

---

## 6. How a reader observes this

A row's `observe` field is meant to be repeatable. Two mechanisms exist.

**The captured frames.** `audit/evidence/tui-frames-2026-10-04-m312/` holds 21 screens — 7 scenarios
(`brand-new`, `idle-with-content`, `streaming`, `tool-cards`, `palette-open`, `permission`, `high-contrast`)
at 60x20, 80x24 and 100x30. Rows above cite these where the frame shows the rule directly. They are a
captured artefact, not a live read: they were taken at M312 and **nothing since then has re-taken them**, so
a reader checking a row against a frame is checking against M312's tree.

**The in-tree frame harness.** `render_rows` / `render_buffer`
(`crates/epsilon-tui/src/app/tests_support.rs:109` and `:119`) call `draw` itself, so a `frame` test above
exercises the production composition rather than a hand-assembled one. `row_text`
(`crates/epsilon-tui/src/app/tests_support.rs:93`) exists because `Cell::symbol()` coalesces adjacent identical
cells — a naive read-back doubles `100%` to `100%%`, which makes a buggy frame look fine and a correct one look broken.

**No test in this file was run for this audit.** The integrator owns the build. Every `frame` / `state only`
label above is a reading of the test's source, not a report of a green run.

---

## 7. Open findings: confirmed, corrected, and still open

Each of these was re-opened in this pass. "Confirmed" means the source says what is claimed; "corrected"
means the claim is wrong in a way that matters.

### 7.1 The footer advertises the wrong keyboard while a transcript search is open — CONFIRMED, and one arm is worse than a no-op

`overlay_open()` (`crates/epsilon-tui/src/app/view.rs:437-446`) lists picker, palette, auth prompt, label
prompt, auth pick, timeline, session browser and pending ask. **`self.search` is not in that list.** So the
branch at `crates/epsilon-tui/src/app/view.rs:383` falls straight through to `:404` and the row is the running
or idle hint set built at `crates/epsilon-tui/src/ui/footer.rs:140-143`:

```
insert · enter send · pageup scroll back · pagedown scroll forward · ctrl+j newline · ctrl+p palette
```

pinned verbatim by `idle_hints_name_only_keys_that_do_something` at `crates/epsilon-tui/src/ui/footer.rs:469-476`.

What each advertised key actually does inside `handle_search_key` (`crates/epsilon-tui/src/app/keys.rs:121-153`,
routed to at `crates/epsilon-tui/src/app/keys.rs:553`):

| Advertised | Actual | Where |
|---|---|---|
| `enter send` | **next match** | `crates/epsilon-tui/src/app/keys.rs:135` |
| `pagedown scroll forward` | notice: `Page Down is not bound in the search view — n/p step, esc or q close` | catch-all `crates/epsilon-tui/src/app/keys.rs:147-152` |
| `ctrl+j newline` | the same notice (`handle_search_key` matches `key.code` and ignores modifiers) | `crates/epsilon-tui/src/app/keys.rs:147-152` |
| `ctrl+p palette` | **silently steps to the previous match** | `crates/epsilon-tui/src/app/keys.rs:139` matches `KeyCode::Char('p')` modifier-blind |

The last is the correction, and it is the worse of the two: `ctrl+p` does not do nothing, it does *something
else*, which is the same class as the `n`-denies-the-tool ordering bug this repository already has a test for
at `crates/epsilon-tui/src/app/tests.rs:4935`.

Two consequences follow from `search` being absent from `overlay_open()`: `composer_focused`
(`crates/epsilon-tui/src/app.rs:1663`) stays `true`, so the composer still draws its accent prompt and a real
terminal caret (`crates/epsilon-tui/src/ui/composer.rs:111-119`) while every keystroke is swallowed; and the
search panel is the only overlay that ships its own key row inside the panel
(`crates/epsilon-tui/src/app.rs:1781`), so the information exists — just not on the row the repository's own
rules call the place keys are advertised.

**Nothing pins the footer row while a search is open.** `no_key_inside_the_search_view_can_end_the_session`
(`crates/epsilon-tui/src/app/tests.rs:4862`) presses PageDown and the letters but asserts only the return
value; it never renders a frame. This holds today and is unpinned.

### 7.2 A comment at `crates/epsilon-tui/src/app/keys.rs:1134-1135` says the opposite of the code

```
// Model picker captures keys before the permission dialog (it can open while a
// run is active, and must always be operable).
if app.picker_open {
```

The permission dialog is routed first, at `crates/epsilon-tui/src/app/keys.rs:550-552`, and returns; the
picker arm is therefore **unreachable** while an ask is parked. `E-056` documents the real order, and
`approval_preempts_palette_and_picker_without_changing_filter`
(`crates/epsilon-tui/src/app/tests.rs:159`) pins it. The comment is stale prose in the middle of the
decision it misdescribes.

### 7.3 A comment four lines above `ui/card.rs`'s `sanitize` credits it with the other function's job

`crates/epsilon-tui/src/ui/card.rs:194-197`:

```
// [`sanitize`] **strips the newline**, because it strips control characters
// and `\n` is one (this is the same `text_safety` rule that keeps a model's
// output from steering the screen).
```

Every clause is false of the function it names. `ui::card::sanitize` (`crates/epsilon-tui/src/ui/card.rs:781-783`)
turns tabs into three spaces and drops a lone CR; it does not touch `\n` and never calls `text_safety`. The
security stripper is `Card::sanitize` (`crates/epsilon-tui/src/state.rs:215`), and `display_safe` **deliberately
preserves** `\t`, `\n` and `\r` (`crates/epsilon-runtime/src/text_safety.rs:63`). The paragraph documents the
historical defect accurately and attributes its cause to the wrong function; the newline is now handled one
line later and correctly at `crates/epsilon-tui/src/ui/card.rs:208-210` (`text.lines().map(...)`), which is the
M312 fix the paragraph is attached to. See `E-017`, which has no test.

### 7.4 The credential store contradicts `AGENTS.md` §6 — reported, not reconciled

`AGENTS.md` §6 says *"Never write a credential to disk in plaintext. Secrets belong in the OS credential
store."* The implementation writes `~/.config/epsilon/auth.json` as `{"<provider>": {"type": "api", "key": "…"}}`
(`crates/epsilon-llm/src/auth.rs:6`, `:15`), and calls that discipline deliberate ("pi `FileAuthStorageBackend`
parity, M35"). It is owner-read/write on unix (`auth_file_is_owner_only_on_unix`,
`crates/epsilon-llm/src/auth.rs:1135`). Per `AGENTS.md` §14 this is reported rather than reconciled: the fix is
a maintainer decision, and no agent should change a credential store as a side effect of an audit.

### 7.5 `--demo` never opens the TUI, and one module doc says the default is the demo — CONFIRMED

`crates/epsilon-cli/src/main.rs:463` sets `Mode::Demo`; `:1206` dispatches it to `demo_mode()`, which builds a
four-step faux script and prints plain lines (`render_demo_event`, `crates/epsilon-cli/src/main.rs:1853`) in
`std::env::temp_dir()` (`:1822`). `epsilon-tui` is never referenced in that path.

The default is the TUI — `crates/epsilon-cli/src/main.rs:339`: `let mut mode = Mode::Tui(None); // M14: plain
`epsilon` opens the interactive TUI`. The line that misdescribes it is `crates/epsilon-cli/src/main.rs:4`:
`//! - default             : scripted M5-style demo against the faux provider`, which contradicts the `USAGE`
constant at `:38` and the `HELP` constant at `:62` in the same file.

**A second doc defect found while verifying this:** the doc comment
`/// The classic scripted demo (M5), kept as the zero-argument experience.`
(`crates/epsilon-cli/src/main.rs:1726`) is **not attached to `demo_mode`**. It sits immediately above
`doctor_mode` (`crates/epsilon-cli/src/main.rs:1732`), glued to that function's own doc line, so
`doctor_mode`'s rustdoc opens by calling itself the demo. The claim is wrong twice over: the zero-argument
experience is the TUI.

### 7.6 `crates/epsilon-cli/tests/terminal.rs` does **not** silently skip — the hypothesis is refuted on both counts

`binary()` (`crates/epsilon-cli/tests/terminal.rs:57`) tries `EPSILON_BIN` first (`:58`), then falls back to
`option_env!("CARGO_BIN_EXE_epsilon")` (`:66`), which Cargo sets automatically for an integration test in a
package whose binary is `epsilon`. Under the battery's test leg (`cargo test --workspace`, which never sets
`EPSILON_BIN`) the eight real-pty tests **do run**. Only when neither variable is present does it skip, and
that skip is loud — `eprintln!("SKIPPED: no EPSILON_BIN and no CARGO_BIN_EXE_epsilon…")` at
`crates/epsilon-cli/tests/terminal.rs:70`. There is **no `#[ignore]` anywhere in the file**.

What *is* wrong is the file header, never updated after M179: `crates/epsilon-cli/tests/terminal.rs:22` and
`:25` still describe a pre-M179 gate that the body of the same file documents as fixed at `:44-56`
("`the_screen_reports_what_the_harness_is_doing` sat broken from M151 to M178 for exactly that reason").

**One residual risk, named:** only one of the eight pins the provider.
`a_typed_line_is_submitted_and_the_run_is_visible` adds `--provider faux`
(`crates/epsilon-cli/tests/terminal.rs:536`, load-bearing per its own comment at `:526`). The other seven
launch `epsilon` bare, so on a machine with a stored provider key they would run against a real provider.

### 7.7 A panel title embeds the raw draft, and a long one can overflow its frame

`crates/epsilon-tui/src/app/panels.rs:80` builds the picker title as
`format!("models {} filter: '{}'", app.theme.glyphs.dash, app.input)` — the raw composer buffer, unsanitised
and unbounded — and `crates/epsilon-tui/src/app/panels.rs:582` does the same for the timeline with
`app.tree_filter.label()`. **No row in this inventory covers what a very long title does to the panel's
geometry**, because the frame tests found use short drafts and the source does not clamp these titles. This is
recorded as an open question rather than a row: the claim "it overflows" was not established here, only that
the input is unbounded and unclamped.

### 7.8 Still open from the survey, unchanged by this pass

- **F2/F3, the dead bindings, are closed by M313.** See §8. The survey's premise no longer holds.
- **`app.transcript.top` / `app.transcript.bottom` have no test at all** (`E-104`).
- **`Ctrl+G` (`app.live.refresh`) is never pressed by any test** (`E-045`).
- The `@`-file popup, the masked API-key prompt, and `/help`, `/hotkeys`, `/settings` are asserted on state
  only; no frame test reads what they put on screen (`E-038`, `E-039`, `E-048`–`E-050`, `E-059`).

---

## 8. M313 — what changed after this survey

Three fixes landed after the survey this file was built from. **Each was re-opened in the source before being
written down, and all three matched.** Where the survey's row would have said something different, the
difference is recorded.

### 8.1 The `unreachable!()` arm in the model picker is deleted

**Verified.** The arm no longer exists. The picker block's first match now ends at
`crates/epsilon-tui/src/app/keys.rs:1200` with `_ => {}`, and the rationale is the comment at
`crates/epsilon-tui/src/app/keys.rs:1186-1199` — *"A guard in front of the code it guards is not a guard. The
correct way to say 'these keys are handled later' is to not match them here, so the second match is the only
reader and there is one answer per chord."*

The three chords now reach their real handlers: `Ctrl+F` to `toggle_favorite`
(`crates/epsilon-tui/src/app/keys.rs:1301`), and `Ctrl+Up` / `Ctrl+Down` to the navigation match below.
`Ctrl+F` is advertised in `/help` and in the picker's own hint row, and the handler it was protecting was
unreachable code.

Pinned by `ctrl_f_in_the_model_picker_toggles_a_favorite_instead_of_panicking` —
`crates/epsilon-tui/src/app/tests.rs:1778`. The test descends into a provider (so stage 2 is what the key acts
on), presses `Ctrl+F`, and asserts the favourite handler reported either way; then presses `Ctrl+Up` and
`Ctrl+Down` and asserts the picker survives. Carried in this file as `E-044`.

The reason 2,061 green tests never saw it is recorded at `crates/epsilon-tui/src/app/tests.rs:1774-1776`: the
one `Ctrl+F` test (`ctrl_b_ctrl_f_move_cursor_by_char`) runs with the picker **closed**.

### 8.2 `app.interrupt` and `tui.input.submit` are read from the binding table

**Verified, and this changes a survey finding.** Both arms are now table-driven:

- `crates/epsilon-tui/src/app/keys.rs:1346` — `_ if app.keybindings.matches("app.interrupt", &key) => {`,
  with the rationale at `:1333-1345`. Behaviour under the default table is unchanged: `specs_for` yields
  `escape` with no modifiers and `key_matches` compares modifiers for equality, so bare Esc matches and
  `Shift+Esc` does not.
- `crates/epsilon-tui/src/app/keys.rs:1410` — `_ if app.keybindings.matches("tui.input.submit", &key) => {`,
  with the rationale at `:1399-1409`. The old form was
  `KeyCode::Enter if app.keybindings.matches("tui.input.submit", &key)` — a guard re-deriving the answer the
  pattern in front of it had already given, so the table was read and then not obeyed.

Pinned by `a_rebound_interrupt_key_is_the_key_that_interrupts` — `crates/epsilon-tui/src/app/tests.rs:1823`,
which asserts **both** directions: the rebound chord interrupts, and `Esc` now says it is not bound rather than
quietly working anyway; and `a_rebound_submit_key_is_the_key_that_submits` — `:1875`, which asserts Enter leaves
the draft alone and the rebound chord submits and clears it.

**This closes the survey's F2/F3 and invalidates one of its findings.** The survey recorded that *every id
except `app.interrupt` has a handler*. That was true of the table as it stood and is now false: all 24
`DEFAULTS` ids have a `keybindings.matches(...)` arm (see §3), and the two ids with a handler and **no** default
chord are `app.transcript.halfUp` (`crates/epsilon-tui/src/app/keys.rs:1686`) and `app.transcript.halfDown` (`crates/epsilon-tui/src/app/keys.rs:1691`), which is
deliberate and documented at `crates/epsilon-tui/src/keybindings.rs:93-122`.

### 8.3 The streaming tail is stripped with `display_safe` on the way into `UiState`

**Verified.** `UiState::apply`'s `MessageUpdate` arm assigns both fields through the stripper:
`crates/epsilon-tui/src/state.rs:1191-1193` for `streaming_thinking` and `:1194-1195` for
`streaming_text`. `display_safe` re-exports from `epsilon-runtime` (`crates/epsilon-tui/src/text_safety.rs:13`),
so the strip happens at the boundary, not at the draw.

Pinned by `a_bidi_override_in_streaming_model_output_never_reaches_the_frame` —
`crates/epsilon-tui/src/app/tests.rs:5600`, which asserts on the **stored field** first and on the frame
second.

**The measurement behind it belongs in this file**, because it is the kind of finding that gets re-invented:
**the frame was already clean before the fix.** ratatui discards a zero-width format character on its way into
a cell, so a `U+202E` in the raw tail never reached the terminal anyway. What was missing was the guarantee —
`AGENTS.md` §6 strips untrusted text *before* it reaches a screen, and "a layout crate happened to discard it"
is a property of a third-party crate that a dependency bump can take away silently. The comment at
`crates/epsilon-tui/src/state.rs:1180-1190` records this measurement in the source, which is where it belongs.

Carried in this file as `E-022`.

---

## 9. Citation check

Every `path:line` in this file was machine-validated against the working tree: the file must exist and the
line number must be within it. The check proves each citation **points at a real line**, not that the line says
what the row claims — rows were written from opening the lines, and the citations below the row record the
verbatim source text so a reader can spot-check without rebuilding the argument.

```
citations parsed: 697   resolvable: 697   distinct files cited: 66   problems: 0
row headings: 110       ids E-001..E-110   gaps: 0   duplicates: 0
rows with `Test: **none found**`: 8
```

That check proves each citation **points at a real line**, not that the line says what the row claims — rows
were written from opening the lines, and the row text records the behaviour that line was read for, so a reader
can spot-check without rebuilding the argument. Every test name was read at its `fn` declaration line before
being written down.

**Two honest limits, stated rather than papered over:**

1. **No test was run.** The integrator owns the build; this audit did not invoke cargo. The `frame` /
   `state only` labels are readings of each test's source, not reports of green runs.
2. **`golden.rs` uses no golden files** (§1), so nothing here compares a rendered frame against a stored
   expectation. A row marked `frame` means the test drives the production draw path and reads the screen back.
   The only byte-level frame corpus in the repository is
   `audit/evidence/tui-frames-2026-10-04-m312/`, which is M312's tree (§6).

