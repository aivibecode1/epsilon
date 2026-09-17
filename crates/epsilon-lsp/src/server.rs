//! Which language server answers for which file, and where to find it.
//!
//! Two rules shape this table. First, **user-installed only**: opencode carries per-server install
//! commands (npm, `go install`, GitHub release downloads) and this port deliberately does not —
//! a coding agent that fetches and executes binaries is a supply chain, and the failure mode is
//! silent for the user. Second, **extension → server, one server per file**: enough to answer
//! "what does the compiler say about the file I just wrote" without the nine operations
//! opencode's `lsp` tool exposes.

use std::path::{Path, PathBuf};

/// One language server, as this crate needs to know it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerDef {
    /// Stable id, also the key for a user-supplied command path.
    pub id: &'static str,
    /// Human name for logs and the doctor report.
    pub name: &'static str,
    /// File extensions this server answers for, without the dot.
    pub extensions: &'static [&'static str],
    /// Executable to look for on `PATH`.
    pub command: &'static str,
    /// Arguments that make it speak LSP on stdio.
    pub args: &'static [&'static str],
    /// Files whose presence marks the project root (so the server is started once per project).
    pub root_markers: &'static [&'static str],
}

/// The servers this harness knows. Order matters only for extensions claimed twice.
pub const SERVERS: &[ServerDef] = &[
    ServerDef {
        id: "rust-analyzer",
        name: "rust-analyzer",
        extensions: &["rs"],
        command: "rust-analyzer",
        args: &[],
        root_markers: &["Cargo.toml"],
    },
    ServerDef {
        id: "typescript-language-server",
        name: "TypeScript language server",
        extensions: &["ts", "tsx", "js", "jsx", "mjs", "cjs"],
        command: "typescript-language-server",
        args: &["--stdio"],
        root_markers: &["package.json", "tsconfig.json"],
    },
    ServerDef {
        id: "pyright",
        name: "Pyright",
        extensions: &["py", "pyi"],
        command: "pyright-langserver",
        args: &["--stdio"],
        root_markers: &[
            "pyproject.toml",
            "setup.py",
            "setup.cfg",
            "requirements.txt",
        ],
    },
    ServerDef {
        id: "gopls",
        name: "gopls",
        extensions: &["go"],
        command: "gopls",
        args: &["serve"],
        root_markers: &["go.mod", "go.work"],
    },
    ServerDef {
        id: "clangd",
        name: "clangd",
        extensions: &["c", "cc", "cpp", "cxx", "h", "hh", "hpp"],
        command: "clangd",
        args: &[],
        root_markers: &["compile_commands.json", "CMakeLists.txt", ".clangd"],
    },
    ServerDef {
        id: "lua-language-server",
        name: "Lua language server",
        extensions: &["lua"],
        command: "lua-language-server",
        args: &[],
        root_markers: &[".luarc.json", ".luarc.jsonc"],
    },
];

/// The server that answers for `path`, chosen by extension. `None` means no server is even
/// configured for this kind of file, which is the common case and not a failure.
pub fn select_server(path: &Path) -> Option<&'static ServerDef> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    SERVERS
        .iter()
        .find(|def| def.extensions.iter().any(|e| *e == ext))
}

/// Locate `command` on `PATH` (plus the platform's usual executable suffixes).
///
/// Deliberately a plain `PATH` walk rather than a `which` dependency: the lookup is twenty lines,
/// and the result has to be inspectable in tests.
pub fn which(command: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let suffixes: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    for dir in std::env::split_paths(&path) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        for suffix in suffixes {
            let candidate = dir.join(format!("{command}{suffix}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// The project root for `path`: the nearest ancestor holding one of `markers`, else the file's
/// own directory. Starting the server per project rather than per file is what makes the first
/// diagnostic useful (a server needs the workspace loaded to say anything true).
pub fn project_root(path: &Path, markers: &[&str]) -> PathBuf {
    let start = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };
    for dir in start.ancestors() {
        if markers.iter().any(|m| dir.join(m).exists()) {
            return dir.to_path_buf();
        }
    }
    start
}

/// `file:///C:/x/y.rs` for a local path — the URI form LSP notifications carry.
pub fn file_uri(path: &Path) -> String {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut text = absolute.to_string_lossy().replace('\\', "/");
    if !text.starts_with('/') {
        text.insert(0, '/');
    }
    // Percent-encode the characters that would break a URI. Paths with spaces are common enough
    // that leaving them raw means the server silently never matches the document.
    let mut out = String::from("file://");
    for ch in text.chars() {
        match ch {
            ' ' => out.push_str("%20"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            '%' => out.push_str("%25"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_is_chosen_by_extension_or_not_at_all() {
        assert_eq!(
            select_server(Path::new("src/main.rs")).map(|d| d.id),
            Some("rust-analyzer")
        );
        assert_eq!(
            select_server(Path::new("app/page.TSX")).map(|d| d.id),
            Some("typescript-language-server")
        );
        assert_eq!(select_server(Path::new("notes.txt")), None);
        assert_eq!(select_server(Path::new("Makefile")), None);
    }

    #[test]
    fn the_root_is_the_nearest_ancestor_with_a_marker() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]").unwrap();
        let nested = root.join("crates/inner/src");
        std::fs::create_dir_all(&nested).unwrap();
        let file = nested.join("lib.rs");
        std::fs::write(&file, "// x").unwrap();
        let def = select_server(Path::new("lib.rs")).unwrap();
        assert_eq!(project_root(&file, def.root_markers), root);

        // No marker anywhere: the file's own directory is the honest answer.
        let orphan = dir.path().join("elsewhere");
        std::fs::create_dir_all(&orphan).unwrap();
        let loose = orphan.join("x.rs");
        std::fs::write(&loose, "// x").unwrap();
        assert_eq!(project_root(&loose, &["does-not-exist"]), orphan);
    }

    #[test]
    fn a_uri_encodes_the_characters_that_would_break_it() {
        let uri = file_uri(Path::new("/tmp/a dir/file#1.rs"));
        assert!(uri.starts_with("file:///"), "{uri}");
        assert!(uri.contains("a%20dir"), "{uri}");
        assert!(uri.contains("file%231.rs"), "{uri}");
        assert!(!uri.contains(' '), "{uri}");
    }

    #[test]
    fn a_missing_command_is_none_and_never_a_panic() {
        assert_eq!(which("definitely-not-a-real-binary-xyz"), None);
    }
}
