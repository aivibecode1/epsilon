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
    Resize,
    Tick,
    StreamDelta { session: u64, text: String },
    StreamDone { session: u64 },
    ToolEvent { id: u64, body: String },
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

## Streaming without jank

- Append deltas into the last assistant buffer (`String` or chunk list). Do not rebuild the entire history `Text` per token.
- Coalesce UI updates — flush the transcript at most every 16–33 ms while streaming, not per token.
- Keep a `viewport_pin: PinBottom | Follow | Manual` flag. Auto-scroll only when the user is already at the bottom.
- Cancel in-flight work on `Esc` / `Ctrl-C` (while streaming) by dropping the worker handle or sending a cancellation token. The UI stays interactive.

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

## Tests

Unit-test `update` with synthetic `Action`s. No terminal required.

Optional: render into a `ratatui::backend::TestBackend` and assert buffer cells for chrome, focus border, and empty/too-small states.

Do not start with PTY integration tests. Add one smoke test only when lifecycle bugs have already shipped.
