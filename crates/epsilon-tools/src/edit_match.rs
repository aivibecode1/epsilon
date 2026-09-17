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
//! 5. **Escape-normalized** match: the model wrote `\n`/`\t` escapes as literal
//!    text instead of the characters they stand for.
//! 6. **Trimmed-boundary** match: the model included the blank lines around the
//!    block it meant.
//! 7. **Block-anchor** match (M71): first and last lines match *approximately*
//!    (similarity ≥ 0.65) and the region between them is replaced. This is the
//!    only stage that can match a *different number* of lines than `old_string`,
//!    which is why it is also the only stage the disproportionality guard can
//!    fire on — a loose anchor match over a huge span is refused rather than
//!    applied, because "matched loosely" is the same thing as "guessed".
//!
//! Stages run in ascending order of risk (exact first, similarity-based last), which
//! differs from opencode's ordering (it interleaves by observed frequency): when two
//! stages would both match, the safer one wins.
//!
//! Occurrence-count safety (exactly-once unless `replace_all`) applies to EVERY
//! stage, not just exact matching.

/// Which stage of the chain produced the replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchStrategy {
    /// Byte-for-byte after EOL normalization.
    Exact,
    /// Per-line equality after trimming both ends of every line.
    LineTrimmed,
    /// Per-line equality after stripping leading whitespace and trimming.
    IndentationFlexible,
    /// Literal `\n`/`\t` escapes in `old_string` were unescaped before matching.
    EscapeNormalized,
    /// Blank lines around the block were ignored.
    TrimmedBoundary,
    /// First and last lines matched approximately; the region between was replaced.
    BlockAnchor,
}

impl MatchStrategy {
    /// Human-readable note for the tool result (empty for the exact path).
    pub fn note(self) -> &'static str {
        match self {
            Self::Exact => "",
            Self::LineTrimmed => " (matched after ignoring trailing whitespace)",
            Self::IndentationFlexible => " (matched after ignoring indentation differences)",
            Self::EscapeNormalized => " (matched after unescaping \\n/\\t in old_string)",
            Self::TrimmedBoundary => " (matched after ignoring blank lines around the block)",
            Self::BlockAnchor => " (matched on the first and last lines only — check the result)",
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
    /// A loose stage matched a span far larger than `old_string` — refused rather
    /// than applied (opencode `isDisproportionateMatch`, `tool/edit.ts:731-737`).
    Suspicious {
        /// Lines the loose stage matched.
        span_lines: usize,
        /// Lines in `old_string`.
        old_lines: usize,
        /// Extra characters the matched span carries over `old_string`.
        extra_chars: usize,
    },
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
///
/// Stages are ordered by risk: an exact match always wins over a loose one, so a file
/// that contains the text verbatim is never "found approximately" somewhere else first.
pub fn plan(content: &str, old: &str, new: &str, replace_all: bool) -> EditOutcome {
    // Fixed-width stages first: each compares a window exactly `old_string` tall, so none of
    // them can match more text than it was given.
    let mut first_ambiguity: Option<usize> = None;
    for (strategy, run) in [
        (
            MatchStrategy::Exact,
            replace_exact as fn(&str, &str, &str, bool) -> Result<Option<(String, usize)>, usize>,
        ),
        (MatchStrategy::LineTrimmed, replace_line_trimmed),
        (MatchStrategy::IndentationFlexible, replace_indent_flexible),
        (MatchStrategy::EscapeNormalized, replace_escape_normalized),
        (MatchStrategy::TrimmedBoundary, replace_trimmed_boundary),
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
                // Higher-priority stages matched multiple times: that IS the answer — a
                // looser stage can only match at least as much, so report and stop.
                first_ambiguity = Some(n);
                break;
            }
        }
    }
    if let Some(n) = first_ambiguity {
        return EditOutcome::Ambiguous(n);
    }

    // The one variable-width stage, last because it is the loosest: it can match a region
    // of a different height than `old_string`, and therefore has its own refusals.
    match replace_block_anchor(content, old, new, replace_all) {
        Ok(Some((output, count))) => EditOutcome::Applied {
            output,
            count,
            strategy: MatchStrategy::BlockAnchor,
        },
        Ok(None) => EditOutcome::NotFound,
        Err(AnchorRefusal::Ambiguous(n)) => EditOutcome::Ambiguous(n),
        Err(AnchorRefusal::Suspicious {
            span_lines,
            old_lines,
            extra_chars,
        }) => EditOutcome::Suspicious {
            span_lines,
            old_lines,
            extra_chars,
        },
    }
}

/// How the block-anchor stage can decline.
enum AnchorRefusal {
    /// More than one candidate region.
    Ambiguous(usize),
    /// One region, but implausibly large for the text that was asked for.
    Suspicious {
        span_lines: usize,
        old_lines: usize,
        extra_chars: usize,
    },
}

/// Both anchor lines must be at least this similar to the model's own first/last line.
/// (opencode's threshold, `tool/edit.ts:700`.)
const ANCHOR_SIMILARITY: f64 = 0.65;

/// Minimum height: with only two lines, "the region between the anchors" is nothing,
/// and the stage degenerates into matching lines that merely look similar.
const ANCHOR_MIN_LINES: usize = 3;

/// Match by first/last-line similarity, replace everything between them.
///
/// This exists for the case the fixed-width stages cannot serve: the model remembered a
/// block's opening and closing lines and paraphrased something in the middle. The risk is
/// the mirror image of that — two anchors far apart that were never a block — which the
/// disproportionality guard refuses.
fn replace_block_anchor(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<Option<(String, usize)>, AnchorRefusal> {
    let content_lines: Vec<&str> = content.split('\n').collect();
    let old_lines: Vec<&str> = old.split('\n').collect();
    if old_lines.len() < ANCHOR_MIN_LINES || old_lines.len() > content_lines.len() {
        return Ok(None);
    }
    let first = old_lines[0].trim();
    let last = old_lines[old_lines.len() - 1].trim();
    if first.is_empty() || last.is_empty() {
        return Ok(None);
    }

    // Candidate openings: lines that look like the model's first line.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for start in 0..content_lines.len() {
        // An empty line is never an anchor: `similarity("", first)` is far below the
        // threshold, so the check below covers it.
        if similarity(content_lines[start].trim(), first) < ANCHOR_SIMILARITY {
            continue;
        }
        // The nearest following line that looks like the closing anchor, so the span is
        // the smallest region the two anchors can bound.
        let end = ((start + ANCHOR_MIN_LINES - 1)..content_lines.len())
            .find(|&e| similarity(content_lines[e].trim(), last) >= ANCHOR_SIMILARITY);
        if let Some(end) = end {
            spans.push((start, end));
        }
    }
    match spans.len() {
        0 => Ok(None),
        n if n > 1 && !replace_all => Err(AnchorRefusal::Ambiguous(n)),
        _ => {
            let chosen = if replace_all {
                spans.as_slice()
            } else {
                &spans[..1]
            };
            // The guard applies to the pick we are about to make.
            let (start, end) = chosen[0];
            let span_lines = end - start + 1;
            let span_chars: usize = content_lines[start..=end].iter().map(|l| l.len() + 1).sum();
            let old_chars: usize = old.len();
            if !replace_all && disproportionate(span_lines, old_lines.len(), span_chars, old_chars)
            {
                return Err(AnchorRefusal::Suspicious {
                    span_lines,
                    old_lines: old_lines.len(),
                    extra_chars: span_chars.saturating_sub(old_chars),
                });
            }
            // Splice every chosen span; their heights may differ, which is exactly why
            // the fixed-width helper cannot serve here.
            let out = splice_variable(&content_lines, chosen, new);
            Ok(Some((out, chosen.len())))
        }
    }
}

/// Character-level similarity in `[0, 1]` — `1 - distance / longer_length`.
///
/// A hand-rolled Levenshtein over two single lines: the crate carries no algorithmic
/// dependencies and does not gain one for this.
fn similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    let longest = a.chars().count().max(b.chars().count());
    if longest == 0 {
        return 1.0;
    }
    let distance = levenshtein(a, b);
    1.0 - (distance as f64 / longest as f64)
}

/// Two-row Levenshtein distance over characters.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// The model wrote `\n`/`\t`/`\r` as literal two-character escapes.
fn replace_escape_normalized(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<Option<(String, usize)>, usize> {
    if !old.contains("\\n") && !old.contains("\\t") && !old.contains("\\r") {
        return Ok(None);
    }
    let unescaped = old
        .replace("\\r\\n", "\n")
        .replace("\\n", "\n")
        .replace("\\t", "\t")
        .replace("\\r", "\r");
    replace_exact(content, &unescaped, new, replace_all)
}

/// The model included the blank lines that surround the block it meant to replace.
fn replace_trimmed_boundary(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<Option<(String, usize)>, usize> {
    let lines: Vec<&str> = old.split('\n').collect();
    let lead = lines.iter().take_while(|l| l.trim().is_empty()).count();
    let trail = lines
        .iter()
        .rev()
        .take_while(|l| l.trim().is_empty())
        .count();
    if lead == 0 && trail == 0 {
        return Ok(None);
    }
    if lead + trail >= lines.len() {
        return Ok(None); // nothing but blank lines; not a block
    }
    let core = lines[lead..lines.len() - trail].join("\n");
    // Match the core exactly; if the file has the surrounding blanks, the exact stage
    // would have found the original first.
    replace_exact(content, &core, new, replace_all)
}

/// Is a loose match implausibly larger than what was asked for?
///
/// opencode's rule (`tool/edit.ts:731-737`): a span of `max(old+3, old*2)` lines or
/// more, or 500+ characters longer than `old_string`, is not a match — it is a
/// coincidence that would rewrite a block nobody asked to rewrite. The fixed-width
/// stages can never trip it (their window is exactly `old_string`'s height, asserted
/// by test); the block-anchor stage can, which is the point.
fn disproportionate(
    span_lines: usize,
    old_lines: usize,
    span_chars: usize,
    old_chars: usize,
) -> bool {
    let line_limit = std::cmp::max(old_lines + 3, old_lines * 2);
    span_lines >= line_limit || span_chars >= old_chars + 500
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

/// Replace variable-height line spans (inclusive ranges) with `new`, rebuilding with '\n'
/// joins plus a trailing newline exactly when the ORIGINAL content had one.
fn splice_variable(content_lines: &[&str], spans: &[(usize, usize)], new: &str) -> String {
    let ended_with_newline = content_lines.last().is_some_and(|l| l.is_empty());
    let core_len = if ended_with_newline {
        content_lines.len() - 1
    } else {
        content_lines.len()
    };
    let core = &content_lines[..core_len];

    // Left to right so the spans are consumed in order; each records where the gap ends.
    let mut out: Vec<&str> = Vec::with_capacity(core_len);
    let mut cursor = 0usize;
    for &(start, end) in spans {
        let start = start.min(core_len);
        let end = end.min(core_len.saturating_sub(1));
        if start < cursor {
            continue; // overlapping spans cannot happen, but never duplicate text
        }
        out.extend(core[cursor..start].iter().copied());
        out.extend(new.split('\n'));
        cursor = end + 1;
    }
    out.extend(core[cursor.min(core_len)..].iter().copied());

    let mut text = out.join("\n");
    if ended_with_newline {
        text.push('\n');
    }
    text
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

/// Byte-offset occurrences of `needle` in `hay` (exact, LF-normalized).
/// Returns (count, first_index). Used by batch edit to collect disjoint spans.
pub fn find_exact_occurrences(hay: &str, needle: &str) -> (usize, Option<usize>) {
    if needle.is_empty() {
        return (0, None);
    }
    let v: Vec<_> = hay.match_indices(needle).collect();
    let count = v.len();
    let first = v.first().map(|(i, _)| *i);
    (count, first)
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

    // ---- M71: the three new stages and the refusal --------------------------------------

    #[test]
    fn a_paraphrased_middle_matches_on_its_anchors() {
        // The model remembered the opening and closing lines and reworded the body — the
        // case the fixed-width stages cannot serve.
        let content = "impl Foo {\n    // does the thing\n    fn bar(&self) {}\n}\n";
        let old = "impl Foo {\n    // does the thing\n    fn bar(&self) {}\n}";
        let out = plan(content, old, "impl Foo {\n    fn baz(&self) {}\n}", false);
        // Exact first, as always.
        assert!(matches!(
            out,
            EditOutcome::Applied {
                strategy: MatchStrategy::Exact,
                ..
            }
        ));

        // Now the model's copy is *wrong* in the middle: anchors still hold.
        let old_typo = "impl Foo {\n    // does teh thing\n    fn bar(&self) {}\n}";
        match plan(
            content,
            old_typo,
            "impl Foo {\n    fn baz(&self) {}\n}",
            false,
        ) {
            EditOutcome::Applied {
                output,
                count,
                strategy,
            } => {
                assert_eq!(strategy, MatchStrategy::BlockAnchor);
                assert_eq!(count, 1);
                assert_eq!(output, "impl Foo {\n    fn baz(&self) {}\n}\n");
                assert!(
                    strategy.note().contains("check the result"),
                    "the model is told this match was loose: {}",
                    strategy.note()
                );
            }
            other => panic!("expected an anchor match, got {other:?}"),
        }
    }

    #[test]
    fn a_two_line_anchor_pair_is_not_enough_to_guess() {
        // Below the minimum height the stage does not run: "the region between the anchors"
        // would be nothing, and the match would rest on two similar-looking lines alone.
        let content = "fn alpha() {\n    body();\n}\n";
        let out = plan(content, "fn alpha() {\n    boddy();", "x", false);
        assert_eq!(out, EditOutcome::NotFound);
    }

    #[test]
    fn a_loose_match_over_a_huge_span_is_refused_not_applied() {
        // Opening and closing braces exist everywhere; anchoring on them must not swallow the
        // file. The region between `mod outer {` and its closing `}` is 40 lines for a 3-line
        // old_string.
        let mut content = String::from("mod outer {\n");
        for i in 0..38 {
            content.push_str(&format!("    fn f{i}() {{}}\n"));
        }
        content.push_str("}\n");
        let old = "mod outer {\n    fn different() {}\n}";
        match plan(&content, old, "mod outer {\n}", false) {
            EditOutcome::Suspicious {
                span_lines,
                old_lines,
                extra_chars,
            } => {
                assert_eq!(old_lines, 3);
                assert!(
                    span_lines >= 40,
                    "the whole module was in the span: {span_lines}"
                );
                assert!(extra_chars > 0, "{extra_chars}");
            }
            other => panic!("a 40-line span for 3 lines must be refused, got {other:?}"),
        }
    }

    #[test]
    fn the_refusal_covers_the_character_rule_as_well_as_the_line_rule() {
        // Four lines (below the line limit for a 3-line old_string, which is 6) but each line
        // is enormous, so the character rule is what fires.
        let old = "fn a() {\n    x\n}";
        let content = format!("fn a() {{\n{}\n{}\n}}\n", "y".repeat(400), "z".repeat(400));
        match plan(&content, old, "fn a() {\n}", false) {
            EditOutcome::Suspicious { extra_chars, .. } => {
                assert!(extra_chars >= 500, "extra_chars = {extra_chars}");
            }
            other => panic!("expected the character rule to fire, got {other:?}"),
        }
    }

    #[test]
    fn a_fixed_width_stage_reports_no_span_numbers_because_there_are_none() {
        // A whitespace-only line: `line-trimmed` matches it (a rendering artifact turned a
        // blank line into spaces), and its window is exactly as tall as old_string by
        // construction — so the refusal is structurally unreachable here, not merely unused.
        // (An earlier version of this test passed through the block-anchor stage instead,
        // which the M3 mutation exposed.)
        let content = "fn a() {\n   \n}\n";
        match plan(content, "fn a() {\n\n}", "fn a() {\n    done();\n}", false) {
            EditOutcome::Applied {
                strategy, output, ..
            } => {
                assert_eq!(strategy, MatchStrategy::LineTrimmed);
                assert_eq!(output, "fn a() {\n    done();\n}\n");
            }
            other => panic!("expected a fixed-width match, got {other:?}"),
        }
        // The structural claim, over a file 300 lines tall: a fixed-width stage can only
        // report the window it was given, so `Suspicious` is unreachable by construction.
        let mut tall = String::new();
        for i in 0..300 {
            tall.push_str(&format!("line {i}\n"));
        }
        tall.push_str("fn a() {\n}\n");
        for old in ["fn a() {\n\n}", "nothing like this exists\nanywhere\nhere"] {
            let out = plan(&tall, old, "x", false);
            assert!(
                !matches!(out, EditOutcome::Suspicious { .. }),
                "a fixed-width stage cannot refuse on span size: {out:?}"
            );
        }
    }

    #[test]
    fn anchors_that_are_only_superficially_alike_do_not_match() {
        // "fn main() -> Result<(), Error> {" vs "let x = 1;" is far below the threshold.
        let content = "fn main() -> Result<(), Error> {\n    let x = 1;\n    Ok(())\n}\n";
        let out = plan(
            content,
            "lets go of the whole thing\n    nothing like it\n}",
            "x",
            false,
        );
        assert_eq!(out, EditOutcome::NotFound);
    }

    #[test]
    fn escaped_newlines_in_the_arguments_match_real_newlines() {
        // A model that JSON-encoded twice: the file has real newlines, old_string has `\n`.
        let out = plan(
            "line one\nline two\nline three\n",
            "line one\\nline two",
            "LINE ONE\nLINE TWO",
            false,
        );
        match out {
            EditOutcome::Applied {
                output, strategy, ..
            } => {
                assert_eq!(strategy, MatchStrategy::EscapeNormalized);
                assert_eq!(output, "LINE ONE\nLINE TWO\nline three\n");
            }
            other => panic!("expected an escape-normalized match, got {other:?}"),
        }
    }

    #[test]
    fn blank_lines_around_the_block_are_ignored() {
        // The model's copy carries blank lines the file does not have (two before the block).
        // Every fixed-width stage fails: the payload is one line TALLER than what is there.
        let content = "fn a() {}\nfn b() {}\nfn c() {}\n";
        match plan(content, "\n\nfn b() {}", "fn B() {}", false) {
            EditOutcome::Applied {
                output,
                count,
                strategy,
            } => {
                assert_eq!(strategy, MatchStrategy::TrimmedBoundary);
                assert_eq!(count, 1);
                assert_eq!(
                    output, "fn a() {}\nfn B() {}\nfn c() {}\n",
                    "the blanks were not inserted; the block was replaced in place"
                );
                assert!(strategy.note().contains("blank lines"));
            }
            other => panic!("expected a boundary-trimmed match, got {other:?}"),
        }

        // Blanks at both ends work the same way. (One blank at the end would be a plain
        // substring — the file's own newline — and the exact stage is meant to win that.)
        match plan(content, "\n\nfn c() {}\n", "fn C() {}", false) {
            EditOutcome::Applied {
                strategy, output, ..
            } => {
                assert_eq!(strategy, MatchStrategy::TrimmedBoundary);
                assert_eq!(output, "fn a() {}\nfn b() {}\nfn C() {}\n");
            }
            other => panic!("expected a boundary-trimmed match, got {other:?}"),
        }

        // `new_string` is applied verbatim: a blank line the model wrote in it stays written
        // (the tool returns a diff, and the note says the match was loose — this stage
        // locates the region, it does not restyle the model's text).
        match plan(content, "\n\nfn b() {}", "fn B() {}\n", false) {
            EditOutcome::Applied { output, .. } => {
                assert_eq!(output, "fn a() {}\nfn B() {}\n\nfn c() {}\n");
            }
            other => panic!("expected a boundary-trimmed match, got {other:?}"),
        }

        // A payload that is nothing but blank lines is not a block.
        assert_eq!(plan("a\n", "\n\n", "x", false), EditOutcome::NotFound);
        // No blank lines to trim: the stage does not run at all.
        assert_eq!(
            plan("fn a() {}\n", "fn zz() {}", "x", false),
            EditOutcome::NotFound
        );
    }

    #[test]
    fn replace_all_applies_to_variable_height_spans() {
        // Two anchor matches of different heights; both are replaced, and the text between
        // and around them survives exactly once.
        // Both blocks open and close with the same lines, and they are different heights.
        let content = "mod A {\n    a1\n    a2\n}\nmid\nmod A {\n    b1\n}\nend\n";
        let old = "mod A {\n    different\n}";
        let out = plan(content, old, "MOD{}", true);
        match out {
            EditOutcome::Applied { output, count, .. } => {
                assert_eq!(count, 2, "both anchors matched: {output:?}");
                assert!(output.contains("MOD{}\nmid"), "{output:?}");
                assert!(output.ends_with("MOD{}\nend\n"), "{output:?}");
            }
            other => panic!("expected both spans to be replaced, got {other:?}"),
        }
    }

    #[test]
    fn ambiguity_still_wins_over_a_looser_stage() {
        // Two exact occurrences of a payload that also carries blank lines: the answer is
        // "ambiguous", not a guess from a looser stage.
        assert_eq!(
            plan("\nx\n\n\nx\n\n", "\nx\n", "y", false),
            EditOutcome::Ambiguous(2)
        );
        // And the anchor stage reports its own ambiguity rather than picking one.
        let doubled = "mod A {\n    one\n}\nmod A {\n    two\n}\n";
        assert_eq!(
            plan(doubled, "mod A {\n    neither\n}", "x", false),
            EditOutcome::Ambiguous(2)
        );
    }

    #[test]
    fn similarity_and_distance_are_what_they_claim() {
        assert_eq!(similarity("same", "same"), 1.0);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("", ""), 0);
        // 0.65 is the anchor threshold: 15 chars with 5 wrong is exactly at it, 4 wrong above.
        assert!(similarity("fn alpha() { // x", "fn alpha() { // y") > ANCHOR_SIMILARITY);
        assert!(similarity("fn alpha() { // x", "totally different line") < ANCHOR_SIMILARITY);
        // Multi-byte characters are counted as characters, not bytes.
        assert_eq!(levenshtein("héllo", "hello"), 1);
    }

    #[test]
    fn the_guard_boundary_is_inclusive_of_opencodes_rule() {
        // max(old+3, old*2): for a 3-line old_string the limit is 6 lines.
        assert!(!disproportionate(5, 3, 100, 100));
        assert!(disproportionate(6, 3, 100, 100));
        // ...and the character rule: 500+ characters longer.
        assert!(!disproportionate(4, 3, 599, 100));
        assert!(disproportionate(4, 3, 600, 100));
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
