//! Deleting a session file, preferring the system trash.
//!
//! ## What this is for
//!
//! A session file is the only record of a conversation. Removing one is the one destructive act the
//! harness performs on its *own* data rather than on the user's workspace, and until this module
//! existed no surface offered it at all: the session browser could resume, filter and start a
//! session, and had no way to get rid of one. That gap was recorded as parity row P-237.
//!
//! ## The algorithm, and where it comes from
//!
//! Pi's `deleteSessionFile`
//! (`references/pi-1.3.0/packages/coding-agent/src/modes/interactive/components/session-selector.ts:654-689`)
//! tries the `trash` command-line tool first and falls back to a permanent unlink. This module is
//! that algorithm, with two deliberate corrections recorded below rather than quietly applied.
//!
//! ## The two places this does not do what pi does
//!
//! 1. **A missing file is an error, not a success.** Pi's `:675` reads
//!    `if (trashResult.status === 0 || !existsSync(sessionPath))` and answers `{ok: true}` — so a
//!    session that vanished a moment ago is reported as *moved to trash*. The message a reader then
//!    sees (`"Session moved to trash"`, `:857`) claims a recoverable copy that was never made. A
//!    caller cannot detect this: from the outside, "already gone" and "moved to trash" are the same
//!    observation. So the existence check happens here, first, and answers `Err`.
//! 2. **A successful `trash` exit is verified by the file actually leaving.** Pi trusts the exit
//!    status alone. A `trash` that reports success and leaves the file behind would make the
//!    fallback below unreachable — and the caller would keep showing a row that is still on disk.
//!    Verifying is free; the wrong trust is what costs.
//!
//! ## What the trash preference is worth, platform by platform
//!
//! The preference is real where a `trash` binary exists (trash-cli on Linux, the `trash` formula on
//! macOS) and dead where none does — **Windows has no `trash` command**, so on the platform this
//! project is developed on the preferred branch can never succeed and every deletion is the
//! unlink fallback. That is not a defect in this module: it is pi's behaviour reproduced faithfully,
//! and the honest cost is that a Windows user gets a permanent delete behind a confirmation.
//!
//! `ponytail:` spawning `trash` is pi-parity at zero dependency cost. Reaching the Windows recycle
//! bin natively needs `SHFileOperationW` or `IFileOperation` through a crate — a supply-chain
//! decision that wants an ADR, not a `Cargo.toml` edit. **Ceiling: no native OS trash. Upgrade
//! trigger: a request that deleted sessions be recoverable on Windows.**

use std::path::Path;
use std::process::{Command, Stdio};

/// How a session file left the disk, and therefore what a reader must be told.
///
/// Two variants rather than a bool because the two outcomes have different recoverability and the
/// messages differ (`"Session moved to trash"` against `"Session deleted"`, pi
/// `session-selector.ts:857`). A bool invites the caller to print the wrong one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteOutcome {
    /// A `trash` tool took the file, so a system trash holds a recoverable copy.
    Trash,
    /// The file was unlinked. There is no copy and no undo.
    Unlink,
}

impl DeleteOutcome {
    /// The sentence a reader sees on success, matching pi's wording
    /// (`session-selector.ts:857`).
    ///
    /// Written here rather than at each call site so the message and the outcome cannot drift
    /// apart — the same reason [`crate::store::original_backup_path`] lives beside the code that
    /// writes the file.
    pub fn message(self) -> &'static str {
        match self {
            DeleteOutcome::Trash => "Session moved to trash",
            DeleteOutcome::Unlink => "Session deleted",
        }
    }
}

/// Delete the session file at `path`, preferring the `trash` tool.
///
/// The error string is the unlink failure plus, when the `trash` attempt produced anything worth
/// saying, a truncated hint about it — pi's `getTrashErrorHint`
/// (`session-selector.ts:661-672`, joined at `:686`). A reader who asks why a delete failed should
/// not have to guess which of the two attempts was the one that mattered.
pub fn delete_session_file(path: &Path) -> Result<DeleteOutcome, String> {
    // Divergence 1 in the module docs: pi's `:675` treats a vanished file as a trashed one, which
    // would have this report a recoverable copy that was never made. A session listed a moment ago
    // and gone now is a race worth naming, not a success worth printing.
    if !path.exists() {
        return Err(format!("no such session file: {}", path.display()));
    }
    // A directory is never a session file. Refuse it before `trash`, which may
    // happily move a whole directory to the trash on platforms that have one.
    if !path.is_file() {
        return Err(format!("not a session file: {}", path.display()));
    }

    let trash_hint = try_trash(path);
    // Divergence 2 in the module docs: pi trusts `trash`'s exit status. The file leaving is the
    // outcome that matters, so it is what is checked.
    if trash_hint.is_none() && !path.exists() {
        return Ok(DeleteOutcome::Trash);
    }

    match std::fs::remove_file(path) {
        Ok(()) => Ok(DeleteOutcome::Unlink),
        Err(e) => {
            let mut message = e.to_string();
            if let Some(hint) = trash_hint {
                // pi truncates the hint to 200 characters (`:671`).
                let hint: String = hint.chars().take(200).collect();
                message.push_str(&format!(" (trash: {hint})"));
            }
            Err(message)
        }
    }
}

/// Run the `trash` tool over `path`, returning `None` when it took the file and `Some(hint)`
/// when it did not — where `hint` is what the attempt had to say about itself.
///
/// Stdin is null: this runs synchronously inside a key handler, and a `trash` that blocks reading a
/// terminal the TUI already owns would hang the interface rather than the command.
fn try_trash(path: &Path) -> Option<String> {
    let mut command = Command::new("trash");
    // A session path is generated (`<iso>_<id>.jsonl`), so a leading `-` cannot occur today — pi
    // guards it anyway (`:658`) and the guard is one arm, so it stays.
    if path.as_os_str().to_string_lossy().starts_with('-') {
        command.arg("--");
    }
    let spawned = command
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();

    match spawned {
        // Success is not believed on its own; the caller verifies the file left. `None`
        // means "the tool reported success" — returning `Some("")` made the caller
        // treat a real trash as a failed attempt and then unlink an already-moved file.
        Ok(output) if output.status.success() => None,
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let first = stderr.lines().next().unwrap_or("").trim();
            if first.is_empty() {
                Some(format!("exit {}", output.status))
            } else {
                Some(format!("exit {} · {first}", output.status))
            }
        }
        // No `trash` binary on this machine (every Windows, most macOS). The message is the one
        // thing worth appending to an unlink failure, so it is kept rather than dropped.
        Err(e) => Some(e.to_string()),
    }
}
