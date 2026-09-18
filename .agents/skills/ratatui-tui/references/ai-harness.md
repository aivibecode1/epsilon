# AI harness chrome

This is the product-specific map for an agent / LLM runner TUI. Stable spatial model first. Fancy widgets second.

## Dominant loop

1. Pick or create a session (sidebar / picker)
2. Type in the composer (`Insert` mode)
3. Send (`Enter` in insert with a modifier policy below)
4. Watch the transcript stream
5. Inspect tool cards if the agent calls tools
6. Edit and send again, or `Esc` to stop

Design every key and pane around that loop. Settings, logs, and model pickers are overlays, not new home screens.

## Canonical layout

```
┌──────────────────────────────────────────────────────────┐
│ harness    glm-4 · 32k/128k · 4.2k tok · $0.02    ● run │ 1
├────────────┬─────────────────────────────────────────────┤
│ sessions   │ you                                     2m  │
│  ▎refactor │   pin the layout on resize                  │
│   docs     │                                             │
│            │ assistant                          stream   │
│ tools      │   working on src/ui/layout.rs               │
│  read_file │   ┌ tool read_file  ok  81ms ─────────────┐ │
│  cargo     │   │ src/ui/layout.rs                       │ │
│            │   └────────────────────────────────────────┘ │
│            ├─────────────────────────────────────────────┤
│            │ › ask a follow-up                           │ 3–8
├────────────┴─────────────────────────────────────────────┤
│ insert  enter send  esc stop  ctrl-n session  ? help     │ 1
└──────────────────────────────────────────────────────────┘
```

Do not add a right inspector until width ≥ 140. When present it shows the selected tool payload or file preview — never a third copy of the transcript.

## Modes

```rust
enum Mode { Normal, Insert, Palette, Picker, Confirm }
enum Focus { Sidebar, Transcript, Composer }
```

| Mode | Keys |
|---|---|
| Normal | `hjkl` / arrows move focus and lists. `i` or `Enter` focuses composer. `:` or `Ctrl-k` opens palette. `/` searches transcript. `n` new session. `q` quit confirm. `?` help. |
| Insert | Text input. `Esc` back to Normal (does not quit). Send binding below. `Ctrl-c` while streaming = cancel, otherwise clear composer. |
| Palette | Fuzzy command list. Type to filter. `Enter` run. `Esc` close. |
| Picker | Session / model / file fuzzy finder. Same keys as palette. |
| Confirm | `y` / `Enter` accept, `n` / `Esc` dismiss. |

Never handle `q` as quit while `Mode::Insert`.

## Send binding

Pick one policy and print it in the footer:

- **Chat default:** `Enter` sends, `Shift-Enter` or `Ctrl-j` newline
- **Editor default:** `Ctrl-Enter` or `Alt-Enter` sends, `Enter` newline

Do not silently switch policies. Store it on config.

## Streaming UX

Header spinner uses one braille frame (`⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`) on tick. Pair with the word `run` / `stream` / `idle` so color is not the only signal.

Transcript:

- User messages are complete blocks
- Assistant message grows at the tail
- A thin caret or block on the last line while streaming
- Tool calls are **cards**, not raw JSON dumps. Title line = name + status + duration. Body = one-line summary, expandable with `o` / `Enter` when the card is selected

Pin behavior:

- If the viewport is at the bottom, follow new tokens
- If the user scrolled up (`k`, `PgUp`, mouse wheel), freeze until they hit `G` or a "jump to live" hint

Stop:

- `Esc` in Normal during a run cancels the worker
- Footer swaps to `esc stop` while `status == Running`

## Sidebar

Two stacked lists, not two boxed panels:

1. Sessions (most recent first). Current session marked with `▎` + bold
2. Activity / last tools (optional, hide when height < 24)

Width 24–32. Titles are plain dim text on the first row of each stack, not inner `Block`s.

Hide the sidebar under 80 columns and expose it through `Ctrl-n` picker instead.

## Composer

- Placeholder dim text when empty (`ask anything`)
- Grows from 3 rows to max 8 (or 30% of height)
- Show a right-aligned `⏎ send` hint only when focused and non-empty
- Disabled look (dim border) while the model is running if the product disallows queueing; otherwise accept queued messages and show a count

Keep composer state as `String` + `usize` cursor, or adopt `tui-textarea` if multiline editing quality matters. Do not invent a half-broken editor.

## Command palette

Center overlay, width `min(72, area.width - 4)`, height `min(16, area.height - 4)`.

Rows are `command  <keyhint>` with the hint dim and right-aligned. Filter is prefix + fuzzy. Empty filter shows the 8–10 most used commands, not the entire catalog.

## Header contents

Left: app name (bold accent).

Right, truncated from the left as width shrinks:

1. status word + spinner
2. model id
3. context used / window
4. session tokens
5. estimated cost

At 80 columns keep name + status + model. Drop cost first, then tokens, then context.

## Footer contents

Always:

```text
{mode}  {keys for this mode}                    {focus}
```

Examples:

```text
normal  i insert  : palette  n session  q quit          sidebar
insert  enter send  esc normal  ctrl-c cancel           composer
run     esc stop  j/k scroll  G live                    transcript
```

Do not list keys that do nothing in the current mode.

## Empty, error, offline

| State | Surface |
|---|---|
| No sessions | sidebar shows `n new session`; transcript shows a 3-line getting-started blurb, not a blank void |
| No messages | composer focused on first launch |
| Stream error | red status word + one-line error under the last assistant header, composer re-enabled |
| Disconnected | header `offline`, send rejected with a footer flash |
| Too small | centered message, ignore other widgets |

## Accessibility / automation

- `--no-tui` prints turns to stdout for piping and screen readers
- `--ascii` disables unicode chrome
- Mouse wheel scrolls the focused pane; click may move focus; nothing requires a click
