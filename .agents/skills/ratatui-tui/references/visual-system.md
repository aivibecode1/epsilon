# Visual system

Professional TUIs look quiet. Color is meaning, not decoration. Chrome is thin. Content is dense.

Copy `assets/theme.rs` into the project and route every style through it.

## Token table

Define tokens by role. Map them to ANSI-16 first, truecolor second.

| Token | Role |
|---|---|
| `bg.base` | default background — usually `Reset` |
| `bg.surface` | sidebar / header strip |
| `bg.overlay` | palette, dialog |
| `text.primary` | body copy |
| `text.muted` | timestamps, hints, unfocused titles |
| `text.invert` | selected row fg when using reversed highlight |
| `accent` | brand + focus border |
| `border` | unfocused hairline |
| `border.focus` | focused pane |
| `status.info` | running, streaming |
| `status.ok` | success, completed tool |
| `status.warn` | retry, truncated context |
| `status.err` | failed tool, panic toast |
| `role.user` | user bubble marker |
| `role.assistant` | assistant marker |
| `role.tool` | tool-call marker |

Honor `NO_COLOR`. When set, every token becomes `Reset` plus `BOLD` / `DIM` / `REVERSED` only.

Do not use `Color::White` or `Color::Black` as "defaults". They fight the user's terminal theme.

## Palette discipline

One accent. The AI-harness default is a single cool accent (cyan or a truecolor `#7aa2f7`) plus semantic red/yellow/green.

Forbidden:

- A different hue per pane
- Rainbow syntax on the transcript unless the user asked for a highlighter
- Emoji as the only status signal (they collapse to tofu on many fonts)

Allowed extras, used sparingly:

- `DIM` for metadata
- `BOLD` for the focused pane title and the selected row
- `REVERSED` for list selection (most reliable across terminals)

## Borders

Default alphabet:

- Rounded focused pane (`╭ ╮ ╰ ╯`)
- No border on unfocused panes
- One-eighth bar `▎` as a selection marker inside lists
- Heavy / double borders only on a blocking confirm dialog

Rules:

- At most one border between the terminal edge and content
- Match border background to pane background so gaps do not leak
- Titles sit in the border gap with spaces — ` Sessions ` — or live on the 1-row header, not both

## Density

An AI harness is a reading surface. Pack the sidebar and header. Pad the transcript slightly.

- Sidebar rows: 1 line each, no blank rows
- Transcript messages: 1-line role header + wrapped body + 1 blank line between messages
- Composer: 1-cell padding inside the block
- Header/footer: exactly 1 row, no box

Truncate metadata (`claude-…`, `12.4k tok`, `$0.021`). Wrap prose.

## Typography

| Treatment | Use |
|---|---|
| bold | focused title, selected item, send affordance |
| dim | timestamps, token counts, unfocused labels |
| reversed | current list row |
| underline | OSC-8 links only |
| italic | optional caption; never the only signal |
| blink | never |

Role headers:

```text
you  14:02
  prompt text

assistant  14:02  streaming
  answer text

tool  read_file  ok  81ms
  src/app.rs
```

Keep the role word stable and short. Do not paint a full-width colored banner per message.

## Selection and focus

Focus must be obvious in monochrome:

1. Focused pane title is bold
2. Focused pane gets the accent border (or a left bar)
3. Selected row is reversed
4. Footer starts with the focused pane name

If you drop color, those four still work.

## Clutter audit method

Walk the buffer and score:

1. Border depth to content (max 1)
2. Signals per state (max 2 — e.g. color + word)
3. Always-on row markers (remove)
4. Chrome cell ratio (header + footer + borders + titles)

Cut in this order: nested `Block`s, duplicate titles, per-row icons, then color.

## ASCII fallback

If `LANG` is not UTF-8 or the user passed `--ascii`:

- `+ - |` instead of box drawing
- `>` instead of `▎` / `▶`
- no shadow

Detect once at startup, store on `Theme`.
