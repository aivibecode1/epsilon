---
name: ratatui-tui
description: Design and implement clean professional Ratatui TUIs in Rust, especially AI harness layouts. Use when building or reviewing terminal UIs with ratatui 0.30, crossterm, color-eyre, tokio, widgets, themes, keyboard focus, async streaming, or when the user mentions TUI, ratatui, terminal layout, composer, sessions pane, or command palette.
license: MIT
compatibility: opencode
metadata:
  version: "1.0"
  audience: rust-tui
  stack: ratatui-0.30
---

# Ratatui TUI Design

Build calm, dense, keyboard-first terminal UIs on Ratatui 0.30. Target product is a long-running full-screen session (AI harness, dashboard, IDE-style panes), not a one-shot CLI.

Load only the reference the current task needs:

| Need | File |
|---|---|
| Event loop, MVU, panic restore, async streaming | `references/architecture.md` |
| Constraints, nested splits, resize, popups | `references/layout.md` |
| Theme tokens, borders, density, clutter audit | `references/visual-system.md` |
| Sessions / transcript / composer / tools / palette | `references/ai-harness.md` |
| Widget recipes and third-party crates | `references/widgets.md` |

## Target versions

- ratatui `0.30` (current patch 0.30.2, MSRV 1.88, edition 2024)
- crossterm `0.29` with feature `event-stream` when async
- color-eyre `0.6` installed **before** `ratatui::init()`
- tokio `1` for LLM streams, tool jobs, and file work

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
8. Stream work off the UI thread. Token chunks, tool output, and HTTP never block `terminal.draw`.
9. Run the two review reflexes below before calling the UI done.

## Non-negotiable contracts

### Lifecycle

- Alternate screen for the session. Restore raw mode, cursor, and buffer on every exit including panic.
- Relayout from `frame.area()` every draw. Do not cache pixel-like coordinates across resize.
- Logs go to a file or a dedicated pane. Never print to stdout while the TUI owns the terminal.
- Suspend/handoff (`Ctrl-Z`, `$EDITOR`) must restore the shell, wait, re-enter, then full redraw.

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

## Default AI-harness chrome

Use this unless the user specified another map:

```
header     1 row   app · model · ctx · tokens · cost · spinner
body       Fill    sidebar | transcript
composer   3–8     multiline input, grows with content, cap at ~30%
footer     1 row   context-sensitive keys · focus · mode
```

Sidebar (22–32 cols, hide under 80 cols): sessions, then a compact activity/tools list.

Overlays (command palette, session picker, confirm, help) render last, centered, with `Clear` behind them. `Esc` dismisses. Focus never leaks into the overlay and the pane beneath at the same time.

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

## Implementation defaults

- `Layout::vertical` / `Layout::horizontal` + `.areas(rect)` + `.spacing(0|1)`
- `Constraint::Length` for chrome, `Fill(1)` for the work surface, `Min` as a safety floor
- `Block::bordered().border_type(BorderType::Rounded)` on focused pane only; unfocused panes use a dim flat separator or no border
- Selection = reversed row or left bar `▎`, not a rainbow of highlight styles
- Filter `KeyEventKind::Press` (Windows repeats key-release)
- `ListState` / `TableState` live on `App`, not inside the widget constructor
- Bounded `tokio::sync::mpsc` channels for token and tool streams

## Do not

- Hardcode `Color::White` / `Color::Black` (breaks light themes and `NO_COLOR`)
- Wrap every pane in `Borders::ALL` (double frames look amateur)
- Block the event loop on model inference
- Rebuild the full transcript `Text` from scratch when one token arrives — append a span, dirty the tail
- Steal `Ctrl-C` without a quit path that restores the terminal
- Depend on mouse hover for essential status
- Introduce a second TUI framework (Cursive, ratatui-kit, iced) unless the user asked

## When reviewing existing code

1. Check panic restore and `color-eyre` order.
2. Check `crossterm` appears once in `cargo tree`.
3. Check draw is not inside an unconditional `loop { draw; sleep(16ms) }`.
4. Check focus + mode matrix vs key handler.
5. Check resize and minimum size.
6. Check theme tokens vs raw colors.
7. Check streaming path does not clone the whole history per token.
