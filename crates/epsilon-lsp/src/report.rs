//! The bounded block an edit carries: `ERROR [line:col] message`.
//!
//! opencode appends this to the edit tool's own output text (`tool/edit.ts:196-211`,
//! `lsp/diagnostic.ts:20-27`) and caps it — errors only, twenty per file. Both caps are the
//! point: a diagnostics dump would push the file's contents out of the model's view, and warnings
//! are the language server's opinions, while errors are what the compiler will refuse.

use std::fmt::Write as _;
use std::path::Path;

/// Diagnostic severity as LSP numbers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// 1 — the code does not compile / the type is wrong.
    Error,
    /// 2 — suspicious but permitted.
    Warning,
    /// 3 — information.
    Info,
    /// 4 — hint.
    Hint,
}

impl Severity {
    /// From the wire number; unknown numbers are treated as `Info` rather than dropped.
    pub fn from_lsp(value: i64) -> Self {
        match value {
            1 => Severity::Error,
            2 => Severity::Warning,
            3 => Severity::Info,
            _ => Severity::Hint,
        }
    }

    /// The word used in the report.
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "ERROR",
            Severity::Warning => "WARN",
            Severity::Info => "INFO",
            Severity::Hint => "HINT",
        }
    }
}

/// One diagnostic, reduced to what an edit's feedback needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// 1-based line.
    pub line: u32,
    /// 1-based column.
    pub column: u32,
    /// Severity.
    pub severity: Severity,
    /// The message, already flattened to one line.
    pub message: String,
}

/// How many diagnostics one file may contribute to a tool result.
pub const MAX_REPORTED: usize = 20;

/// Render the block for `path`, or `None` when there is nothing to say.
///
/// Errors only — a file with warnings alone produces no block at all, because the model cannot act
/// on "unused import" and every extra line costs context.
pub fn render_report(path: &Path, diagnostics: &[Diagnostic]) -> Option<String> {
    let mut errors: Vec<&Diagnostic> = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    if errors.is_empty() {
        return None;
    }
    // Line order, so the model reads the file top-down like the file itself.
    errors.sort_by_key(|d| (d.line, d.column));
    let shown = errors.len().min(MAX_REPORTED);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());

    let mut out = String::new();
    let _ = writeln!(
        out,
        "[lsp] {name}: {shown} error{}",
        if errors.len() == 1 { "" } else { "s" }
    );
    for d in errors.iter().take(shown) {
        // Flattened: a raw newline inside a message would read as a new entry in the tool output.
        let message = d.message.replace(['\n', '\r'], " ");
        let _ = writeln!(
            out,
            "  {} [{}:{}] {}",
            d.severity.label(),
            d.line,
            d.column,
            message
        );
    }
    if errors.len() > shown {
        let _ = writeln!(
            out,
            "  (+{} more not shown)",
            errors.len().saturating_sub(shown)
        );
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diag(line: u32, sev: Severity, msg: &str) -> Diagnostic {
        Diagnostic {
            line,
            column: 5,
            severity: sev,
            message: msg.into(),
        }
    }

    #[test]
    fn warnings_alone_produce_no_block_because_the_model_cannot_act_on_them() {
        let diags = vec![diag(3, Severity::Warning, "unused import")];
        assert_eq!(render_report(Path::new("src/lib.rs"), &diags), None);
        assert_eq!(render_report(Path::new("src/lib.rs"), &[]), None);
    }

    #[test]
    fn errors_are_reported_in_file_order_with_their_position() {
        let diags = vec![
            diag(12, Severity::Error, "mismatched types"),
            diag(2, Severity::Error, "cannot find value `x`"),
            diag(12, Severity::Warning, "unused"),
        ];
        let block = render_report(Path::new("src/lib.rs"), &diags).expect("errors exist");
        assert!(block.contains("[lsp] lib.rs: 2 errors"), "{block}");
        let first = block.find("cannot find value").unwrap();
        let second = block.find("mismatched types").unwrap();
        assert!(first < second, "line order: {block}");
        assert!(
            block.contains("ERROR [2:5] cannot find value `x`"),
            "{block}"
        );
        assert!(!block.contains("unused"), "{block}");
    }

    #[test]
    fn the_report_is_capped_and_says_how_much_it_left_out() {
        let diags: Vec<Diagnostic> = (1..=25).map(|i| diag(i, Severity::Error, "boom")).collect();
        let block = render_report(Path::new("a.rs"), &diags).unwrap();
        assert_eq!(block.matches("ERROR [").count(), MAX_REPORTED);
        assert!(block.contains("(+5 more not shown)"), "{block}");
    }

    #[test]
    fn a_multiline_message_stays_one_line() {
        // Tool output shape must survive: a raw newline in a message would look like a new entry.
        let d = Diagnostic {
            line: 1,
            column: 1,
            severity: Severity::Error,
            message: "expected one of\nthe following".into(),
        };
        let block = render_report(Path::new("a.rs"), &[d]).unwrap();
        assert_eq!(block.lines().count(), 2, "{block}");
    }

    #[test]
    fn severity_numbers_map_to_words_without_dropping_unknown_ones() {
        assert_eq!(Severity::from_lsp(1), Severity::Error);
        assert_eq!(Severity::from_lsp(2), Severity::Warning);
        assert_eq!(Severity::from_lsp(3), Severity::Info);
        assert_eq!(Severity::from_lsp(4), Severity::Hint);
        assert_eq!(Severity::from_lsp(99), Severity::Hint);
    }
}
