# Widgets

## Built-in set (reach for these first)

| Job | Widget |
|---|---|
| Chrome box / title | `Block` |
| Prose, composer preview, empty states | `Paragraph` |
| Sessions, commands, files | `List` + `ListState` |
| Tabular tool logs | `Table` + `TableState` |
| Mode tabs (rare in a harness) | `Tabs` |
| Context meter | `Gauge` |
| Scroll thumb | `Scrollbar` + `ScrollbarState` |
| Erase under a modal | `Clear` |
| Raw drawing | `Canvas` (avoid for app chrome) |

Construct widgets each frame from state. Keep `*State` on `App`.

```rust
frame.render_stateful_widget(list, area, &mut app.session_list_state);
```

## Block recipes

```rust
use ratatui::widgets::{Block, BorderType, Borders, Padding};

let focused = Block::bordered()
    .border_type(BorderType::Rounded)
    .border_style(theme.border_focus)
    .title(" sessions ")
    .padding(Padding::horizontal(1));

let flat = Block::new().padding(Padding::horizontal(1));
```

`Block::bordered()` is `Borders::ALL`. Prefer it only on the focused pane or an overlay.

Title API on 0.30 accepts anything `Into<Line>`. You can still call `.title(" sessions ")`.

## List recipe

```rust
use ratatui::widgets::{List, ListItem, ListState};

let items: Vec<ListItem> = app
    .sessions
    .iter()
    .map(|s| {
        let marker = if s.id == app.current { "▎" } else { " " };
        ListItem::new(format!("{marker}{}", s.title))
    })
    .collect();

let list = List::new(items)
    .highlight_style(theme.selection)
    .highlight_symbol("");
frame.render_stateful_widget(list, area, &mut app.session_list_state);
```

Leave `highlight_symbol` empty when you already draw `▎`. Two markers is clutter.

## Transcript recipe

Do not dump the whole history into one `Paragraph` once the session exceeds a few hundred lines.

Virtualize:

1. Store messages as `Vec<Message>` with cached wrapped line counts
2. Compute which message range intersects the viewport
3. Render only that range into the `Rect`
4. Drive a `ScrollbarState` from `offset` / `total_lines`

On a stream delta, rewrap **only the last message**.

## Tool card recipe

A pending, ok, and err tool card are one widget with three states, not three
widgets — build it from the shared theme surfaces so approving a card is a
state change, not a re-render from a different code path:

```rust
fn tool_card(call: &ToolCall, theme: &Theme, selected: bool) -> Block<'static> {
    let (surface, status_word) = match call.status {
        ToolStatus::Pending => (theme.surface_pending(), "pending"),
        ToolStatus::Ok => (theme.surface_ok(), "ok"),
        ToolStatus::Err => (theme.surface_err(), "failed"),
        ToolStatus::Denied => (theme.surface_err(), "denied"),
    };
    let border = if selected { theme.border_focus() } else { theme.border() };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border)
        .style(surface)
        .title(format!(" tool {}  {status_word} ", call.name))
}
```

On a 16-colour terminal `surface(...)` returns `Style::default()` (see
`assets/theme.rs::has_surfaces`) — the border and the status word in the title
still carry the state, so the card degrades instead of going blank. See
`references/ai-harness.md` for the pending/ask/confirm policy this card
renders and the exact key bindings.

## Diff recipe

Render a unified diff as `Line`s with a two-cell gutter, not as a second
`Paragraph` per hunk:

```rust
fn diff_line(kind: DiffLineKind, text: &str, theme: &Theme) -> Line<'static> {
    let (gutter, style) = match kind {
        DiffLineKind::Add => ("+ ", theme.success()),
        DiffLineKind::Del => ("- ", theme.danger()),
        DiffLineKind::Context => ("  ", theme.dim()),
    };
    Line::from(vec![
        Span::styled(gutter, style),
        Span::styled(text.to_string(), style),
    ])
}
```

Cap the rendered lines at the call site (`references/ai-harness.md`) and keep
that cap check outside this function — a diff widget that decides its own
truncation can't be reused for the full-screen expanded view.

## Markdown-lite recipe

One pass over a completed message, never per streamed token (see
`references/architecture.md`), producing `Vec<Line>` from a small hand-rolled
scanner rather than a CommonMark crate — the token table in
`references/ai-harness.md` is the whole spec:

```rust
fn render_markdown(src: &str, theme: &Theme) -> Vec<Line<'static>> {
    // Walk src line by line; track fence state for ``` blocks; split
    // inline `code` and **bold** with a byte-index scan, not regex — the
    // alternation is small and fixed, and pulling in `regex` for four
    // patterns is a build-time cost the feature doesn't earn.
    todo!("see references/ai-harness.md for the token table")
}
```

## Composer recipe

Short term: `Paragraph` + manual cursor, insert chars in `Mode::Insert`.

When editing quality matters:

- `tui-textarea` for multiline (verify crate support for ratatui 0.30 before adding)
- `tui-input` for single-line popups (palette filter, rename session)

Show the caret by styling the cell under `cursor_x/y` with `REVERSED` when `Focus::Composer`.

- Move and delete by grapheme, not by `char` or byte:
  `UnicodeSegmentation::graphemes(text, true)` from `unicode-segmentation`
  gives the boundaries `unicode-width` doesn't. A flag emoji or a combining
  accent is several `char`s in one grapheme; a `Backspace` that removes one
  `char` instead of one grapheme corrupts the glyph instead of deleting it.
  `unicode-width` still measures how many *columns* the result draws — the two
  crates answer different questions and a composer needs both.
- Enable `crossterm::event::EnableBracketedPaste` (see
  `references/architecture.md`) if the composer should accept a multi-line
  paste as one insert instead of a flood of `Enter` keys.

## Overlay recipe

```rust
use ratatui::widgets::{Block, Clear, List, Paragraph};

frame.render_widget(Clear, popup);
frame.render_widget(
    Block::bordered()
        .border_type(BorderType::Rounded)
        .title(" commands "),
    popup,
);
// render inner list in popup.inner(Margin::new(1, 1))
```

## Gauge / context meter

```rust
use ratatui::widgets::Gauge;

Gauge::default()
    .ratio(used / window)
    .label(format!("{used:.0}k/{window:.0}k"))
    .gauge_style(theme.status_info)
```

Use in the header only if width ≥ 120. Otherwise print `12k/128k` as dim text.

## Third-party crates (opt-in)

Add a crate only when the built-in widget is genuinely insufficient.

| Crate | Use | Check |
|---|---|---|
| `tui-textarea` | real multiline editor | ratatui 0.30 compatibility |
| `tui-input` | single-line fields | same |
| `tui-tree-widget` | file trees | same |
| `ratatui-image` | image previews | needs protocol-capable terminal |
| `tui-logger` | in-app log pane | keep tracing file sink too |
| `unicode-width` | cell width | always |
| `unicode-segmentation` | grapheme-aware cursor movement | composer only |
| `arboard` / `copypasta` | clipboard yank, local terminals | pair with OSC 52 for SSH — see `references/ai-harness.md` |
| `base64` | OSC 52 clipboard fallback | tiny, no async feature needed |
| `tokio-util` | `CancellationToken` for multi-job cancel | see `references/architecture.md` |

Do not pull `cursive`, `ratatui-kit`, or `iced` into an existing immediate-mode app.

## Custom widgets

Implement `Widget` for `&T` when the widget does not mutate itself.

```rust
use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};

struct RoleLine<'a> { role: &'a str, time: &'a str, theme: &'a Theme }

impl Widget for RoleLine<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 { return; }
        // write spans into buf
    }
}
```

Use `StatefulWidget` when render updates a cursor / offset owned by the caller.

Guard every custom `render` against `width == 0 || height == 0`. Resize and hidden sidebars produce empty rects.
