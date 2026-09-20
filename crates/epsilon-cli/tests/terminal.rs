//! The terminal integration test: drive the **built binary** through a real
//! pseudo-console and assert on the screen.
//!
//! # Why this exists next to the in-process golden frames
//!
//! `epsilon-tui`'s golden tests render into a `TestBackend`, which answers "what
//! does a frame look like" and cannot answer "does the binary put that frame on a
//! terminal". Everything outside that boundary is where the reported Windows
//! freeze actually lived: terminal initialisation, raw mode, the alternate
//! screen, the resize event, and the event stream. A test that stops at the draw
//! call cannot fail for any of those reasons, and a harness that never spawns the
//! program cannot notice that the program never starts.
//!
//! So this test spawns the real `epsilon` in a real pty, reads what it writes back
//! through a terminal emulator, and asserts on the resulting screen — the same
//! surface a user looks at.
//!
//! # Running it
//!
//! ```text
//! cargo build -p epsilon-cli
//! EPSILON_BIN=target/debug/epsilon.exe cargo test -p epsilon-cli --test terminal
//! ```
//!
//! It skips — loudly, and without passing silently — when `EPSILON_BIN` is unset.
//! A test binary cannot reliably locate a sibling binary's output path, and
//! guessing one would either fail on a fresh clone or, worse, silently test a
//! stale build. Naming the binary explicitly is the honest version.

use std::io::{Read, Write};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

/// How long to let the TUI come up before reading the screen. Generous: a cold
/// binary on a loaded machine is slow, and a flaky test is worse than a slow one.
const SETTLE: Duration = Duration::from_secs(7);
/// How long to let a keystroke take effect.
const STEP: Duration = Duration::from_millis(600);

/// The built binary, or a skip.
fn binary() -> Option<String> {
    match std::env::var("EPSILON_BIN") {
        Ok(path) if !path.trim().is_empty() => Some(path),
        _ => {
            eprintln!(
                "SKIPPED: set EPSILON_BIN to the built binary to run the terminal \
                 integration test, e.g.\n  \
                 cargo build -p epsilon-cli && EPSILON_BIN=target/debug/epsilon epsilon..."
            );
            None
        }
    }
}

/// A running TUI in a pty, with its screen kept up to date.
struct Session {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: std::sync::Arc<std::sync::Mutex<Box<dyn Write + Send>>>,
    screen: vt100::Parser,
    rx: Receiver<Vec<u8>>,
    _pty: Box<dyn portable_pty::MasterPty + Send>,
}

impl Session {
    /// Start the binary in its own pty. `args` are the CLI arguments.
    fn start(binary: &str, args: &[&str], cwd: &std::path::Path) -> Self {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: 30,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("a pty");

        let mut cmd = CommandBuilder::new(binary);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.cwd(cwd);
        // A deterministic environment: the test must not depend on the developer's
        // terminal variables, and `NO_COLOR` keeps assertions about text stable.
        cmd.env("TERM", "xterm-256color");
        cmd.env("NO_COLOR", "1");

        let child = pair.slave.spawn_command(cmd).expect("the binary starts");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("a reader");
        let writer = pair.master.take_writer().expect("a writer");
        let writer = std::sync::Arc::new(std::sync::Mutex::new(writer));

        // **Answer the terminal's Device Status Report.** A pseudo-console asks
        // "where is the cursor?" with `ESC [ 6 n` and, like a real terminal, will
        // not emit anything else until it hears back. A driver that stays silent
        // looks exactly like a program that renders nothing — which is what this
        // test asserted, wrongly, before the handshake was added here.
        let reply = std::sync::Arc::clone(&writer);
        let (dsr_tx, dsr_rx) = channel::<()>();
        std::thread::spawn(move || {
            let mut answered = 0usize;
            while dsr_rx.recv().is_ok() {
                answered += 1;
                if answered > 64 {
                    break;
                }
                if let Ok(mut w) = reply.lock() {
                    let _ = w.write_all(b"\x1b[1;1R");
                    let _ = w.flush();
                }
            }
        });

        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            let mut tail: Vec<u8> = Vec::new();
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                // Spot the query however the stream splits it across reads.
                tail.extend_from_slice(&buf[..n]);
                if tail.windows(4).any(|w| w == b"\x1b[6n") {
                    let _ = dsr_tx.send(());
                    tail.clear();
                }
                if tail.len() > 8 {
                    let keep = tail.split_off(tail.len() - 8);
                    tail = keep;
                }
                if tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });

        Self {
            child,
            writer,
            screen: vt100::Parser::new(30, 100, 2000),
            rx,
            _pty: pair.master,
        }
    }

    /// Feed everything the program has written into the emulator.
    fn pump(&mut self) {
        while let Ok(chunk) = self.rx.try_recv() {
            self.screen.process(&chunk);
        }
    }

    /// Everything the program has written into the emulator, waiting `for_` first.
    fn settle(&mut self, for_: Duration) {
        let until = Instant::now() + for_;
        while Instant::now() < until {
            self.pump();
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The screen as text, one line per row.
    fn screen(&mut self) -> String {
        self.pump();
        self.screen.screen().contents()
    }

    /// Type literal text, one character at a time.
    ///
    /// Character by character rather than one write, because a burst is a
    /// different thing: the harness coalesces input, and a test that pastes 20
    /// characters at once is testing the coalescer's worst case, not typing.
    fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.key_bytes(ch.to_string().as_bytes());
            self.settle(Duration::from_millis(30));
        }
    }

    /// Send one byte, for keys that are not characters (Enter, Esc).
    fn key(&mut self, byte: u8) {
        self.key_bytes(&[byte]);
    }

    /// Write raw bytes to the program's stdin.
    fn key_bytes(&mut self, bytes: &[u8]) {
        let mut w = self.writer.lock().expect("the writer");
        w.write_all(bytes).expect("a keystroke");
        w.flush().expect("a flush");
    }

    /// Resize the terminal the program is running in.
    ///
    /// Both halves have to move together. Resizing the pty alone leaves the
    /// emulator believing the screen is still its original size, so the test
    /// reads back a screen the program is not drawing into — which makes a
    /// correct resize look like a row overflowing.
    fn resize(&mut self, rows: u16, cols: u16) {
        self._pty
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("a resize");
        self.screen = vt100::Parser::new(rows, cols, 2000);
    }

    /// Is the program still running?
    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// How wide is the widest line the program has put on the screen?
    fn widest_row(&mut self) -> usize {
        self.screen()
            .lines()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// A scratch workspace with nothing in it, so startup is fast and deterministic.
fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a scratch workspace");
    std::fs::write(dir.path().join("README.md"), "# scratch\n").expect("a file");
    dir
}

#[test]
fn the_binary_starts_a_tui_and_draws_its_chrome() {
    let Some(bin) = binary() else { return };
    let ws = workspace();
    let mut session = Session::start(&bin, &[], ws.path());
    session.settle(SETTLE);

    assert!(
        session.alive(),
        "the binary exited instead of opening the TUI. This is the failure a \
         drawing test cannot see: the program never started."
    );

    let screen = session.screen();
    assert!(
        screen.contains("ask anything") || screen.contains("E P S I L O N"),
        "the first frame shows neither the composer placeholder nor the welcome \
         screen, so the TUI opened and drew nothing:\n{screen}"
    );
    assert!(
        screen.contains("idle"),
        "no status line, so the header did not draw:\n{screen}"
    );
    assert!(
        session.widest_row() <= 100,
        "a row is wider than the terminal ({} columns), which wraps and shifts \
         everything below it:\n{screen}",
        session.widest_row()
    );
}

#[test]
fn typing_echoes_into_the_composer() {
    // The reported bug in its simplest form: the program accepts typing and the
    // text never appears. In-process tests cannot fail for this reason, because
    // they never involve raw mode or the event stream.
    let Some(bin) = binary() else { return };
    let ws = workspace();
    let mut session = Session::start(&bin, &[], ws.path());
    session.settle(SETTLE);

    session.type_text("hello there");
    session.settle(STEP);
    let screen = session.screen();
    assert!(
        screen.contains("hello there"),
        "typed text did not reach the screen:\n{screen}"
    );
}

#[test]
fn the_screen_reports_what_the_harness_is_doing() {
    // Transparency is a feature, so it gets a test that runs the binary rather
    // than the widgets: the user must be able to see the model, the context
    // gauge and the cost without asking for them.
    let Some(bin) = binary() else { return };
    let ws = workspace();
    let mut session = Session::start(&bin, &[], ws.path());
    session.settle(SETTLE);

    let screen = session.screen();
    // Before the first reply there is no prompt to measure, so the gauge reads
    // `?/<window>` — the honest answer, and the one the metrics module documents.
    // The test asserts the shape rather than the number: it must show the window
    // size it is measuring against, not a fabricated zero.
    assert!(
        screen.contains("?/") || screen.contains('%'),
        "no context gauge on the first frame:\n{screen}"
    );
    assert!(
        screen.contains('$'),
        "no cost on the first frame:\n{screen}"
    );
    assert!(
        screen.contains("cache"),
        "no cache indicator on the first frame:\n{screen}"
    );
    assert!(
        screen.contains("DeepSeek") || screen.contains("Faux") || screen.contains("offline"),
        "the model is not named on the first frame:\n{screen}"
    );
}

#[test]
fn a_typed_line_is_submitted_and_the_run_is_visible() {
    // The whole loop through the real terminal: type, press Enter, and see the
    // harness acknowledge the turn. This is the path the owner reported as
    // freezing, and it is the one worth the most as a test.
    let Some(bin) = binary() else { return };
    let ws = workspace();
    let mut session = Session::start(&bin, &[], ws.path());
    session.settle(SETTLE);

    session.type_text("say hi");
    session.key(b'\r');
    session.settle(Duration::from_secs(6));

    let screen = session.screen();
    assert!(
        session.alive(),
        "the binary died when a turn was submitted:\n{screen}"
    );
    // The submitted text becomes a transcript row and the composer empties; the
    // header stops saying `idle` at least once during the run. Either the turn
    // completed or it is visibly still running — what must not happen is that
    // nothing on screen changed.
    assert!(
        screen.contains("say hi"),
        "the submitted text is not on screen at all:\n{screen}"
    );
    assert!(
        screen.contains("running")
            || screen.contains("tool-call")
            || screen.contains("Run complete")
            || screen.contains("streaming"),
        "the turn was submitted and the screen shows no sign of it — this is the \
         reported freeze:\n{screen}"
    );
}

#[test]
fn a_narrow_terminal_is_survived_rather_than_mangled() {
    // Below the layout floor the program must say so. What it must not do is
    // crash or draw rows wider than the window.
    let Some(bin) = binary() else { return };
    let ws = workspace();
    let mut session = Session::start(&bin, &[], ws.path());
    session.settle(SETTLE);

    session.resize(10, 44);
    session.settle(Duration::from_secs(3));

    assert!(
        session.alive(),
        "the binary died on resize:\n{}",
        session.screen()
    );
    let screen = session.screen();
    assert!(
        session.widest_row() <= 44,
        "a row is wider than the resized terminal:\n{screen}"
    );
    let _ = session;
}
