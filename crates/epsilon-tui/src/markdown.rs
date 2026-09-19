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

/// Convert assistant markdown into styled transcript rows.
///
/// The theme comes in rather than being reached for: the palette is a runtime
/// decision (colour depth, `NO_COLOR`), and a renderer that reads it implicitly
/// cannot be tested in more than one of those modes.
pub fn md_to_lines(text: &str, theme: &Theme) -> Vec<Line<'static>> {
    // LaTeX: render math spans to unicode before markdown parsing (pi `markdown.ts` + `latex.ts`).
    let preprocessed = crate::latex::preprocess_math(text);
    let text: &str = preprocessed.as_str();
    use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

    let mut out: Vec<Line<'static>> = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = Vec::new();
    let mut in_code_block = false;
    let mut list_depth = 0usize;

    let top = |stack: &Vec<Style>| stack.last().copied().unwrap_or_else(|| theme.assistant());
    let flush = |spans: &mut Vec<Span<'static>>, out: &mut Vec<Line<'static>>| {
        out.push(Line::from(std::mem::take(spans)));
    };

    for ev in Parser::new_ext(text, Options::ENABLE_STRIKETHROUGH) {
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
                spans.push(Span::styled("❯ ".to_string(), theme.system()));
            }
            Event::Start(Tag::List(_)) => {
                list_depth += 1;
            }
            Event::End(TagEnd::List(_)) => {
                list_depth = list_depth.saturating_sub(1);
            }
            Event::Start(Tag::Item) => {
                spans.push(Span::styled(
                    "  • ".repeat(list_depth.max(1)),
                    theme.system(),
                ));
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
                if in_code_block {
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

    fn plain(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.to_string()).collect()
    }

    #[test]
    fn paragraphs_become_single_plain_rows() {
        let theme = Theme::detect();
        let lines = md_to_lines("hello world", &theme);
        assert_eq!(lines.len(), 1);
        assert_eq!(plain(&lines[0]), "hello world");
    }

    #[test]
    fn strong_emphasis_code_get_distinct_styles() {
        let theme = Theme::detect();
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
        let theme = Theme::detect();
        let lines = md_to_lines("# Title\n\nbody", &theme);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].style, theme.title());
        assert_eq!(plain(&lines[0]), "Title");
        assert_eq!(plain(&lines[1]), "body");
    }

    #[test]
    fn fenced_blocks_render_as_gutter_rows() {
        let theme = Theme::detect();
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
        let theme = Theme::detect();
        let lines = md_to_lines("- one\n- two\n  - nested", &theme);
        let bullets: Vec<String> = lines.iter().map(|l| plain(l)).collect();
        assert!(bullets[0].starts_with("  • "), "{bullets:?}");
        assert!(bullets.last().unwrap().contains("nested"), "{bullets:?}");
    }

    #[test]
    fn latex_math_renders_to_unicode_and_strips_delimiters() {
        let theme = Theme::detect();
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
}
