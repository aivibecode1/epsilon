# AI harness chrome

This is the product-specific map for an agent / LLM runner TUI. Stable spatial model first. Fancy widgets second.

## Contents

Dominant loop · Canonical layout · Modes · Send binding · Streaming UX ·
Tool approval · Diffs · Rendering assistant markdown · Sidebar · Composer ·
Clipboard · Command palette · Header contents · Footer contents · Session
persistence · Empty, error, offline · Accessibility / automation

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
| Confirm | `y` / `Enter` accept, `n` / `Esc` dismiss. Reserved for the heavier, destructive-action prompt — see Tool approval below for the lighter inline case. |

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
- Tool calls are **cards**, not raw JSON dumps. Title line = name + status + duration. Body = one-line summary, expandable with `o` / `Enter` when the card is selected. See Tool approval and Diffs below for the pending/approved states and how an edit renders.

Pin behavior:

- If the viewport is at the bottom, follow new tokens
- If the user scrolled up (`k`, `PgUp`, mouse wheel), freeze until they hit `G` or a "jump to live" hint

Stop:

- `Esc` in Normal during a run cancels the worker
- Footer swaps to `esc stop` while `status == Running`

## Tool approval

Most of what makes an AI harness feel safe to leave running is not the
transcript — it is what happens in the gap between "the model wants to do X"
and "X happens."

Three tiers. Give each a visibly different card so the reader doesn't have to
read the body to know the stakes:

| Tier | Example | Surface |
|---|---|---|
| Auto | read-only tool inside the project dir (`read_file`, `grep`) | runs, card shows the result, no prompt |
| Ask | writes a file, runs a build, calls the network | card renders pending; footer shows `y approve  n deny  a always` |
| Confirm | recursive delete, force-push, anything outside the project directory | `Mode::Confirm` modal, not the inline card — see `references/layout.md` |

Pending-card anatomy:

```text
┌ tool write_file  pending ───────────────────────────────┐
│ src/ui/theme.rs                              +12 -3     │
│ y approve   n deny   o preview diff                      │
└────────────────────────────────────────────────────────┘
```

Rules:

- The pending state uses `theme.surface_pending()` — or, on a 16-colour
  terminal with no surfaces, a gutter glyph (see `assets/theme.rs::has_surfaces`).
  Approved becomes `surface_ok()`, denied or failed becomes `surface_err()`.
  The card's own colour and title word carry the status; do not require
  reading the body to learn it.
- `a` ("always") remembers the decision for that tool name for the rest of the
  session only. It is never the default and it never persists to config — a
  session that reopens with every write auto-approved is a session the user
  did not agree to.
- One pending card blocks that tool call, not the UI. The transcript keeps
  scrolling, other queued tool calls from the same turn keep running if they
  do not depend on this one's result, and the composer stays usable to queue
  the next message.
- A denied call is a normal transcript entry, not an error:
  `tool write_file  denied` in `danger()`, one line, no stack trace. The model
  sees the denial and can propose something else.
- `n` on a selected pending card means *deny*, not *new session* — the same
  collision the Normal-mode keymap already has with `n`. Check "is a pending
  card selected" before falling through to the rest of the Normal-mode keymap,
  the same way `o` already means *open the selected card* rather than
  something global. Don't rebind the global `n`; scope the pending card's keys
  ahead of it.

## Diffs

A `write_file` or `edit` tool call is more useful as a diff than as a
before/after dump. Keep it inside the card, capped, not a separate pane:

- Unified format: one `+`/`-`/` ` gutter column, then the line. Colour the
  gutter and the line together with `success()` / `danger()`, not the gutter
  alone — a colourblind reader needs the `+`/`-` character to survive without
  colour.
- Cap the card at roughly 12 visible lines with `… 43 more lines (o to open)`.
  `o` on the selected card opens the full diff in the right inspector (width
  ≥ 140) or a full-screen overlay below that width — never inline-expand a
  40-line diff into the transcript, it breaks virtualization.
- A rename or a binary file shows one line
  (`renamed src/a.rs → src/b.rs`, `binary file, 4.1kb`), never a diff body.

See `references/widgets.md` for the rendering recipe.

## Rendering assistant markdown

The model's own turns are usually markdown-flavoured: inline code, fenced
blocks, bullet lists, the occasional heading. A full CommonMark renderer is
overkill for what a chat model actually emits — hand-roll the subset:

| Markdown | Render as |
|---|---|
| `` `code` `` | `theme.code()`, backticks dropped |
| ` ```lang ` fence | left rule `│` in `code_block()`, block dedented, language label dim on the opening line |
| `**bold**` | `Emphasis::Strong` on the span, asterisks dropped |
| `- item` / `1. item` | hanging indent, bullet or number kept, nested lists indent by 2 |
| `# heading` / `## heading` | `title()` style, `#` characters dropped, not a horizontal rule |
| everything else | plain text, unrecognised syntax printed as-is rather than swallowed |

Parse once per completed message, not per token — re-parsing markdown on every
streamed delta is the same mistake as rebuilding the whole transcript `Text`
per token, and for the same reason (see `references/architecture.md`). While a
message is still streaming, render its raw tail as plain text and only run the
markdown pass once `StreamDone` lands; a code fence that hasn't been closed
yet has no correct rendering anyway.

Do not syntax-highlight fenced code by default. A `syntect` / `tree-sitter`
highlighter is a legitimate opt-in "polish" pass, but it is a new dependency
and a new failure mode (unknown language string, highlighting a 2,000-line
paste) for a feature the dim `code_block()` style already serves adequately.

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

Keep composer state as `String` + `usize` cursor, or adopt `tui-textarea` if
multiline editing quality matters. Do not invent a half-broken editor. See
`references/architecture.md` for enabling multi-line paste as one insert
instead of a flood of `Enter` keys, and `references/widgets.md` for
grapheme-aware cursor movement.

## Clipboard

`arboard` / `copypasta` (see `references/widgets.md`) reach the OS clipboard,
which does not exist when the harness is running over SSH or inside a remote
`tmux` — a "yank" that only calls them fails silently there, and nothing in
the UI explains why. Emit an OSC 52 sequence alongside the crate call:

```rust
fn osc52_copy(text: &str) {
    use base64::{engine::general_purpose::STANDARD, Engine};
    print!("\x1b]52;c;{}\x07", STANDARD.encode(text));
}
```

Terminals with OSC 52 support (iTerm2, kitty, WezTerm, Windows Terminal, and
most others by 2026) forward it to the real host clipboard even across an SSH
hop. It needs no dependency beyond a base64 encoder most trees already have,
and it is the one of the two paths that still works remotely.

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
tool    y approve  n deny  a always  o diff              transcript
```

Do not list keys that do nothing in the current mode.

## Session persistence

See `references/architecture.md` (Crash recovery) for the write path — a JSON
Lines log flushed after every completed turn. On the read path, opening a
session from the sidebar replays its log file into the transcript in one pass
before the first `terminal.draw`, not as a simulated re-stream: a resumed
session should render at the same speed regardless of how long the original
conversation took.

## Empty, error, offline

| State | Surface |
|---|---|
| No sessions | sidebar shows `n new session`; transcript shows a 3-line getting-started blurb, not a blank void |
| No messages | composer focused on first launch |
| Stream error | red status word + one-line error under the last assistant header, composer re-enabled |
| Disconnected | header `offline`, send rejected with a footer flash |
| Pending approval, unanswered | tool card stays pending and blocks only that tool; transcript, other panes, and the composer (queuing) all stay live |
| Resumed session | sidebar marks it as resumed; transcript replays the saved log in one pass before the first draw |
| Too small | centered message, ignore other widgets |

## Accessibility / automation

- `--no-tui` prints turns to stdout for piping and screen readers
- `--ascii` disables unicode chrome
- Mouse wheel scrolls the focused pane; click may move focus; nothing requires a click
