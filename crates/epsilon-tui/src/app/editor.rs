//! The external-editor bridge: `$EDITOR` resolution, and the suspend/spawn/read-back
//! round trip a user expects from `Ctrl+G`.
//!
//! ## Why this file exists
//!
//! Five free functions that answer "which editor, and what did it do to my draft". They were
//! interleaved in `app.rs` between `TuiConfig` and the `now_ms` helper, sharing a file with
//! state that has nothing to do with spawning a process.
//!
//! Two of them are already shaped for testing and it shows: `editor_command_from` takes the
//! three candidate settings as arguments instead of reading the environment, so the
//! precedence order is a pure function; `edit_with` takes the spawner as a `&dyn Fn`, so the
//! whole round trip runs with no real editor, no TTY and no suspend. That is the pattern the
//! rest of the codebase should copy, and it was invisible while these sat in a 2,000-line
//! file between unrelated items.
//!
//! ## The rule for this move
//!
//! Sliced by line range and byte-identical to `app.rs` at the parent commit.

use super::*;

/// M59: which editor to run — explicit config wins, then `$VISUAL`, then
/// `$EDITOR` (pi's `getExternalEditorCommand` order), else a telling error.
/// Pure so the order is testable without touching the environment.
pub(super) fn editor_command_from(
    configured: Option<&str>,
    visual: Option<&str>,
    editor: Option<&str>,
) -> Result<String, String> {
    for candidate in [configured, visual, editor] {
        if let Some(cmd) = candidate.map(str::trim).filter(|c| !c.is_empty()) {
            return Ok(cmd.to_string());
        }
    }
    Err("no editor configured — set $EDITOR or $VISUAL, or add\n    [editor]\n    command = \"code --wait\"\nto ~/.config/epsilon/config.toml".to_string())
}

/// M59: the terminal-free half of the external editor — write the draft to a
/// temp file, hand it to `spawn`, read it back. Split out (spawner injected) so
/// the whole round trip is testable without a real editor, a TTY, or a suspend.
pub(super) fn edit_with(
    draft: &str,
    spawn: &dyn Fn(&std::path::Path) -> Result<i32, String>,
) -> Result<String, String> {
    let dir = std::env::temp_dir().join(format!(
        "epsilon-editor-{}",
        epsilon_contracts::new_id("ed")
    ));
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let file = dir.join("prompt.md");
    let result = (|| -> Result<String, String> {
        std::fs::write(&file, draft).map_err(|e| format!("write {}: {e}", file.display()))?;
        let code = spawn(&file)?;
        if code != 0 {
            return Err(format!("editor exited with status {code} — draft kept"));
        }
        let edited =
            std::fs::read_to_string(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
        // pi trims exactly one trailing newline (editors love adding one).
        Ok(edited.strip_suffix('\n').unwrap_or(&edited).to_string())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Launch `command <file>` with the terminal handed over, and wait for it. The
/// platform shell expands the user's own command line (`code --wait`,
/// `vim -u NONE`, `notepad`) exactly as a shell would.
pub(super) fn spawn_editor(command: &str, file: &std::path::Path) -> Result<i32, String> {
    use std::process::{Command, Stdio};
    let quoted = format!("\"{}\"", file.display());
    #[cfg(windows)]
    let mut child = {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(format!("{command} {quoted}"));
        c
    };
    #[cfg(not(windows))]
    let mut child = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(format!("{command} {quoted}"));
        c
    };
    let status = child
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("cannot start '{command}': {e}"))?;
    Ok(status.code().unwrap_or(-1))
}

/// M59: suspend the TUI, run the editor on the draft, resume. The event loop
/// owns the terminal, so this is called between frames — never from a handler.
pub(super) fn open_external_editor(terminal: &mut ratatui::DefaultTerminal, app: &mut App) {
    let Ok(command) = editor_command(epsilon_core::config::load_config_string(
        "editor", "command",
    )) else {
        return;
    };
    let draft = app.input.clone();
    // Leave the alternate screen + raw mode so the child owns the terminal (pi
    // `ui.stop()`); bracketed paste is re-armed on the way back.
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste);
    if let Err(e) = ratatui::try_restore() {
        app.ui
            .push_notice(format!("editor: terminal restore failed: {e}"));
        return;
    }
    let outcome = edit_with(&draft, &|path| spawn_editor(&command, path));
    match ratatui::try_init() {
        Ok(mut fresh) => {
            // A fresh terminal starts with blank buffers, so the next draw is a
            // full repaint — no stale half-screen after the editor exits.
            let _ = fresh.clear();
            *terminal = fresh;
        }
        Err(e) => app
            .ui
            .push_notice(format!("editor: cannot re-enter the UI: {e}")),
    }
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste);
    match outcome {
        Ok(text) => {
            let trimmed = text.trim_end_matches('\n').to_string();
            app.cursor_chars = trimmed.chars().count();
            app.input = trimmed;
            app.ui.push_notice("editor: draft updated");
        }
        Err(e) => app.ui.push_notice(format!("editor: {e}")),
    }
}

/// [`editor_command_from`] reading the real environment.
pub(super) fn editor_command(configured: Option<String>) -> Result<String, String> {
    let visual = std::env::var("VISUAL").ok();
    let editor = std::env::var("EDITOR").ok();
    editor_command_from(configured.as_deref(), visual.as_deref(), editor.as_deref())
}
