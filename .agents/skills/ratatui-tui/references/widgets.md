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

## Composer recipe

Short term: `Paragraph` + manual cursor, insert chars in `Mode::Insert`.

When editing quality matters:

- `tui-textarea` for multiline (verify crate support for ratatui 0.30 before adding)
- `tui-input` for single-line popups (palette filter, rename session)

Show the caret by styling the cell under `cursor_x/y` with `REVERSED` when `Focus::Composer`.

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
| `arboard` / `copypasta` | clipboard yank | optional |

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
