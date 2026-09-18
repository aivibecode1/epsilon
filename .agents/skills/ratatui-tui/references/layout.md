# Layout

Ratatui 0.30 layout is constraint-based (kasuari). Prefer the ergonomic constructors.

## Split once per region per frame

```rust
use ratatui::layout::{Constraint, Flex, Layout, Rect};

let [header, body, composer, footer] = Layout::vertical([
    Constraint::Length(1),
    Constraint::Fill(1),
    Constraint::Length(composer_h), // 3..=8, cap at 30% of height
    Constraint::Length(1),
]).areas(frame.area());

let [sidebar, transcript] = Layout::horizontal([
    Constraint::Length(if wide { 28 } else { 0 }),
    Constraint::Fill(1),
]).areas(body);
```

`.areas::<N>()` panics if `N` does not match the constraint count. That is what you want — a mismatch is a bug.

## Constraint choice

| Constraint | Use |
|---|---|
| `Length(n)` | Chrome you can count — header, footer, status, compact composer |
| `Fill(weight)` | The work surface. Weights share leftover space |
| `Min(n)` | Safety floor so a pane does not collapse to 0 |
| `Max(n)` | Cap a growing pane (composer, log) |
| `Percentage(p)` | Only when the ratio must survive extreme resize |
| `Ratio(a, b)` | Rare. Prefer `Fill` weights |

Never express a header as `Percentage(5)`. It will waste rows on a 60-row terminal and vanish on a 12-row one.

## Spacing and flex

```rust
Layout::horizontal([Constraint::Fill(1), Constraint::Fill(2)])
    .spacing(1)
    .flex(Flex::Start)
    .areas(area);
```

- `spacing(1)` is a gutter. Prefer it over wrapping both panes in full borders.
- `Flex::Start` is the default you want for app chrome.
- `Flex::Center` is for overlays.
- `Flex::SpaceAround` follows flexbox (edge gap = half the middle gap). Old 0.29 behavior is `Flex::SpaceEvenly`.

## Responsive breakpoints

Compute layout from `frame.area()`, not a stored `Rect`.

| Width | Behavior |
|---|---|
| ≥ 140 | sidebar 28–32 + transcript + optional right inspector 36 |
| 100–139 | sidebar 24 + transcript |
| 80–99 | hide sidebar; `Tab` cycles a single stack (sessions / chat / tools) |
| 60–79 | chat + composer + footer only |
| < 60 or height < 12 | full-screen `terminal too small` message, no broken splits |

Height:

- header + footer always 1
- composer 3 at rest, grows with line count, max `min(8, area.height / 3)`
- transcript gets everything else

## Nested splits

Split the outer frame first, then split children. Do not build one giant constraint list for the whole screen.

```rust
let [main, footer] = Layout::vertical([
    Constraint::Fill(1),
    Constraint::Length(1),
]).areas(frame.area());

let [left, right] = Layout::horizontal([
    Constraint::Length(28),
    Constraint::Fill(1),
]).areas(main);

let [sessions, tools] = Layout::vertical([
    Constraint::Fill(3),
    Constraint::Fill(2),
]).areas(left);
```

## Popup / overlay geometry

Render overlays last.

```rust
use ratatui::widgets::Clear;

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 3; // optical center
    Rect { x, y, width, height }
}

frame.render_widget(Clear, popup);
frame.render_widget(dialog, popup);
```

`Clear` erases whatever was drawn underneath so the modal does not composite through the transcript. Vertical third — not exact middle — reads better.

Optional 0.30.1+ drop shadow: `Block::bordered().shadow(Default::default())` on the overlay only.

## Focus chrome without nested frames

Do not wrap the whole app in `Borders::ALL` and then wrap every pane again.

- Unfocused pane: no border, or a single dim `│` gutter from `spacing(1)`
- Focused pane: one rounded border **or** a 1-cell title line with an accent, not both stacked
- Overlay: one rounded border + shadow

## Too-small state

```rust
if area.width < 60 || area.height < 12 {
    frame.render_widget(
        Paragraph::new("terminal too small — need 60×12")
            .centered(),
        area,
    );
    return;
}
```

Honest collapse beats clipped composer text and swallowed key hints.
