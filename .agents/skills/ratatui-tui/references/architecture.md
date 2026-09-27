# Architecture

## App skeleton

Install color-eyre first so Ratatui can wrap the panic hook and restore the terminal.

```rust
use color_eyre::Result;
use ratatui::DefaultTerminal;

fn main() -> Result<()> {
    color_eyre::install()?;
    ratatui::run(|terminal| App::new().run(terminal))
}

struct App {
    should_quit: bool,
    mode: Mode,
    focus: Focus,
    dirty: bool,
}

impl App {
    fn run(mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.should_quit {
            if self.dirty {
                terminal.draw(|frame| self.draw(frame))?;
                self.dirty = false;
            }
            self.handle_event()?;
        }
        Ok(())
    }
}
```

If the project cannot use `ratatui::run`, call `ratatui::init()` / `ratatui::restore()` and wrap the existing panic hook so restore still runs.

## Elm / MVU split

Keep three types separate:

```rust
enum Mode { Normal, Insert, Palette, Picker, Confirm }
enum Focus { Sidebar, Transcript, Composer, Overlay }
enum Action {
    Quit,
    Key(crossterm::event::KeyEvent),
    Paste(String),
    Resize,
    Tick,
    StreamDelta { session: u64, text: String },
    StreamDone { session: u64 },
    ToolEvent { id: u64, body: String },
    ToolApproval { id: u64, decision: Approval },
    Error(String),
}

fn update(app: &mut App, action: Action) -> Option<Action> { ... }
fn draw(app: &App, frame: &mut ratatui::Frame) { ... }
```

`update` owns all mutation. `draw` only reads. Services (LLM, tools, fs) emit `Action`s; they never touch Ratatui types.

## Sync event loop

Fine for apps with no network:

```rust
use crossterm::event::{self, Event, KeyEventKind};
use std::time::Duration;

match event::poll(Duration::from_millis(125))? {
    false => { /* optional tick */ }
    true => match event::read()? {
        Event::Key(key) if key.kind == KeyEventKind::Press => { /* handle */ }
        Event::Resize(_, _) => app.dirty = true,
        Event::Mouse(m) => { /* optional */ }
        _ => {}
    },
}
```

Always filter `KeyEventKind::Press`. On Windows, release events will otherwise double-toggle.

## Async event loop (AI harness default)

LLM streams and tool jobs require Tokio. Pattern:

1. Bound an `mpsc` channel (capacity 256–1024). Unbounded queues hide backpressure and blow memory during fast token streams.
2. Spawn one task that forwards `crossterm::EventStream`.
3. Spawn workers that send `Action::StreamDelta` / `ToolEvent`.
4. `tokio::select!` in the UI task between `rx.recv()`, a slow tick (250–500 ms), and shutdown.

```rust
#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    ratatui::run(|terminal| App::new().run_async(terminal))
}
```

Do not call blocking `event::read()` on the runtime thread that also drives draws.

Draw only when `dirty` or on tick while a stream is active (cursor blink / spinner). Idle sessions should sit near 0% CPU.

## Bracketed paste

Enable it before the loop starts, disable it on restore:

```rust
use crossterm::event::{EnableBracketedPaste, DisableBracketedPaste};
use crossterm::execute;
use std::io::stdout;

execute!(stdout(), EnableBracketedPaste)?;
// ... run the app ...
execute!(stdout(), DisableBracketedPaste)?;
```

Without it, a pasted multi-line prompt arrives as a flood of `Event::Key` — one
`KeyCode::Enter` per newline. If `Enter` sends, that flood sends a half-typed
message after the first line and inserts newlines into whatever is left in the
composer. With it, the whole paste arrives as one `Event::Paste(String)`;
insert it into the composer buffer directly and never route it through the key
handler:

```rust
Event::Paste(text) => app.composer.insert_str(&text),
```

`ratatui::run` and `ratatui::init()` do not enable this for you — it is a
`crossterm` opt-in, separate from raw mode and the alternate screen, and it
needs the matching `Disable` call on the way out or the next thing the user
runs in that terminal inherits it.

## Streaming without jank

- Append deltas into the last assistant buffer (`String` or chunk list). Do not rebuild the entire history `Text` per token.
- Coalesce UI updates — flush the transcript at most every 16–33 ms while streaming, not per token.
- Keep a `viewport_pin: PinBottom | Follow | Manual` flag. Auto-scroll only when the user is already at the bottom.
- Cancel in-flight work on `Esc` / `Ctrl-C` (while streaming) by dropping the worker handle or sending a cancellation token. The UI stays interactive.

## Cancellation

A bound `mpsc` per job works for one stream. Once two or three things can be in
flight at once — an LLM stream, a tool call, a background health check —
cancelling "the current run" by dropping one receiver stops covering the case.
Fan a `tokio_util::sync::CancellationToken` out instead:

```rust
use tokio_util::sync::CancellationToken;

let run_token = CancellationToken::new();

let llm_token = run_token.child_token();
tokio::spawn(stream_completion(llm_token, tx.clone()));

let tool_token = run_token.child_token();
tokio::spawn(run_tool(tool_token, tx.clone()));

// Esc in Normal during a run:
run_token.cancel(); // every child_token().is_cancelled() flips at once
```

Each worker checks `token.is_cancelled()` at its await points, or races it with
`tokio::select!`, and exits without needing to know what else was spawned under
the same run. Drop the token and build a fresh one per run rather than reusing
it — a cancelled `CancellationToken` stays cancelled.

## State ownership

| Lives on App | Lives on the widget for one frame |
|---|---|
| `ListState`, `TableState`, `ScrollbarState` | `List`, `Table`, `Paragraph` |
| mode, focus, composer text + cursor | `Block` chrome |
| session list, messages, tool logs | highlight styles from theme |
| overlay stack | `Clear` + centered rect |

Implement `StatefulWidget` when a region has a stable contract. Use a free `fn render_header(...)` until that boundary exists.

## Crossterm version skew

Multiple crossterm major versions in one tree break the event queue. After adding crates:

```text
cargo tree -p crossterm
```

There must be one version. Ratatui 0.30 exposes feature flags such as `crossterm_0_29` if a dependency pins an older line.

## Logging

- `tracing` + file appender or `tui-logger` pane.
- Never `println!` / `dbg!` after `ratatui::init()`.
- Panic reports must hit stderr **after** restore, which `ratatui::run` + color-eyre already arrange.

## Crash recovery

Append each completed turn to a JSON Lines file as it finishes, not only when the session ends:

```rust
writeln!(session_log, "{}", serde_json::to_string(&turn)?)?;
session_log.flush()?;
```

A `flush()` after every turn, not every token, is the right cost: worst case on
a crash mid-stream is one incomplete assistant message, not the whole session.
On startup, a session with an existing log file replays it into the transcript
in one pass before the first draw rather than starting blank — a harness that
loses history on a panic teaches the user not to trust it. See
`references/ai-harness.md` for the read-side (resumed session) behavior.

## Tests

Unit-test `update` with synthetic `Action`s. No terminal required.

Render into a `ratatui::backend::TestBackend` to assert on the actual buffer
for chrome, focus border, and empty/too-small states — cheaper than it sounds,
and it catches a class of bug a pure `update` test cannot (a border style that
never got applied, a pane rendered one column off):

```rust
use ratatui::{backend::TestBackend, Terminal};

#[test]
fn focused_pane_gets_the_accent_border() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let app = App::new_for_test();

    terminal.draw(|f| draw(&app, f)).unwrap();

    let buf = terminal.backend().buffer();
    let top_left = buf.get(0, 0);
    assert_eq!(top_left.symbol(), "╭"); // rounded border, focused
}
```

Do not start with PTY integration tests. Add one smoke test only when lifecycle bugs have already shipped.
