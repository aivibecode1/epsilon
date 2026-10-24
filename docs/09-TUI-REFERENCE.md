# Terminal IDE reference

This is the screen-level guide to `epsilon` — what is drawn, what takes the
keyboard, and how the pieces fit. It is written for a reader who is using the
terminal IDE, not changing it. The code lives in `crates/epsilon-tui`; the design
rules are in [`../AGENTS.md`](../AGENTS.md) §8.

## The frame

Epsilon draws one full-screen layout. It is deliberately quiet: a header, the
conversation, a boxed composer, and a footer, with the project explorer docked on
the right.

```text
idle                    model · context · cost                 Files · 5  f6
▎ ❯ explain the retry path
▪ assistant
  The retry lives in …
  ╭─ edit src/retry.rs ───────────────────── +12 −3 ─╮
  │ …                                                │
  ╰──────────────────────────────────────────────────╯
╭─ prompt ────────────────────────────────────────────╮
│ ❯ ask, or type / for commands                       │
╰─────────────────────────────────────────────────────╯
f6 files · / help · esc cancel
```

The exact glyphs depend on the repertoire (`EPSILON_ASCII=1` forces the ASCII
set). **Controls get boxes; user and assistant messages get their own cards.**
The composer is a box because the user's hands are on it; overlays are boxes
because they take the keyboard; each user or assistant message is a four-sided
card with a title:

```text
╭─ ▎ you ──────────────────────────────────────╮
│ run the tests                                │
╰──────────────────────────────────────────────╯
╭─ ▪ assistant ────────────────────────────────╮
│ The retry lives in src/retry.rs.             │
╰──────────────────────────────────────────────╯
```

There is no blank row between cards: the bottom border of one and the top border
of the next are the separation. The card costs two chrome rows and two rail
columns; its body wraps to the space that remains. Tool cards keep their title
and body rail, and notices and reasoning stay ordinary rows. The live streaming
answer is already a card while it streams, so it does not change shape when it
commits. Colour reinforces the title and border; the text label carries the role
under `NO_COLOR`.

### Regions

| Region | What it is for | Gives up space first |
|---|---|---|
| Header | Mode, model, live context/cost metrics | Metrics disappear first |
| Transcript | The conversation and tool cards | Last — it is the reason the app opened |
| Composer | Your prompt, with slash completion | Shrinks to one row before the transcript |
| Footer | Focus, hints, the keys that work now | Optional half drops on a narrow frame |
| Explorer | Workspace tree, docked right | Becomes a focused panel below the dock floor |

`Regions::split` owns the vertical arithmetic and `Explorer::split` owns the
horizontal split. `CHROME_ROWS` is four: header, gap, composer rule and footer.
The gap and the rule are now the composer box's top and bottom borders; the side
rails cost two columns of text width, not two rows.

### The explorer

The project explorer is **on the right by default** and **does not take the
keyboard** until you focus it. Your first keystroke always reaches the composer,
which is the point of separating *visibility* from *focus*.

- Docked width: 30% of the frame, clamped to 24–38 columns, with a one-column
  gutter. It docks only when the workspace keeps at least 44 columns; below that
  it is drawn as a focused band between header and footer instead of squeezing
  the conversation.
- `F6` shows and focuses it. Pressing `F6` again hides a focused explorer;
  pressing it on a resting (visible, unfocused) explorer focuses it.
- `Esc` returns focus to the composer without closing the panel.
- `Enter` opens a file in the viewer or toggles a directory. `Left` / `Right`
  collapse and expand.
- The tree is **live**: while it is visible it re-walks the workspace every 900 ms
  and refreshes `git status` every 3 s. A file written by the agent, another
  editor or a build appears without pressing `r`; the selected path is preserved
  across the rebuild.
- The status column is right-aligned: `A` added, `M` modified, `?` untracked,
  `D` deleted, `R` renamed, `!` conflicted. A changed directory shows a roll-up
  dot so a folded subtree cannot hide a change. Nesting is drawn with `│` guides
  rather than blank indentation.
- The open/closed state and collapsed directories persist per workspace.

### The file viewer and editor

`Enter` on a tree row opens the viewer over the transcript. It is read-only until
you press `e`; then it is an editor with the same key path as the composer. `Esc`
closes an overlay or leaves the editor. Unsaved edits stay visible after you
leave edit mode and are refused at quit (`Ctrl+Q` says `quit refused`) rather than
silently discarded. The viewer shows a bounded prefix of a large file and marks
it partial; open genuinely large files in an external editor.

## The composer

The composer is the default focus. It is a multi-line editor with grapheme-safe
editing (a backspace never splits a combining sequence or a ZWJ emoji). It is
drawn as a **box**: `╭─ prompt ─╮` above, a left and right rail, and `╰─╯` below.
The border uses the focus colour while the composer owns the keyboard and the
quiet border while an overlay does. The box reuses rows the layout already
reserved, so it costs no conversation space.

| Input | Action |
|---|---|
| `Enter` | Send the prompt |
| `Alt+Enter` | Queue a follow-up; it runs after the current turn instead of steering it |
| `Esc` | Cancel the run, close an overlay, or return focus to the composer |
| `/` at the start | Open the slash menu |
| `@` | Insert a workspace file reference |
| `Ctrl+C` | Clear the draft (not the session) |
| `Ctrl+P` | Open the command palette |
| `Ctrl+Q` | Quit, refused while a run is in flight or an editor has unsaved work |

The slash menu and palette are the **composer's width**, so a description is not
clipped while the frame has empty space. Rows are columns: command, description,
and a right-aligned key hint where one exists.

## Overlays

Overlays are panels over the transcript, not new screens. The common keys are:

| Key | Action |
|---|---|
| `Enter` | Choose / apply |
| `Esc` | Close without applying |
| `Tab` / `Shift+Tab` | Move within a row or switch scope, depending on the panel |
| `/` | Search the overlay's list, where offered |
| `Space` | Toggle a checklist row (models, scoped models) |
| `Ctrl+S` | Save staged checklist changes (scoped models) |

The command palette (`Ctrl+P`) lists every command including the `Files` row,
which is the `F6` tree action. `/tree` is the **session** tree, not the file tree.

### Skills and subagent browser

`/skills` and bare `/subagent` open a **read-only browser** rather than printing a
list into the conversation. The window shows a few rows around the selection:
state, name and a one-line purpose. Below it, the selected entry carries its full
description, why it is useful, its credit where adapted, its scope (`built-in`,
`project` or `user`) and the exact `/skill` or `/subagent` command that changes it.
`up`/`down`, `PageUp`/`PageDown`, `Home`/`End` move; `Enter` puts the selected
writer command into the prompt (it does not run it); `Esc` closes. The commands
remain the only writer, so the browser cannot drift from the installed state.

The panel is centred over the transcript and clears its band, like the other
overlays. It does not take the composer's row: at 80×24 it is about twenty rows
and the prompt box stays visible underneath, which is how the user can close it
and act on the command the detail row named.

## Keys

The canonical table is in `crates/epsilon-contracts/src/keybind.rs` and is drawn
by `/hotkeys`. The letters below are defaults; every id can be rebound in
`~/.config/epsilon/config.toml` under `[keys]`.

| Key | Action |
|---|---|
| `F6` | Show/focus or hide the explorer |
| `Ctrl+P` | Command palette |
| `Ctrl+M` | Model picker |
| `F2` / `Shift+F2` | Cycle scoped models forward/back |
| `Ctrl+T` | Cycle thinking level |
| `Ctrl+X` | Expand/collapse tool output |
| `Alt+E` | Edit the draft in `$EDITOR` |
| `F4` | Copy the last assistant message |
| `Ctrl+G` | Refresh the live model listing |
| `Ctrl+Up` / `Ctrl+Down` | Previous / next prompt in the transcript |
| `Ctrl+Home` / `Ctrl+End` | Top / bottom of the transcript |
| `Ctrl+O` | Cycle the transcript filter |
| `Ctrl+N` | New session |
| `Ctrl+Q` | Quit |

`/hotkeys` is the honest list: it prints every effective binding and every id
that is unbound, rather than the subset with a default. Nothing the UI advertises
is a no-op — if a key is shown by `f6 files` in the footer, F6 acts.

## Themes and accessibility

There are three palettes: `dark` (default), `light`, and `high-contrast`
(`/theme` toggles or names one for the run; the settings panel persists a
choice). `--light` and `EPSILON_THEME=light` select the light palette at launch.
`EPSILON_ASCII=1` replaces the box-drawing and Unicode glyphs with ASCII.

The project targets WCAG 2.2 AA: body text at least 4.5:1, secondary text and
boundaries at least 3:1 in every theme. Colour never carries meaning alone — a
state word (`[off]`, `refused`, `running`) sits beside it. Reduced motion and OS
dark/light are honoured where the terminal exposes them; the narrow-terminal
band and the `MIN_WIDTH` floor exist so a small window says it cannot show the
conversation instead of drawing a mangled frame.

**Known limit:** no screen-reader announcement pass has been recorded. See
[`04-STATUS.md`](04-STATUS.md).

## States every async view must show

The terminal IDE standard is that a view is never a silent blank. Each of these
has a visible state: loading, empty, error, cancelled, and long-running. If a
panel is empty it names the directory or command that would fill it. If a run is
cancelled it says so. If a preview is partial it says so.

## Where the code is

| File | Responsibility |
|---|---|
| `crates/epsilon-tui/src/app.rs` | Application state, submit path, event loop |
| `crates/epsilon-tui/src/app/commands.rs` | Slash-command handlers |
| `crates/epsilon-tui/src/app/keys.rs` | Key dispatch and focus rules |
| `crates/epsilon-tui/src/ui/layout.rs` | Frame and explorer geometry |
| `crates/epsilon-tui/src/ui/transcript.rs` | Transcript rows and wrapping |
| `crates/epsilon-tui/src/ui/card.rs` | Tool cards, diffs, approval rows |
| `crates/epsilon-tui/src/renderers/` | Per-tool card rendering |
| `crates/epsilon-tui/src/ui/theme.rs` | Palette and contrast tokens |

The whole terminal frontend is a reducer over `AgentEvent`s, so a new tool does
not need a new draw loop; it needs a renderer if its card is special.

## Related

- [Command reference](07-COMMANDS.md) — every slash command and argument.
- [User guide](05-USER-GUIDE.md) — task map.
- [Pi parity matrix](parity/pi-tui.md) — what is ported, partial, or deliberately different.
