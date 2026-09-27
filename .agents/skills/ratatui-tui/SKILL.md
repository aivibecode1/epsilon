---
name: ratatui-tui
description: Design and implement clean, professional Ratatui TUIs in Rust — especially AI-harness / coding-agent layouts (sessions pane, transcript, composer, tool cards, approval prompts, diffs). Use this whenever the user is building or reviewing a terminal UI with ratatui 0.30, crossterm, color-eyre, or tokio, or mentions TUI, ratatui, terminal layout, composer, sessions pane, command palette, tool-call approval, diff view, or streaming chat in a terminal — even when they describe the feature without naming ratatui, e.g. "a box that shows the command before it runs".
license: MIT
compatibility: opencode
metadata:
  version: "1.1"
  audience: rust-tui
  stack: ratatui-0.30
---

# Ratatui TUI Design

Build calm, dense, keyboard-first terminal UIs on Ratatui 0.30. Target product is a long-running full-screen session (AI harness, dashboard, IDE-style panes), not a one-shot CLI.

Load only the reference the current task needs:

| Need | File |
|---|---|
| Event loop, MVU, panic restore, async streaming, cancellation, crash recovery | `references/architecture.md` |
| Constraints, nested splits, resize, popups, confirm dialogs | `references/layout.md` |
| Theme tokens, borders, density, contrast, light/dark, clutter audit | `references/visual-system.md` |
| Sessions / transcript / composer / tools / approval / diffs / palette | `references/ai-harness.md` |
| Widget recipes, tool cards, diffs, markdown rendering, third-party crates | `references/widgets.md` |

There is also a **working** theme to copy rather than re-derive: `assets/theme.rs`
(role-based palette, colour-depth detection, `NO_COLOR` fallback, column-accurate
width helpers, and tests). It compiles as-is and encodes four decisions that a
naive token struct gets wrong — read its header comment before adapting it.
It ships one dark palette by design; see `references/visual-system.md` before
adding a light variant rather than trying to detect the terminal's background.

## Target versions

- ratatui `0.30` (current patch 0.30.2, MSRV 1.88, edition 2024)
- crossterm `0.29` with feature `event-stream` when async
- color-eyre `0.6` installed **before** `ratatui::init()`
- tokio `1` for LLM streams, tool jobs, and file work
- `unicode-width` `0.2` always; add `unicode-segmentation` `1` once the composer edits by grapheme rather than just measuring by column
- `tokio-util` `0.7` once more than one in-flight job needs to stop together

Apps depend on `ratatui`. Widget libraries depend on `ratatui-core`.

Prefer official helpers:

```rust
fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    ratatui::run(|terminal| App::new().run(terminal))
}
```

Do not hand-roll raw-mode + alt-screen + panic restore unless the project already owns that path.

## Design workflow

1. Classify the product. An AI harness is a **full-screen session** on the alternate screen.
2. Name the dominant loop. For a harness that is usually type → send → stream → inspect tools → type again.
3. Pick a stable spatial model from `references/ai-harness.md`. Do not invent a new chrome layout on every screen.
4. Sketch four widths before coding — 160+, 120, 80, and 60-column tmux split. Write the fallback, not only the wide layout.
5. Define theme tokens first. Never scatter `Color::Cyan` through widgets.
6. Separate `App` state, `Message`/`Action` update, and `draw`. Rendering must be a pure function of state.
7. Wire keys and focus next. Mouse is optional acceleration, never a required path.
8. Decide the tool-approval policy before the first tool card exists: what runs unattended, what waits for a `y`/`n`, and what needs the heavier confirm modal. Retrofitting this after the transcript renderer is built means redesigning the card, not adding a flag.
9. Stream work off the UI thread. Token chunks, tool output, and HTTP never block `terminal.draw`.
10. Run the two review reflexes below before calling the UI done.

## Non-negotiable contracts

### Lifecycle

- Alternate screen for the session. Restore raw mode, cursor, and buffer on every exit including panic.
- Relayout from `frame.area()` every draw. Do not cache pixel-like coordinates across resize.
- Logs go to a file or a dedicated pane. Never print to stdout while the TUI owns the terminal.
- Suspend/handoff (`Ctrl-Z`, `$EDITOR`) must restore the shell, wait, re-enter, then full redraw.
- Persist the transcript after every completed turn (JSON Lines is enough), not only at exit. A crash should cost at most one in-flight message, not the session. See `references/architecture.md`.

### Rendering

- Immediate mode. UI = `f(state)`. Redraw on input, data, resize, or a cheap tick. No 60 fps busy loop.
- Measure display width with `unicode-width`, never `str::len()`.
- Virtualize transcripts, session lists, and tool logs. Do not allocate a `Paragraph` of the entire chat history every frame.
- Keep panel positions stable. Spatial memory is navigation.

### Meaning

- Semantic tokens only (`text.primary`, `status.error`, `border.focus`). Honor `NO_COLOR`.
- Never encode state with color alone. Pair with text, symbol, or position.
- Every action is keyboard-reachable. Vim keys (`hjkl`, `g`/`G`, `/`, `Esc`) are the default lingua franca outside insert mode.
- Footer shows the keys that work **right now**, not a dump of the full keymap.
- **Detect colour depth once, at startup, and carry it on the theme.** Claiming
  24-bit colour on a console that lacks it produces unreadable output; falling
  back to 16 colours merely looks plainer. `COLORTERM` and `WT_SESSION` are the
  positive signals; the safe default is 16.
- **Pair every glyph vocabulary with an ASCII fallback** (`EPSILON_ASCII=1` or
  similar). A legacy code page renders `╭` as mojibake, and the answer cannot be
  detected reliably — a documented switch beats a wrong guess.
- **`NO_COLOR` still has to convey meaning.** Do not map every role to
  `Style::default()`; give "background material" dim+italic and "the value you
  must catch" bold. A theme that collapses under `NO_COLOR` makes reasoning
  traces indistinguishable from answers.
- **Withhold filled background bands on a 16-colour terminal** rather than
  approximating them. A wrong background collides with whatever theme the user
  already runs; a missing one just looks plainer, and the gutter glyph still
  carries the distinction.

### Safety

- Ask before a tool call mutates the filesystem, spawns a process, or reaches the network. "Always allow" is an explicit per-session opt-in the user reaches for, never the state the harness starts in.
- Route destructive-looking calls (recursive delete, force-push, anything outside the project directory) through the heavier `Mode::Confirm` modal, not the inline pending card. The card is for "did you want this," the modal is for "are you sure."
- An unanswered approval blocks that tool call, never the event loop. Scrolling, other panes, and queuing the next message all stay live while one card waits on `y`/`n`/`a`. See `references/ai-harness.md`.

## Default AI-harness chrome

Use this unless the user specified another map:

```
header     1 row   app · model · ctx · tokens · cost · spinner
body       Fill    sidebar | transcript
composer   3–8     multiline input, grows with content, cap at ~30%
footer     1 row   context-sensitive keys · focus · mode
```

Sidebar (22–32 cols, hide under 80 cols): sessions, then a compact activity/tools list.

Overlays (command palette, session picker, confirm, help) render last, centered, with `Clear` behind them. `Esc` dismisses. Focus never leaks into the overlay and the pane beneath at the same time. A pending tool approval is a state of the tool card in the transcript, not a fourth chrome region — see `references/ai-harness.md`.

Full layout rules live in `references/ai-harness.md`.

## Code shape

```text
src/
  main.rs          color-eyre + ratatui::run
  app.rs           App, Mode, Focus, should_quit
  event.rs         Event / Action enums, crossterm + app messages
  update.rs        fn update(app, action) -> Option<Action>
  ui/
    mod.rs         split + dispatch
    theme.rs       tokens only
    header.rs
    sidebar.rs
    transcript.rs
    composer.rs
    footer.rs
    overlay.rs
  services/        llm client, tools — no ratatui types
```

Update functions return follow-up actions. Widgets borrow state; they do not own I/O.

## Review reflexes (run unprompted)

### Clutter audit

Count, then cut:

- Border nesting deeper than one between terminal edge and content
- Four signals for one state (icon + color + badge + label)
- Markers drawn on every row (they mark nothing)
- Chrome cells vs data cells — header/footer/borders should stay thin

Name the exact widgets to remove. "Make it cleaner" is not a review.

### Floor test

State what happens at 80×24 and 60×20:

- Which pane is dropped first (sidebar)
- What truncates vs wraps (header metadata truncates, composer wraps)
- When the app shows `terminal too small` instead of a broken layout

A three-column design without a single-pane fallback is incomplete.

## Traps that cost real time

Each of these produced a bug that survived review, and each is cheap to avoid if
you know it is there.

- **Character counts are not columns.** `⠹`, `┌` and `界` are each one `char`
  but one, one and two columns. Every width decision — layout, truncation, "does
  this fit" — goes through `unicode-width`. Measured in `chars()` instead, rows
  come out one cell too wide and wrap. See `assets/theme.rs`.
- **`Cell::symbol()` can hold a repeated run.** ratatui coalesces adjacent
  identical cells into one cell whose symbol repeats, so reading a screen buffer
  back and concatenating symbols doubles characters (`100%` → `100%%`). Append
  the whole symbol and step by its display width.
- **`''.chars().all(..)` is vacuously true**, so "count the rows that are only
  dashes" counts every blank row. Guard test predicates with a non-empty check.
- **Trim a status row from its far end.** The half that yields must give up the
  end touching the other half, or the leading fragments survive and the model
  name — the one value that explains every other number — is what disappears.
- **Clamp a one-row bar on the way out.** Two width tables (yours and ratatui's)
  can disagree by a column on rare code points, so measure the finished line once
  more and cut it to the area. That makes "this row must never wrap" structural
  rather than a property of arithmetic that has to stay correct in every branch.
- **Do not put lifecycle markers in the transcript.** `-- run start --` between
  every question and answer turns a conversation into a log file. The header
  already says whether a run is in flight; a row saying so is a third copy.
- **When the UI looks frozen, time a tool call before reading any rendering
  code.** A measured case: every Windows tool call spawned `powershell -Command`
  without `-NoProfile`, loading the user's interactive profile first — 6.2 s for a
  one-word `echo` against 1.1 s without it. From the TUI that is
  indistinguishable from a deadlock, and it looked exactly like a rendering bug
  for a long time. `cargo build` + a stopwatch beats reading the draw loop.
- **Bracketed paste, or a chat message arrives as five.** Enabling `Enter`-to-send
  without enabling bracketed paste means a multi-line paste sends after its first
  line and inserts the rest as newlines into whatever composer is left.
  `crossterm::event::EnableBracketedPaste` turns the whole paste into one
  `Event::Paste(String)`. See `references/architecture.md`.
- **Column width and grapheme boundaries are two different crates.**
  `unicode-width` says how many cells a string draws; it does not say where a
  cursor may land. Backspacing a flag emoji or a combining accent by `char`
  instead of by grapheme cluster (`unicode-segmentation`) mangles the glyph
  instead of removing it. See `references/widgets.md`.
- **The OS clipboard doesn't exist over SSH.** `arboard`/`copypasta` reach a
  local display server. A "yank" that only calls them fails silently the moment
  the harness runs on a remote box, with nothing in the UI to explain why. Emit
  OSC 52 too. See `references/ai-harness.md`.

## Implementation defaults

- `Layout::vertical` / `Layout::horizontal` + `.areas(rect)` + `.spacing(0|1)`
- `Constraint::Length` for chrome, `Fill(1)` for the work surface, `Min` as a safety floor
- `Block::bordered().border_type(BorderType::Rounded)` on focused pane only; unfocused panes use a dim flat separator or no border
- Selection = reversed row or left bar `▎`, not a rainbow of highlight styles
- Filter `KeyEventKind::Press` (Windows repeats key-release)
- `ListState` / `TableState` live on `App`, not inside the widget constructor
- Bounded `tokio::sync::mpsc` channels for token and tool streams
- `crossterm::event::EnableBracketedPaste` at startup, disabled again on restore, once the composer accepts multi-line paste
- One `CancellationToken` per run, cloned into each spawned worker, once more than one job needs to stop together

## Do not

- Hardcode `Color::White` / `Color::Black` (breaks light themes and `NO_COLOR`)
- Wrap every pane in `Borders::ALL` (double frames look amateur)
- Block the event loop on model inference
- Rebuild the full transcript `Text` from scratch when one token arrives — append a span, dirty the tail
- Steal `Ctrl-C` without a quit path that restores the terminal
- Depend on mouse hover for essential status
- Introduce a second TUI framework (Cursive, ratatui-kit, iced) unless the user asked
- Auto-approve a tool call because the last one of the same kind was approved — "always" is a session-scoped opt-in the user chose, not a pattern the harness infers
- Re-parse markdown, or re-run a diff's line cap, on every streamed token instead of once the message completes

## When reviewing existing code

1. Check panic restore and `color-eyre` order.
2. Check `crossterm` appears once in `cargo tree`.
3. Check draw is not inside an unconditional `loop { draw; sleep(16ms) }`.
4. Check focus + mode matrix vs key handler.
5. Check resize and minimum size.
6. Check theme tokens vs raw colors.
7. Check streaming path does not clone the whole history per token.
8. Check a pending tool-call approval can't be skipped by a keypress the terminal had already buffered before the prompt appeared, and that its `y`/`n`/`a` keys don't collide with whatever those keys already do in the surrounding mode.
