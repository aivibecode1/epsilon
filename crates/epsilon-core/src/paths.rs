//! Path resolution shared by every crate that must confine a path to a workspace.
//!
//! This started in `epsilon-tools` (where the tools needed it) and moved here (M74) when the
//! context-reference expander needed the same rule: one implementation, so a file the tools
//! consider inside the workspace can never be one the expander considers outside it.

use std::path::{Component, Path, PathBuf};

/// Resolve `raw` against `root`, normalizing `.`/`..` WITHOUT touching the filesystem,
/// then report whether the result stayed inside the workspace.
///
/// Lexical normalization only: symlink escapes are a Layer-2 (sandbox) concern, not ours.
pub fn resolve_in_workspace(root: &Path, raw: &str) -> (PathBuf, bool) {
    let candidate = Path::new(raw);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let mut normalized = PathBuf::new();
    for comp in joined.components() {
        match comp {
            Component::ParentDir => {
                if !normalized.pop() {
                    // Escaping above the root at join time: mark outside immediately.
                    return (joined, false);
                }
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    let inside = normalized.starts_with(root);
    (normalized, inside)
}
