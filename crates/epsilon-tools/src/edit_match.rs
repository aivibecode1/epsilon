//! Fuzzy-recovery matching for the edit tool.
//!
//! Ported idea (opencode `tool/edit.ts` replacer chain, simplified): models copy
//! `old_string` out of `read` output, which renders LF-stripped lines — so on
//! CRLF files a byte-exact match systematically fails. The chain:
//!
//! 1. **Normalize** the file and both arguments to LF, remember the file's
//!    dominant ending, restore it on write.
//! 2. **Exact** match (the common case, still first).
//! 3. **Line-trimmed** match: per-line equality ignoring trailing whitespace
//!    (catches copied trailing spaces / editor artifacts).
//! 4. **Indentation-flexible** match: per-line equality ignoring leading
//!    whitespace (catches the model guessing the indent level wrong).
//!
//! Deliberately NOT ported: opencode's Levenshtein block-anchor stage — it needs
//! a scoring pass for marginal gain over stages 3-4, and this crate carries no
//! algorithmic dependencies. Occurrence-count safety (exactly-once unless
//! `replace_all`) applies to EVERY stage, not just exact matching.

/// Which stage of the chain produced the replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchStrategy {
    /// Byte-for-byte after EOL normalization.
    Exact,
    /// Per-line equality after trimming both ends of every line.
    LineTrimmed,
    /// Per-line equality after stripping leading whitespace and trimming.
    IndentationFlexible,
}

impl MatchStrategy {
    /// Human-readable note for the tool result (empty for the exact path).
    pub fn note(self) -> &'static str {
        match self {
            Self::Exact => "",
            Self::LineTrimmed => " (matched after ignoring trailing whitespace)",
            Self::IndentationFlexible => " (matched after ignoring indentation differences)",
        }
    }
}

/// Result of planning an edit against LF-normalized content.
#[derive(Debug, PartialEq, Eq)]
pub enum EditOutcome {
    /// Replacement planned; `output` is the full new file body (LF-normalized).
    Applied {
        output: String,
        count: usize,
        strategy: MatchStrategy,
    },
    /// No stage of the chain found the text.
    NotFound,
    /// A stage found the text more than once and `replace_all` was not set.
    Ambiguous(usize),
}

/// The file's dominant line ending: CRLF when any CRLF is present, else LF.
/// (Mixed-ending files normalize to CRLF wholesale — documented simplification.)
pub fn detect_eol(s: &str) -> &'static str {
    if s.contains("\r\n") { "\r\n" } else { "\n" }
}

/// Normalize CRLF (and stray CR) to LF so matching sees what `read` showed.
pub fn normalize_to_lf(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// Restore the file's native ending after a successful LF-space replacement.
pub fn restore_eol(lf_text: &str, eol: &str) -> String {
    if eol == "\r\n" {
        lf_text.replace('\n', "\r\n")
    } else {
        lf_text.to_string()
    }
}

/// Run the full chain against LF-normalized inputs.
pub fn plan(content: &str, old: &str, new: &str, replace_all: bool) -> EditOutcome {
    let mut first_ambiguity: Option<usize> = None;
    for (strategy, run) in [
        (
            MatchStrategy::Exact,
            replace_exact as fn(&str, &str, &str, bool) -> Result<Option<(String, usize)>, usize>,
        ),
        (MatchStrategy::LineTrimmed, replace_line_trimmed),
        (MatchStrategy::IndentationFlexible, replace_indent_flexible),
    ] {
        match run(content, old, new, replace_all) {
            Ok(Some((output, count))) => {
                return EditOutcome::Applied {
                    output,
                    count,
                    strategy,
                };
            }
            Ok(None) => continue, // stage found nothing; try the next one
            Err(n) => {
                // Higher-priority stages matched multiple times: that IS the
                // answer — later (looser) stages can only be at least as
                // ambiguous, so report and stop.
                first_ambiguity = Some(n);
                break;
            }
        }
    }
    match first_ambiguity {
        Some(n) => EditOutcome::Ambiguous(n),
        None => EditOutcome::NotFound,
    }
}

/// Byte-exact replacement. `Err(count)` = ambiguous, `Ok(None)` = absent.
fn replace_exact(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<Option<(String, usize)>, usize> {
    let count = content.matches(old).count();
    match count {
        0 => Ok(None),
        n if n > 1 && !replace_all => Err(n),
        _ => {
            let output = if replace_all {
                content.replace(old, new)
            } else {
                content.replacen(old, new, 1)
            };
            Ok(Some((output, count)))
        }
    }
}

/// Per-line matcher shared by the two loose stages: finds windows of
/// `old_lines.len()` consecutive content lines where `eq` holds pairwise.
/// Returns (start indices, ...) for every match.
fn find_line_windows<F>(content_lines: &[&str], old_lines: &[&str], eq: F) -> Vec<usize>
where
    F: Fn(&str, &str) -> bool,
{
    if old_lines.is_empty() || old_lines.len() > content_lines.len() {
        return Vec::new();
    }
    (0..=content_lines.len() - old_lines.len())
        .filter(|&start| {
            content_lines[start..start + old_lines.len()]
                .iter()
                .zip(old_lines.iter())
                .all(|(c, o)| eq(c, o))
        })
        .collect()
}

/// Splice `new_lines` over each matched window (right-to-left so earlier
/// indices stay valid) and rebuild the body.
fn replace_line_trimmed(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<Option<(String, usize)>, usize> {
    // opencode `lineTrimmedMatch`: ignore TRAILING whitespace per line;
    // leading whitespace still counts (indentation errors are the next stage's job).
    replace_by_lines(content, old, new, replace_all, |c, o| {
        c.trim_end() == o.trim_end()
    })
}

fn replace_indent_flexible(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<Option<(String, usize)>, usize> {
    replace_by_lines(content, old, new, replace_all, |c, o| c.trim() == o.trim())
}

fn replace_by_lines<F>(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
    eq: F,
) -> Result<Option<(String, usize)>, usize>
where
    F: Fn(&str, &str) -> bool,
{
    let content_lines: Vec<&str> = content.split('\n').collect();
    let old_lines: Vec<&str> = old.split('\n').collect();
    // An empty last element means old ended with '\n' — keep it significant so
    // multi-line anchors don't accidentally match one line short.
    let starts = find_line_windows(&content_lines, &old_lines, eq);
    match starts.len() {
        0 => Ok(None),
        n if n > 1 && !replace_all => Err(n),
        _ => {
            let chosen = if replace_all {
                starts.as_slice()
            } else {
                &starts[..1]
            };
            Ok(Some((
                splice_windows(&content_lines, chosen, old_lines.len(), new),
                chosen.len(),
            )))
        }
    }
}

/// Replace each `[start, start+width)` window with `new`, rebuilding with '\n'
/// joins plus a trailing newline exactly when the ORIGINAL content had one.
fn splice_windows(content_lines: &[&str], starts: &[usize], width: usize, new: &str) -> String {
    let ended_with_newline = content_lines.last().is_some_and(|l| l.is_empty());
    // Body without the sentinel empty tail (if any); windows index this slice.
    let core_len = if ended_with_newline {
        content_lines.len() - 1
    } else {
        content_lines.len()
    };
    let core = &content_lines[..core_len];

    let mut out: Vec<&str> = Vec::with_capacity(core_len);
    let mut cursor = 0usize;
    for &start in starts {
        let start = start.min(core_len);
        out.extend(core[cursor..start].iter().copied());
        out.extend(new.split('\n'));
        cursor = start + width;
    }
    out.extend(core[cursor.min(core_len)..].iter().copied());

    let mut text = out.join("\n");
    if ended_with_newline {
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_on_lf_files_is_unchanged_behavior() {
        let out = plan("a\nb\nc\n", "b", "B", false);
        assert_eq!(
            out,
            EditOutcome::Applied {
                output: "a\nB\nc\n".into(),
                count: 1,
                strategy: MatchStrategy::Exact
            }
        );
    }

    #[test]
    fn ambiguous_exact_reports_count_even_when_loose_stage_unique() {
        // Two byte-exact occurrences; looseness can only widen the set.
        assert_eq!(plan("x\nx\n", "x", "y", false), EditOutcome::Ambiguous(2));
    }

    #[test]
    fn replace_all_hits_every_occurrence() {
        let out = plan("x\nx\n", "x", "y", true);
        assert_eq!(
            out,
            EditOutcome::Applied {
                output: "y\ny\n".into(),
                count: 2,
                strategy: MatchStrategy::Exact
            }
        );
    }

    #[test]
    fn trailing_whitespace_differences_still_match() {
        // Model copied trailing spaces from a rendering artifact.
        let out = plan(
            "fn a() {\n    let x = 1;\n}\n",
            "fn a() {\n    let x = 1;   \n}",
            "fn a() {\n    let x = 2;\n}",
            false,
        );
        assert_eq!(
            out,
            EditOutcome::Applied {
                output: "fn a() {\n    let x = 2;\n}\n".into(),
                count: 1,
                strategy: MatchStrategy::LineTrimmed
            }
        );
    }

    #[test]
    fn wrong_indentation_still_matches_flexibly() {
        // Replacement lands VERBATIM: the loose stages locate the region; they
        // do not restyle the new text (deliberate simplification vs pi's
        // indent-preserving diff — the result note tells the model it matched
        // loosely so it can verify).
        let out = plan(
            "loop {\n    tick();\n}\n",
            "loop {\n  tick();\n}",
            "loop {\n  tock();\n}",
            false,
        );
        assert_eq!(
            out,
            EditOutcome::Applied {
                output: "loop {\n  tock();\n}\n".into(),
                count: 1,
                strategy: MatchStrategy::IndentationFlexible
            }
        );
    }

    #[test]
    fn genuinely_different_text_is_not_found() {
        assert_eq!(
            plan("alpha\nbeta\n", "alpha\ngamma", "z", false),
            EditOutcome::NotFound
        );
    }

    #[test]
    fn eol_detection_and_restore_round_trip() {
        assert_eq!(detect_eol("a\r\nb"), "\r\n");
        assert_eq!(detect_eol("a\nb"), "\n");
        assert_eq!(restore_eol("1\n2", "\r\n"), "1\r\n2");
        assert_eq!(restore_eol("1\n2", "\n"), "1\n2");
    }

    #[test]
    fn window_splice_preserves_trailing_newline() {
        let out = plan("one\ntwo\nthree\nfour\n", "two\nthree", "TWO", false);
        assert_eq!(
            out,
            EditOutcome::Applied {
                output: "one\nTWO\nfour\n".into(),
                count: 1,
                strategy: MatchStrategy::Exact
            }
        );
    }
}
