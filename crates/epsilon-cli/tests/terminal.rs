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
///
/// M179: this used to require `EPSILON_BIN` to be set by hand, which meant the whole
/// suite was skipped by `cargo test --workspace` — and a test that is skipped by the
/// battery rots. `the_screen_reports_what_the_harness_is_doing` sat broken from M151 to
/// M178 for exactly that reason: nothing ran it for twenty-seven milestones.
///
/// Cargo already solves this. For an integration test in a package that has a binary
/// called `epsilon`, `CARGO_BIN_EXE_epsilon` points at the binary Cargo has just built,
/// so the suite can find it with no environment variable at all. `EPSILON_BIN` stays
/// honoured, because pointing the suite at a specific build is occasionally the point.
///
/// It still skips, loudly, in the one case that cannot be helped: running the compiled
/// test binary on its own with no build having happened (`cargo test --no-run`, a stray
/// binary). A skip is then honest; a silent pass would not be.
fn binary() -> Option<String> {
    if let Ok(path) = std::env::var("EPSILON_BIN")
        && !path.trim().is_empty()
    {
        return Some(path);
    }
    // `option_env!` rather than `env!`: the variable is absent when this crate is
    // compiled as a library or with `--no-run`, and a hard `env!` would make the whole
    // test binary fail to compile for a condition that has a correct answer.
    match option_env!("CARGO_BIN_EXE_epsilon") {
        Some(path) => Some(path.to_string()),
        None => {
            eprintln!(
                "SKIPPED: no EPSILON_BIN and no CARGO_BIN_EXE_epsilon. Run the suite
                through cargo, which builds the binary first: cargo test -p epsilon-cli
                --test terminal -- --test-threads=1"
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
        let mut cmd = CommandBuilder::new(binary);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.cwd(cwd);
        Self::start_with(cmd)
    }

    /// Start the binary **through a shell**, which is how a user starts it.
    ///
    /// The tests above launch `epsilon` as the pty's own process, so no conhost, no
    /// profile and no shell banner are in the way. That is the right setup for testing
    /// the TUI and the wrong setup for the owner's actual report, which was about
    /// launching it from PowerShell. This is that path: the shell is the pty's process
    /// and the TUI is its child, inheriting the same console.
    ///
    /// Whether this shell is PowerShell.
    ///
    /// Matched on the resolved path rather than with `ends_with("powershell")`:
    /// the path is `powershell.exe`, so that suffix never matched and
    /// PowerShell fell through to the cmd branch, where `/Q /K` is not a
    /// PowerShell argument and the shell exited with a parser error.
    fn is_powershell(shell: &str) -> bool {
        shell.to_lowercase().contains("powershell")
    }

    /// Start `shell` and run `script` in it.
    ///
    /// `script` is the shell's own command text, so the caller owns the quoting
    /// rather than this helper guessing at it for two different shells.
    fn start_in_shell(shell: &str, script: &str, cwd: &std::path::Path) -> Self {
        let mut cmd = CommandBuilder::new(shell);
        cmd.cwd(cwd);
        if Self::is_powershell(shell) {
            cmd.arg("-NoProfile");
            cmd.arg("-NoLogo");
            cmd.arg("-Command");
            cmd.arg(script);
            return Self::start_with(cmd);
        }

        // M196: cmd gets the command through **stdin**, not through `/c`.
        //
        // `cmd /c "<text>"` fails for a path containing a space, and it fails in
        // a way that hides the cause. `portable-pty` builds the child's command
        // line itself, so the quotes this test adds come back to cmd
        // backslash-escaped, and cmd reports the name it was asked for:
        //
        //     '"C:\...\epsilon.exe"' is not recognized as an internal or
        //      external command, operable program or batch file.
        //
        // The backslashes in that message are the tell — cmd is not echoing the
        // command, it is quoting back the *program name* it resolved from the
        // mangled argument. Adding `call`, using the 8.3 short path, and
        // dropping the quotes were each measured: none of them help, because
        // the damage happens in argv construction before cmd parses anything.
        //
        // Writing the command into the shell's input stream avoids argv
        // entirely. This is also what a user does — types the command at the
        // prompt — so the test exercises the real path rather than a
        // programmatic one that only the test uses.
        cmd.arg("/Q"); // no echo: we assert on the screen, not on the command
        cmd.arg("/K"); // keep the shell alive so the TUI owns the console
        cmd.arg("PROMPT $P$G");
        Self::start_with(cmd)
    }

    fn start_with(mut cmd: CommandBuilder) -> Self {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: 30,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("a pty");

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
    // M178: this used to assert a `$` and the word "cache" on the first frame, and
    // had been failing since M151 without anyone noticing - it needs EPSILON_BIN, so
    // the battery skips it. M151 made both of those deliberately untrue: cost is shown
    // only once there is one, because a fabricated `$0.00` is a lie, and the cache
    // became a volume/rate form (`R0 W0 CH -`). Asserting the current contract is the
    // fix; the stale assertion was the bug.
    //
    // Polish wave: a zero-traffic session collapses the whole usage cluster
    // (`↑0 ↓0 · R0 W0 CH n/a` + cost) into one quiet `no usage yet` segment —
    // the same honesty rule as the cost above (no fabricated zeros), so the
    // first frame names neither CH nor R/W. The cluster returns with the
    // first traffic (asserted after the turn below, where `$` reappears).
    assert!(
        !screen.contains('$'),
        "a cost is shown before any turn has run, so it must be fabricated:\n{screen}"
    );
    assert!(
        screen.contains("no usage yet"),
        "an empty session must collapse its zero gauges, not print them:\n{screen}"
    );
    assert!(
        screen.contains("DeepSeek") || screen.contains("Faux") || screen.contains("offline"),
        "the model is not named on the first frame:\n{screen}"
    );

    // The other half of the cost contract: it is *hidden* before a turn and *shown*
    // after one. Asserting only the first half would let a header that never shows a
    // cost at all pass - which is exactly the kind of test that outlives the thing it
    // was written for.
    session.type_text("say hi");
    session.key(b'\r');
    session.settle(Duration::from_secs(6));
    let after = session.screen();
    assert!(
        after.contains('$'),
        "no cost after a turn has run, so the estimate is missing:\n{after}"
    );
}

#[test]
fn typing_still_works_while_a_turn_is_streaming() {
    // The reported defect, and the test this repository never had.
    //
    // The claim was: the TUI "freezes and shows no input or output the moment you
    // start typing, even though tokens are clearly being consumed underneath" —
    // three assertions at once, none of which any prior test could check. Every
    // other test in this file runs against a turn that is over in microseconds,
    // because `FauxProvider` emits its whole event sequence with no delay. A test
    // that only ever photographs a finished turn cannot see a freeze that happens
    // mid-turn.
    //
    // `--stream-pace-ms` (crates/epsilon-llm/src/paced.rs) makes a turn last
    // seconds, so this test drives the state the report is actually about: submit,
    // wait until the header says the turn is in flight, then type.
    //
    // Measured against a real conhost window on 2026-09-28, at 100x30 under both
    // cmd and PowerShell, the typed text appears while the turn is at `running 2s`
    // and the assistant output keeps growing. This test is what keeps that true.
    let Some(bin) = binary() else { return };
    let ws = workspace();

    // A long answer, so the paced turn is long enough to type into. 30ms per event
    // over ~100 deltas is a turn measured in seconds.
    //
    // Written by hand rather than with `serde_json`: this crate has two
    // dev-dependencies (portable-pty and vt100) and adding a third to format eight
    // lines of JSON would be a poor trade. The text is plain ASCII, so it needs no
    // escaping beyond the surrounding quotes.
    let script = ws.path().join("paced-turn.json");
    let body = "This answer is long on purpose so that the turn stays in flight \
                while the test types into the composer. "
        .repeat(6);
    std::fs::write(
        &script,
        format!("[{{\"type\":\"assistant\",\"text\":\"{body}\"}}]"),
    )
    .expect("the script is written");
    let script_arg = script.to_string_lossy().to_string();

    let mut session = Session::start(
        &bin,
        &[
            "--provider",
            "faux",
            "--script",
            &script_arg,
            "--stream-pace-ms",
            "30",
        ],
        ws.path(),
    );
    session.settle(SETTLE);

    session.type_text("explain the freeze");
    session.key(b'\r');

    // Wait for the turn to actually be running before typing. Without this the
    // test could pass for the wrong reason: a fast turn would already be over and
    // the typing would be tested against a settled screen, which is a state the
    // other tests here already cover.
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut streaming = false;
    while std::time::Instant::now() < deadline {
        if session.screen().contains("streaming") {
            streaming = true;
            break;
        }
        session.settle(Duration::from_millis(250));
    }
    assert!(
        streaming,
        "the turn never reported itself in flight, so this test would not be \
         exercising the state it exists for"
    );

    // Now type, while the turn is streaming.
    session.type_text("typed while streaming");
    session.settle(Duration::from_millis(800));
    let mid = session.screen();

    assert!(
        mid.contains("typed while streaming"),
        "typing during a live turn did not reach the screen:\n{mid}"
    );
    assert!(
        mid.contains("streaming"),
        "the turn stopped being reported in flight after typing, which is the \
         freeze this test exists to catch:\n{mid}"
    );
}

#[test]
fn a_typed_line_is_submitted_and_the_run_is_visible() {
    // The whole loop through the real terminal: type, press Enter, and see the
    // harness acknowledge the turn. This is the path the owner reported as
    // freezing, and it is the one worth the most as a test.
    //
    // **`--provider faux` is load-bearing, and it was missing.** With no flag the
    // launch auto-detects: `auto_real` returns `Some` as soon as *any* provider key
    // is present (`epsilon-runtime/src/lib.rs:5845`), so on a developer machine with a
    // stored key this test silently became a live-network test. Measured failing on
    // 2026-10-04 against a real 401, with an error row where the assertions expected a
    // run — and the test could not tell the difference, because a *refused* turn and a
    // *successful* one differ only in which strings appear. Pinning the provider makes
    // it hermetic and makes its own precondition true by construction.
    let Some(bin) = binary() else { return };
    let ws = workspace();
    let mut session = Session::start(&bin, &["--provider", "faux"], ws.path());
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
/// M178: the same TUI, started the way a user starts it — through a shell.
///
/// Everything above launches `epsilon` as the pty's own process. That is the cleanest
/// way to test a TUI, and it is not what the owner did: the report was about launching
/// it from CMD and from PowerShell, and M177 measured that only with the out-of-band
/// probe. This puts the shell in the suite, so a PowerShell-only regression is a test
/// failure rather than something nobody notices until a user reports it again.
///
/// The shell is the pty's process and the TUI is its child. That is the point: a TUI
/// that draws correctly when it owns the terminal can still fail when it is a child of a
/// shell that has already written a prompt and a banner to the same screen.
fn shell_path(shell: &str) -> Option<String> {
    // A shell that is not installed is a skip, not a failure: this suite also runs on
    // ubuntu, where neither cmd.exe nor powershell.exe exists.
    let found = which(shell);
    match found {
        Some(p) => Some(p),
        None => {
            eprintln!("SKIPPED: {shell} is not on PATH, so this cannot run here");
            None
        }
    }
}

/// Resolve an executable on PATH. Deliberately small and Windows-tolerant.
fn which(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect();
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
        // A bare name on a unix-like PATH still deserves a try.
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

/// Drive the TUI through a shell: it must open, draw, and accept typed input.
fn assert_works_through_shell(shell: &str) {
    let Some(shell_path) = shell_path(shell) else {
        return;
    };
    let Some(bin) = binary() else { return };
    let ws = workspace();
    // The command the shell runs. Quoting is the caller's, because cmd and PowerShell
    // disagree about it and guessing wrong produces a shell error that looks like a TUI
    // failure — which is the M170 mistake in a different costume.
    // `&` is PowerShell's call operator and a command separator in cmd.exe, so one
    // spelling cannot serve both: under cmd it would be a syntax error that looks
    // exactly like a TUI that never started.
    // The binary path, spelled so cmd can execute it.
    //
    // M196: this test fails on any machine whose build directory contains a
    // space — this repository's does — and not because the TUI is broken. The
    // pty hands `cmd` an argv it has already quoted, so the double quotes this
    // script adds arrive backslash-escaped and cmd searches for a program
    // literally named:
    //
    //     '"C:\Users\...\epsilon.exe"' is not recognized as an internal or
    //      external command, operable program or batch file.
    //
    // The backslashes in that message are the tell: they are not cmd echoing
    // the command, they are cmd reporting the NAME it was asked for.
    //
    // Quoting alone was measured NOT to be the fix — from PowerShell,
    // `cmd /c "path"` runs the binary, and so does the same text via a `.cmd`
    // file. The 8.3 short path was tried next and is not available either:
    // this volume has no short names, so `%~sI` returns the input unchanged.
    //
    // `call` is the spelling that works, because it tells cmd to parse the
    // quoted string as a command rather than treating the whole of `/c` as one
    // literal argument. Verified by feeding the identical text through a pty's
    // stdin, which is how this test starts the shell.
    let script = if shell_path.to_lowercase().contains("powershell") {
        format!("& '{}'", bin)
    } else if bin.contains(' ') {
        // `call` plus quotes, TYPED rather than passed as argv.
        //
        // Both halves are needed and each was measured. A bare path splits on
        // its space; quotes alone work when cmd reads the line itself but not
        // when the line comes in as argv, because the pty's own quoting then
        // survives cmd's quote-stripping as literal characters and cmd hunts
        // for a program named `"C:\...\epsilon.exe"`. `call` tells cmd to
        // parse the quoted string as a command instead of treating the whole
        // of `/c` as one argument, which is what makes the quotes mean
        // something.
        format!("call \"{bin}\"")
    } else {
        // No spaces, no quotes, no ambiguity.
        bin.clone()
    };
    let mut session = Session::start_in_shell(&shell_path, &script, ws.path());
    // cmd was started with `/K` and no command, so the command goes in the way a
    // user types it: keystrokes into the shell's input. For PowerShell the
    // command was already passed via `-Command`, so there is nothing to type.
    if !shell_path.to_lowercase().contains("powershell") {
        session.settle(Duration::from_millis(500));
        session.type_text(&script);
        session.key(b'\r');
    }
    session.settle(SETTLE);

    assert!(
        session.alive(),
        "the binary exited instead of opening the TUI when started from {shell}. This is
        the failure a drawing test cannot see: the program never started.\n--- screen ---\n{}\n--- end ---",
        session.screen()
    );
    let screen = session.screen();
    assert!(
        screen.contains("ask anything") || screen.contains("E P S I L O N"),
        "started from {shell}, the first frame shows neither the composer placeholder
        nor the welcome screen, so the TUI opened and drew nothing:\n{screen}"
    );
    assert!(
        screen.contains("idle"),
        "started from {shell}, no status line, so the header did not draw:\n{screen}"
    );

    // Typing is the half of the owner's report that matters: the screen froze "the
    // moment you start typing". Echo proves the keys reached the composer.
    session.type_text("hello from the shell test");
    session.settle(STEP);
    let after = session.screen();
    assert!(
        after.contains("hello from the shell test"),
        "typed text never reached the composer when started from {shell}:\n{after}"
    );
    assert!(
        session.widest_row() <= 100,
        "a row is wider than the terminal when started from {shell}"
    );
}

#[test]
fn the_tui_works_when_started_from_powershell() {
    assert_works_through_shell("powershell");
}

#[test]
fn the_tui_works_when_started_from_cmd() {
    assert_works_through_shell("cmd");
}
