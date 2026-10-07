//! The `epsilon-desktop` binary.
//!
//! Everything of substance lives in the library, so that a test can drive the same draw path
//! without a process. This file exists to parse arguments and open the window.
//!
//! **The CLI does not offer this window behind a flag**, which an earlier version of this
//! comment claimed: `epsilon-cli` has no dependency on this crate, and adding one would make
//! the CLI carry a GUI stack it does not otherwise need. The window has its own binary, and
//! that is the whole of it.

use std::path::PathBuf;

const HELP: &str = "\
epsilon-desktop — the Epsilon IDE, in a native window

USAGE:
    epsilon-desktop [PATH] [OPTIONS]

ARGS:
    PATH    Workspace to open. Defaults to the current directory.

OPTIONS:
    -h, --help          Print this help and exit.
        --session FILE  Open a specific session JSONL instead of starting one.
        --model ID      Run on a specific model, overriding the saved choice.
        --light         Use the light palette (otherwise follows EPSILON_THEME).
        --dark          Use the dark palette explicitly.
";

fn main() -> std::process::ExitCode {
    let mut workspace: Option<PathBuf> = None;
    let mut session: Option<PathBuf> = None;
    let mut model_id: Option<String> = None;
    let mut theme: Option<epsilon_tui::ThemeMode> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return std::process::ExitCode::SUCCESS;
            }
            "--session" => match args.next() {
                Some(p) => session = Some(PathBuf::from(p)),
                None => return fail("--session needs a path"),
            },
            "--model" => match args.next() {
                Some(m) => model_id = Some(m),
                None => return fail("--model needs an id"),
            },
            "--light" => theme = Some(epsilon_tui::ThemeMode::Light),
            "--dark" => theme = Some(epsilon_tui::ThemeMode::Dark),
            other if other.starts_with('-') => return fail(&format!("unknown option: {other}")),
            other => workspace = Some(PathBuf::from(other)),
        }
    }

    let workspace = workspace
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    let opts = epsilon_desktop::Options {
        workspace,
        theme,
        session,
        model_id,
        // Shared with the terminal (P-040): one preference, both front ends.
        hide_thinking: epsilon_core::config::load_hide_thinking(),
        // Live provider listings for the model picker, built exactly as the
        // terminal's binary builds them (same endpoint, same auth).
        live: live_fetcher(),
    };

    match epsilon_desktop::run(opts) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => fail(&format!("could not open the Epsilon window: {e}")),
    }
}

/// Live provider-listing capability for the model picker: present only when
/// built with `--features real`; the fetch is a plain authenticated GET
/// against the provider's models endpoint. Built exactly as the terminal's
/// binary builds it, so both front ends merge the same live rows.
fn live_fetcher() -> Option<epsilon_tui::LiveFetcher> {
    #[cfg(feature = "real")]
    {
        Some(std::sync::Arc::new(|hint: String| {
            Box::pin(async move { epsilon_llm::catalog::live_models(&hint).await })
                as futures_util::future::BoxFuture<'static, Result<Vec<String>, String>>
        }))
    }
    #[cfg(not(feature = "real"))]
    {
        None
    }
}

/// Print a message and return the failure exit code.
///
/// Written out rather than inlined so every failure path leaves the same shape:
/// the message on stderr, exit code 2.
fn fail(message: &str) -> std::process::ExitCode {
    eprintln!("epsilon-desktop: {message}");
    eprintln!("try `epsilon-desktop --help`");
    std::process::ExitCode::from(2)
}
