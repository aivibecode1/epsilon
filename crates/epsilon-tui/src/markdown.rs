//! Assistant-markdown → styled ratatui `Line`s (pi's `markdown.ts` component, ported
//! to a pull-parser model via pulldown-cmark).
//!
//! Scope: headings (accent+bold), emphasis/strong/inline-code spans, fenced blocks as
//! dim gutter rows, bullet lists with `•`, blockquotes prefixed `❯`. Everything else
//! renders as plain text lines. This is display polish — the MODEL still receives raw
//! text; only the transcript pane interprets markdown.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::ui::theme::Theme;

/// A GFM table being collected, laid out when `End(Table)` arrives.
///
/// Ported from pi's `renderTable` (`tui/src/components/markdown.ts:842`),
/// including the decision that matters most: **a table too narrow to lay out
/// falls back to readable plain rows** rather than emitting columns one
/// character wide, which is unreadable and reads as a rendering bug.
///
/// The width budget is pi's: `| ` + (n-1)*` | ` + ` |` costs `3n + 1` columns of
/// border, and whatever is left belongs to the cells.
#[derive(Default)]
struct TableState {
    /// Rows of cell text, collected per row. Row 0 is the header.
    rows: Vec<Vec<String>>,
    /// Cell text for the row currently being collected.
    current: Vec<String>,
    /// Text of the cell being collected.
    cell: String,
}

impl TableState {
    fn end_row(&mut self) {
        if self.current.is_empty() {
            return;
        }
        self.rows.push(std::mem::take(&mut self.current));
    }

    /// Lay the table out, or return nothing when there is nothing to draw.
    fn render(self, theme: &Theme, width: usize) -> Vec<Line<'static>> {
        if self.rows.is_empty() {
            return Vec::new();
        }
        let cols = self.rows.iter().map(Vec::len).max().unwrap_or(0);
        if cols == 0 {
            return Vec::new();
        }
        // pi: border overhead is 3n+1; below one column per cell a table cannot be
        // drawn stably.
        let overhead = 3 * cols + 1;
        if width <= overhead + cols {
            return self.fallback(theme);
        }
        let avail = width - overhead;

        // Natural width per column, capped so one long cell cannot starve the
        // rest; the remainder is shared proportionally.
        let mut widths: Vec<usize> = vec![1; cols];
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                let w = display_width(cell);
                if w > widths[i] {
                    widths[i] = w;
                }
            }
        }
        let total: usize = widths.iter().sum();
        if total > avail {
            // Shrink proportionally, never below 1, so no column vanishes.
            let mut scaled: Vec<usize> = widths
                .iter()
                .map(|w| ((*w as f64 / total as f64) * avail as f64).floor().max(1.0) as usize)
                .collect();
            let mut used: usize = scaled.iter().sum();
            // Give the leftover back to the widest column, one at a time.
            let mut i = 0;
            while used < avail && cols > 0 {
                scaled[i % cols] += 1;
                used += 1;
                i += 1;
            }
            widths = scaled;
        }

        let mut out = Vec::new();
        let rule = "\u{2500}".repeat(avail + 2);
        for (r, row) in self.rows.iter().enumerate() {
            let is_header = r == 0;
            let mut line = String::from("\u{2502} ");
            for (i, &w) in widths.iter().enumerate() {
                if i > 0 {
                    line.push_str(" \u{2502} ");
                }
                let cell = row.get(i).map(String::as_str).unwrap_or("");
                let text = truncate_to_width(cell, w);
                line.push_str(&text);
                for _ in display_width(&text)..w {
                    line.push(' ');
                }
            }
            line.push_str(" \u{2502}");
            // A table opens with a border, so the rule comes BEFORE the header
            // row. Drawing it after made the header read as a caption to the
            // body instead of as the column names.
            if is_header {
                out.push(Line::from(Span::styled(
                    format!("\u{250c}{rule}\u{2510}"),
                    theme.system(),
                )));
            }
            let style = if is_header {
                theme.title()
            } else {
                theme.assistant()
            };
            out.push(Line::from(Span::styled(line, style)));
        }
        out
    }

    /// A table too narrow for columns, as readable `a | b` rows.
    ///
    /// The raw source is not available here (pulldown-cmark has already parsed
    /// it), so the cells are rejoined with ` | ` -- the same text, in the order
    /// it was written, without pretending to align columns it cannot fit.
    fn fallback(self, theme: &Theme) -> Vec<Line<'static>> {
        self.rows
            .iter()
            .map(|row| {
                let joined = row.join(" | ");
                Line::from(Span::styled(joined, theme.assistant()))
            })
            .collect()
    }
}

/// Display width of a string, counting the characters a terminal actually shows.
fn display_width(s: &str) -> usize {
    crate::ui::layout::display_width(s)
}

/// Truncate to `w` display columns, appending an ellipsis when anything was cut.
///
/// This was a fourth hand-rolled cut that walked `char`s summing
/// `UnicodeWidthChar::width`, which is the arithmetic that splits an emoji ZWJ sequence in
/// half — see [`crate::wrap::prefix_at_width`] for the measured disagreement between the
/// two width rules. It now delegates to the one grapheme-aware implementation in the
/// crate, which already measures the ellipsis it appends.
fn truncate_to_width(s: &str, w: usize) -> String {
    crate::wrap::fit_width(s, w)
}

/// Convert assistant markdown into styled transcript rows.
///
/// The theme comes in rather than being reached for: the palette is a runtime
/// decision (colour depth, `NO_COLOR`), and a renderer that reads it implicitly
/// cannot be tested in more than one of those modes.
pub fn md_to_lines(text: &str, theme: &Theme) -> Vec<Line<'static>> {
    md_to_lines_in(text, theme, 80)
}

/// As [`md_to_lines`], with the terminal width a table may use.
///
/// The width only affects tables; everything else wraps downstream.
pub fn md_to_lines_in(text: &str, theme: &Theme, terminal_width: usize) -> Vec<Line<'static>> {
    // LaTeX: render math spans to unicode before markdown parsing (pi `markdown.ts` + `latex.ts`).
    let preprocessed = crate::latex::preprocess_math(text);
    let text: &str = preprocessed.as_str();
    use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

    let mut out: Vec<Line<'static>> = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = Vec::new();
    let mut in_code_block = false;
    let mut list_depth = 0usize;
    // A table's cells arrive as inline events between Start(Table)/End(Table), and
    // a cell is a `|`-delimited slice of them. Collect each cell's rendered text,
    // then lay the row out on End(TableHead)/End(TableRow).
    let mut table: Option<TableState> = None;
    // GFM task lists: pulldown-cmark emits the marker on the paragraph INSIDE the
    // list item, so the flag is set per item and consumed by the next text event.
    let mut pending_task: Option<bool> = None;

    let top = |stack: &Vec<Style>| stack.last().copied().unwrap_or_else(|| theme.assistant());
    // Pushes a line only when there is something to push. The table arms call
    // this at cell and row boundaries, where spans is always empty; pushing
    // unconditionally put a blank line in front of every table, which is what
    // the first version did.
    let flush = |spans: &mut Vec<Span<'static>>, out: &mut Vec<Line<'static>>| {
        if spans.is_empty() {
            return;
        }
        out.push(Line::from(std::mem::take(spans)));
    };

    // M196: GFM tables and task lists, and strikethrough which was ENABLED but had
    // no match arm -- a setting that claimed to work and silently did not, which is
    // this repository's `app.clear` defect in a different costume. All three are on
    // because all three are handled below; an option with no arm is a lie in the
    // source. pi renders tables in full (`markdown.ts:842`), so parity is the goal
    // rather than a subset.
    const MD_OPTIONS: Options = Options::ENABLE_STRIKETHROUGH
        .union(Options::ENABLE_TABLES)
        .union(Options::ENABLE_TASKLISTS);
    for ev in Parser::new_ext(text, MD_OPTIONS) {
        match ev {
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => flush(&mut spans, &mut out),
            Event::Start(Tag::Heading { level, .. }) => {
                style_stack.push(match level {
                    // H1/H2 carry weight; deeper headings are labels, not titles.
                    HeadingLevel::H1 | HeadingLevel::H2 => theme.title(),
                    _ => theme.dim_strong(),
                });
            }
            Event::End(TagEnd::Heading(_)) => {
                style_stack.pop();
                flush(&mut spans, &mut out);
            }
            Event::Start(Tag::Emphasis) => {
                style_stack.push(top(&style_stack).add_modifier(Modifier::ITALIC))
            }
            Event::End(TagEnd::Emphasis) => {
                style_stack.pop();
            }
            Event::Start(Tag::Strong) => {
                style_stack.push(top(&style_stack).add_modifier(Modifier::BOLD))
            }
            Event::End(TagEnd::Strong) => {
                style_stack.pop();
            }
            Event::Start(Tag::BlockQuote(_)) => {
                // **From the repertoire table, not a literal (A43).** `❯` is not in
                // CP437, and this is *model* output — the densest region of the whole
                // application, and the one place a model's own text is rendered
                // verbatim. A box here reads as a defect in the model's answer.
                spans.push(Span::styled(
                    format!("{} ", theme.glyphs.quote),
                    theme.system(),
                ));
            }
            Event::Start(Tag::List(_)) => {
                list_depth += 1;
            }
            Event::End(TagEnd::List(_)) => {
                list_depth = list_depth.saturating_sub(1);
            }
            // M196: there was no `End(Item)` arm at all, so every item in a list
            // was appended to the same line -- "- a - b" on one row. Pre-existing,
            // and only visible once a test looked at multi-item output.
            Event::End(TagEnd::Item) => flush(&mut spans, &mut out),
            Event::Start(Tag::Item) => {
                // A task-list item's bullet becomes a checkbox, so the marker
                // emitted for the item is consumed on the first text event
                // instead of being pushed here.
                if pending_task.is_none() {
                    spans.push(Span::styled(
                        "  • ".repeat(list_depth.max(1)),
                        theme.system(),
                    ));
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code_block = true;
                if let CodeBlockKind::Fenced(lang) = kind
                    && !lang.is_empty()
                {
                    out.push(Line::from(Span::styled(
                        format!("··· {lang}"),
                        theme.code_block(),
                    )));
                } else {
                    out.push(Line::from(Span::styled("···", theme.code_block())));
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                out.push(Line::from(Span::styled("···", theme.code_block())));
            }
            Event::Text(t) => {
                // The task marker precedes the item's text; this is where it is
                // finally used, so `[-] item` renders as a checkbox rather than
                // a bullet followed by a literal "[ ]".
                if pending_task.is_some() {
                    let depth = list_depth.max(1);
                    let mark = if pending_task.take() == Some(true) {
                        "[x] "
                    } else {
                        "[ ] "
                    };
                    spans.push(Span::styled(
                        format!("{}{mark}", "  ".repeat(depth)),
                        theme.system(),
                    ));
                }
                // Inside a table cell, text accumulates into the cell rather than
                // becoming a row of its own. The `else` below is the ordinary
                // path, so the two cannot drift.
                if let Some(st) = table.as_mut() {
                    st.cell.push_str(&t);
                } else if in_code_block {
                    for line in t.lines() {
                        out.push(Line::from(Span::styled(
                            format!("│ {line}"),
                            theme.code_block(),
                        )));
                    }
                } else {
                    spans.push(Span::styled(t.to_string(), top(&style_stack)));
                }
            }
            Event::Code(c) => {
                spans.push(Span::styled(format!(" {c} "), theme.code()));
            }
            Event::SoftBreak | Event::HardBreak => flush(&mut spans, &mut out),
            Event::Rule => out.push(Line::from(Span::styled("─".repeat(24), theme.system()))),
            // The arm `ENABLE_STRIKETHROUGH` was enabled for and never had.
            Event::Start(Tag::Strikethrough) => {
                style_stack.push(top(&style_stack).add_modifier(Modifier::CROSSED_OUT))
            }
            Event::End(TagEnd::Strikethrough) => {
                style_stack.pop();
            }
            // GFM task list: the marker is on the first text event of the item.
            Event::TaskListMarker(checked) => pending_task = Some(checked),
            Event::Start(Tag::Table(_)) => {
                flush(&mut spans, &mut out);
                table = Some(TableState::default());
            }
            // A cell boundary ends the cell. GFM tables emit `TableCell` per
            // cell, so the End arm is the reliable place to close one.
            Event::End(TagEnd::TableCell) => {
                if let Some(st) = table.as_mut() {
                    let text = std::mem::take(&mut st.cell);
                    st.current.push(text.trim().to_string());
                }
            }
            // A new row begins with an empty cell list. Without this the
            // second body row's cells were appended to the first row's, and
            // only one body row came out.
            Event::Start(Tag::TableRow) | Event::Start(Tag::TableHead) => {
                if let Some(st) = table.as_mut() {
                    st.current.clear();
                }
            }
            Event::End(TagEnd::Table) => {
                flush(&mut spans, &mut out);
                if let Some(st) = table.take() {
                    out.extend(st.render(theme, terminal_width));
                }
            }
            Event::End(TagEnd::TableHead) | Event::End(TagEnd::TableRow) => {
                if let Some(st) = table.as_mut() {
                    st.end_row();
                }
                flush(&mut spans, &mut out);
            }
            // A cell boundary also ends the cell, which is where the text goes.
            Event::Start(Tag::TableCell) => {
                if let Some(st) = table.as_mut() {
                    st.cell.clear();
                }
            }
            _ => {}
        }
    }
    // Trailing paragraph without an End event is impossible per spec, but flush anyway.
    if !spans.is_empty() {
        flush(&mut spans, &mut out);
    }
    if out.is_empty() {
        out.push(Line::from(String::new()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::{ColorMode, UNICODE};

    fn plain(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.to_string()).collect()
    }

    /// A theme with every role filled in, chosen by the test rather than by the
    /// room it is run in.
    ///
    /// `Theme::detect()` reads `NO_COLOR`, `COLORTERM` and `EPSILON_ASCII` from
    /// whatever environment the runner happens to have. That is right for the
    /// product and wrong for a test, and the cost was not hypothetical: every
    /// test in this module used `detect()`, and `a_table_becomes_columns` asserts
    /// that the header row is *visibly distinct* from the body. With `NO_COLOR`
    /// set — which this repository's own machine sets — every role collapses to
    /// `Style::new()`, the two styles are equal, and the test fails for a reason
    /// that has nothing to do with the code under test. A test whose verdict
    /// depends on an environment variable is a test that lies on half the
    /// machines that run it.
    ///
    /// `app/golden.rs` reached the same conclusion independently and says why in
    /// its own header; this is that decision applied to the module that needed it
    /// more, because it is the one that compares *two* styles for difference
    /// rather than comparing one against the theme that produced it.
    fn theme() -> Theme {
        Theme::new(ColorMode::Truecolor, UNICODE)
    }

    #[test]
    fn paragraphs_become_single_plain_rows() {
        let theme = theme();
        let lines = md_to_lines("hello world", &theme);
        assert_eq!(lines.len(), 1);
        assert_eq!(plain(&lines[0]), "hello world");
    }

    #[test]
    fn strong_emphasis_code_get_distinct_styles() {
        let theme = theme();
        let lines = md_to_lines("**bold** and `code` end", &theme);
        assert_eq!(lines.len(), 1);
        let l = &lines[0];
        assert_eq!(l.spans[0].style.add_modifier, Modifier::BOLD);
        assert_eq!(l.spans[0].content.to_string(), "bold");
        assert!(plain(l).contains(" and "));
        let code_span = l.spans.iter().find(|s| s.style == theme.code());
        assert!(code_span.is_some(), "inline code themed: {:?}", l.spans);
    }

    #[test]
    fn headings_use_accent_bold() {
        let theme = theme();
        let lines = md_to_lines("# Title\n\nbody", &theme);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].style, theme.title());
        assert_eq!(plain(&lines[0]), "Title");
        assert_eq!(plain(&lines[1]), "body");
    }

    #[test]
    fn fenced_blocks_render_as_gutter_rows() {
        let theme = theme();
        let lines = md_to_lines("```rust\nlet x = 1;\n```", &theme);
        assert!(lines[0].spans[0].content.contains("rust"));
        assert_eq!(lines[1].spans[0].content.to_string(), "│ let x = 1;");
        assert_eq!(lines.last().unwrap().spans[0].content.to_string(), "···");
        assert!(lines.iter().all(|l| {
            l.spans
                .iter()
                .all(|s| s.style == theme.code_block() || s.content.starts_with("···"))
        }));
    }

    #[test]
    fn lists_get_bullets_and_nested_indent() {
        let theme = theme();
        let lines = md_to_lines("- one\n- two\n  - nested", &theme);
        let bullets: Vec<String> = lines.iter().map(|l| plain(l)).collect();
        assert!(bullets[0].starts_with("  • "), "{bullets:?}");
        assert!(bullets.last().unwrap().contains("nested"), "{bullets:?}");
    }

    fn all(line: &[Line<'_>]) -> String {
        line.iter().map(plain).collect::<Vec<_>>().join("\n")
    }

    /// A GFM table becomes columns, not run-together text.
    ///
    /// Before M196 the parser never enabled tables, so `a | b` arrived as one
    /// paragraph and the pipes were the only clue it had been a table.
    #[test]
    fn a_table_becomes_columns() {
        let theme = theme();
        let lines = md_to_lines_in(
            "| name | role |\n|------|------|\n| ada | author |\n| alan | pioneer |",
            &theme,
            80,
        );
        let text = all(&lines);
        assert!(text.contains('│'), "cells are separated by a rule: {text}");
        assert!(text.contains("name") && text.contains("author"), "{text}");
        // The header must be visibly distinct, or a reader cannot tell which row
        // is which.
        let header_style = lines[0].spans[0].style;
        let body_style = lines
            .iter()
            .skip(1)
            .find(|l| plain(l).contains("ada"))
            .map(|l| l.spans[0].style)
            .expect("a body row");
        assert_ne!(header_style, body_style, "the header row is styled: {text}");
    }

    /// Every rendered row is the same width, so the columns line up.
    #[test]
    fn table_rows_are_the_same_width() {
        let theme = theme();
        let lines = md_to_lines_in(
            "| short | a much longer header |\n|---|---|\n| x | y |\n| longer value | z |",
            &theme,
            60,
        );
        // Only the data rows: the border is a different shape by construction
        // (it wraps the row with corners), so including it would be comparing
        // two things that are meant to differ.
        let widths: Vec<usize> = lines
            .iter()
            .map(plain)
            .filter(|s| s.starts_with('│'))
            .map(|s| s.chars().count())
            .collect();
        assert!(widths.len() >= 3, "header and two body rows: {widths:?}");
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "every data row shares one width: {widths:?}"
        );
    }

    /// Too narrow to lay out, and the honest response is readable text rather
    /// than one-character columns. This is pi's fallback (`markdown.ts:855-865`).
    #[test]
    fn a_table_too_narrow_falls_back_to_readable_rows() {
        let theme = theme();
        let lines = md_to_lines_in(
            "| a | b | c | d | e |\n|---|---|---|---|---|\n| 1 | 2 | 3 | 4 | 5 |",
            &theme,
            8,
        );
        let text = all(&lines);
        assert!(
            !text.contains('│'),
            "no column rules when there is no room for columns: {text}"
        );
        assert!(
            text.contains("a | b"),
            "the cells are still readable: {text}"
        );
    }

    /// A task list renders a checkbox, not a literal `[ ]` after a bullet.
    #[test]
    fn a_task_list_renders_a_checkbox() {
        let theme = theme();
        let text = all(&md_to_lines_in("- [x] done\n- [ ] todo", &theme, 80));
        assert!(text.contains("[x]") && text.contains("[ ]"), "{text}");
        assert!(
            !text.contains("• ["),
            "the bullet is replaced by the checkbox, not prefixed to it: {text}"
        );
    }

    /// The option was ENABLED before M196 and had no arm, so `~~gone~~` printed
    /// with its tildes and no strike. An option with no handler is a lie in the
    /// source, which is what this test names.
    #[test]
    fn strikethrough_actually_strikes() {
        let theme = theme();
        let lines = md_to_lines("~~gone~~", &theme);
        let struck = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .any(|s| s.style.add_modifier.contains(Modifier::CROSSED_OUT));
        assert!(struck, "the text carries a strike: {:?}", lines[0].spans);
        assert!(!plain(&lines[0]).contains('~'), "and the tildes are gone");
    }

    /// Each list item is its own line.
    ///
    /// There was no `End(Item)` arm, so `- a` and `- b` rendered as one row
    /// reading "- a - b". A single-item list could not show it, which is why
    /// every earlier test passed.
    #[test]
    fn list_items_are_separate_lines() {
        let theme = theme();
        let text = all(&md_to_lines("- alpha\n- beta\n- gamma", &theme));
        assert_eq!(text.lines().count(), 3, "one line per item: {text}");
        for item in ["alpha", "beta", "gamma"] {
            assert!(
                text.lines().any(|l| l.ends_with(item)),
                "{item} ends its own line: {text}"
            );
        }
    }

    #[test]
    fn latex_math_renders_to_unicode_and_strips_delimiters() {
        let theme = theme();
        let lines = md_to_lines(r"Let $\alpha$ and $$\beta = \gamma$$ end", &theme);
        let joined = lines
            .iter()
            .map(|l| plain(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains('α'), "alpha -> unicode: {joined}");
        assert!(
            joined.contains('β') && joined.contains('γ'),
            "beta/gamma: {joined}"
        );
        assert!(!joined.contains('$'), "delimiters stripped: {joined}");
        assert!(!joined.contains(r"\alpha"), "raw latex gone: {joined}");
    }

    /// **A table cell holding an emoji or a CJK string is cut whole and padded right.**
    ///
    /// The table renderer truncates each cell to its column and then pads with spaces
    /// counted by `display_width(&text)..w`. When truncation walked `char`s, an emoji cell
    /// was cut mid-cluster *and* the padding arithmetic disagreed with the width actually
    /// emitted, so the row came out a different width from its neighbours — the columns
    /// stopped lining up, which is the whole point of a table.
    ///
    /// This is the frame-level witness: the rows are compared by **display width**, not by
    /// character count, because `"👨‍👩‍👧‍👦".chars().count()` is 4 and `│` is 1 while the pair
    /// occupies 3 columns together.
    #[test]
    fn a_table_cell_with_an_emoji_or_cjk_is_cut_whole_and_padded_right() {
        use unicode_width::UnicodeWidthStr;
        let theme = theme();
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
        let md = format!(
            "| who | note |\n|---|---|\n| {family}{family} | a |\n| \u{65E5}\u{672C}\u{8A9E} | b |"
        );
        let lines = md_to_lines_in(&md, &theme, 40);
        let widths: Vec<usize> = lines
            .iter()
            .map(plain)
            .filter(|s| s.starts_with('\u{2502}'))
            .map(|s| UnicodeWidthStr::width(s.as_str()))
            .collect();
        assert!(widths.len() >= 3, "header and two body rows: {widths:?}");
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "every row is the same number of COLUMNS, whatever the script: {widths:?}"
        );
        // And no row carries a cluster cut in half: the family emoji appears whole or not
        // at all, so a stray zero-width joiner is the defect.
        for line in &lines {
            let text = plain(line);
            assert!(
                !text.contains('\u{200D}') || text.contains(family),
                "a ZWJ survived without its sequence: {text:?}"
            );
        }
    }
}
