//! Markdown, for the words the model wrote.
//!
//! # Why this exists and is not "the TUI's markdown renderer"
//!
//! The terminal frontend has one (`epsilon_tui::markdown`). **It is not portable**, and
//! pretending otherwise would be the same mistake as writing a third copy of the workspace
//! walk before M236 moved it to core: the two renderers share a *parser* and nothing else.
//! That parser is [`pulldown_cmark`], already in this workspace for the terminal; its
//! output is `Line`/`Span` over `ratatui::style::Style`, coupled to a width-aware table
//! layout and to the terminal's own `Theme`.
//!
//! So what is shared is the parser and the *contract* — markdown in, styled runs out —
//! and the render stays per-frontend. The rule both renderers keep is the one that matters:
//! when a construct cannot be laid out honestly, fall back to something readable rather
//! than to something that looks like a rendering bug.
//!
//! # Which text gets interpreted at all
//!
//! **Only the assistant's.** A user who types `**hi**` must see `**hi**`; a shell command
//! in the transcript must stay verbatim; a diff line is a diff line. Rendering is an
//! interpretation, and applying it to text that is not prose makes the window lie about
//! what was said — so the drawer reaches for [`render`] on the assistant path only.
//!
//! # Cost, stated because it is on a streaming path
//!
//! An answer is re-parsed on every `MessageUpdate`, so a hundred-event stream parses the
//! text a hundred times: O(n) per delta on a growing document. It is bounded by the
//! transcript's own row cap, and it is far cheaper than the alternative — re-parsing on a
//! timer and showing stale text — but it is the first thing to revisit if a long answer
//! stutters.

use egui::text::TextFormat;
use egui::{Color32, RichText};

/// How a run of text is styled.
///
/// A neutral enum rather than egui's `RichText`, because **`RichText`'s fields are
/// private** and a renderer whose output cannot be inspected cannot be tested. The
/// conversion happens once, in the drawer, where the palette is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanStyle {
    /// Ordinary prose.
    Plain,
    /// `*italic*`.
    Emph,
    /// `**bold**`.
    Strong,
    /// `~~struck~~`.
    Strike,
    /// `` `code` ``.
    Code,
}

/// One styled run of text.
///
/// A run rather than a whole block, so a paragraph with bold in the middle draws as one
/// label with mixed styling instead of one label per span — which is both faster and the
/// only way a widget wraps it sensibly.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    /// The text.
    pub text: String,
    /// How it is styled.
    pub style: SpanStyle,
}

impl Span {
    /// A plain span.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: SpanStyle::Plain,
        }
    }

    /// Convert to something drawable, in `base`.
    ///
    /// Code keeps its own colour rather than the body colour: an agent quoting a command
    /// needs it to look like a command, and monospace alone is not enough to read as
    /// "this was not prose".
    pub fn rich(&self, base: Color32, code: Color32) -> RichText {
        let rt = RichText::new(self.text.clone()).color(match self.style {
            SpanStyle::Code => code,
            _ => base,
        });
        match self.style {
            SpanStyle::Plain => rt,
            SpanStyle::Emph => rt.italics(),
            SpanStyle::Strong => rt.strong(),
            SpanStyle::Strike => rt.strikethrough(),
            SpanStyle::Code => rt.monospace(),
        }
    }

    /// This run's `TextFormat`, for building a [`LayoutJob`].
    ///
    /// egui 0.36's `RichText` has **private fields and no public concatenation**, so a
    /// block with mixed styling cannot be built by collecting `RichText`s. A
    /// `LayoutJob` can, and `WidgetText: From<LayoutJob>` — which is what [`job`] is for.
    pub fn format(&self, base: Color32, code: Color32) -> TextFormat {
        let (font, color) = match self.style {
            SpanStyle::Code => (egui::FontId::monospace(BODY_SIZE - 1.0), code),
            _ => (egui::FontId::proportional(BODY_SIZE), base),
        };
        TextFormat::simple(font, color)
    }
}

/// The transcript's body text size, in points.
pub const BODY_SIZE: f32 = 13.0;

/// Join `runs` into one drawable, wrapping as a single paragraph.
///
/// This is the only way to draw mixed styling in egui 0.36, and it is why a block with
/// bold in the middle of it wraps as *one* paragraph rather than as one label per span.
pub fn job(runs: &[Run], base: Color32, code: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for run in runs {
        job.append(&run.text, 0.0, run.format(base, code));
    }
    job
}

/// A run, under its own name: the portable one.
pub type Run = Span;

/// A block of the rendered answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// A heading, at `level` (1 = `#`).
    Heading {
        /// Heading level, 1 through 6.
        level: u8,
        /// Its styled runs.
        runs: Vec<Run>,
    },
    /// Ordinary prose.
    Paragraph(Vec<Run>),
    /// A bullet or numbered item, carrying the marker to draw.
    ListItem {
        /// `• `, `1. `, and so on.
        marker: String,
        /// Its styled runs.
        runs: Vec<Run>,
    },
    /// A block quote.
    Quote(Vec<Run>),
    /// A fenced code block: the language tag, and the lines.
    Code {
        /// The info string, when there was one.
        language: Option<String>,
        /// The code, verbatim. Never styled as prose and never re-wrapped.
        lines: Vec<String>,
    },
    /// A horizontal rule.
    Rule,
}

/// How a run of text is styled. Accumulated on a stack while inline tags are open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    Plain,
    Emph,
    Strong,
    Strike,
    Code,
}

/// One run being accumulated, before it becomes a [`Run`].
#[derive(Debug, Clone, PartialEq)]
struct Pending {
    style: Style,
    text: String,
}

/// Render assistant markdown into blocks.
///
/// Constructs this does not model are dropped rather than guessed at: HTML is not
/// rendered (an agent that emits `<div>` should have its *words* shown, not its markup),
/// and a table becomes plain rows — the same call the terminal renderer makes, for the
/// same reason.
pub fn render(markdown: &str) -> Vec<Block> {
    let mut options = pulldown_cmark::Options::empty();
    options.insert(pulldown_cmark::Options::ENABLE_STRIKETHROUGH);
    options.insert(pulldown_cmark::Options::ENABLE_TABLES);

    let mut state = Renderer {
        out: Vec::new(),
        pending: Vec::new(),
        styles: Vec::new(),
        kind: None,
        quote_depth: 0,
        list_stack: Vec::new(),
        code: None,
        in_table: false,
        table_row: Vec::new(),
        table_cell: String::new(),
    };

    for event in pulldown_cmark::Parser::new_ext(markdown, options) {
        state.feed(event);
    }
    state.close_code();
    state.flush();
    state.out
}

struct Renderer {
    out: Vec<Block>,
    pending: Vec<Pending>,
    styles: Vec<Style>,
    kind: Option<BlockKind>,
    quote_depth: usize,
    /// `(is_ordered, next number)` per open list.
    list_stack: Vec<(bool, u64)>,
    code: Option<CodeState>,
    in_table: bool,
    table_row: Vec<String>,
    table_cell: String,
}

#[derive(Debug, Default)]
struct CodeState {
    language: Option<String>,
    lines: Vec<String>,
    current: String,
}

/// What a block being collected is, while it is being collected.
#[derive(Debug, Clone, PartialEq)]
enum BlockKind {
    Paragraph,
    Heading(u8),
    ListItem { marker: String },
}

impl Renderer {
    /// The style every run is currently under.
    fn style(&self) -> Style {
        // Code wins: `**` inside `code` is literal, and a fence is handled separately.
        self.styles
            .iter()
            .rev()
            .copied()
            .find(|s| *s == Style::Code)
            .unwrap_or_else(|| self.styles.last().copied().unwrap_or(Style::Plain))
    }

    fn push_text(&mut self, text: &str) {
        let text = text.replace('\n', " ");
        if text.is_empty() {
            return;
        }
        let style = self.style();
        match self.pending.last_mut() {
            Some(last) if last.style == style => last.text.push_str(&text),
            _ => self.pending.push(Pending {
                style,
                text: text.to_string(),
            }),
        }
    }

    fn push_code(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        match self.pending.last_mut() {
            Some(last) if last.style == Style::Code => last.text.push_str(text),
            _ => self.pending.push(Pending {
                style: Style::Code,
                text: text.to_string(),
            }),
        }
    }

    /// Turn the accumulated runs into portable spans.
    fn runs(&mut self) -> Vec<Run> {
        std::mem::take(&mut self.pending)
            .into_iter()
            .map(|p| Run {
                text: p.text,
                style: match p.style {
                    Style::Plain => SpanStyle::Plain,
                    Style::Emph => SpanStyle::Emph,
                    Style::Strong => SpanStyle::Strong,
                    Style::Strike => SpanStyle::Strike,
                    Style::Code => SpanStyle::Code,
                },
            })
            .collect()
    }

    /// Move the runs being collected into a finished block.
    fn flush(&mut self) {
        let Some(kind) = self.kind.take() else {
            return;
        };
        if self.pending.is_empty() {
            return;
        }
        let runs = self.runs();
        if runs.is_empty() {
            return;
        }
        // A quote keeps its content and only changes the marker the drawer draws, so a
        // quoted list item stays a list item. That is why the block is rebuilt rather
        // than pushed: the two cases consume `runs` differently and it is not `Copy`.
        let block = if self.quote_depth > 0 {
            Block::Quote(runs)
        } else {
            match kind {
                BlockKind::Heading(level) => Block::Heading { level, runs },
                BlockKind::Paragraph => Block::Paragraph(runs),
                BlockKind::ListItem { marker } => Block::ListItem { marker, runs },
            }
        };
        self.out.push(block);
    }

    fn close_code(&mut self) {
        if let Some(mut c) = self.code.take() {
            Self::split_code_lines(&mut c);
            if !c.current.is_empty() {
                c.lines.push(std::mem::take(&mut c.current));
            }
            if !c.lines.is_empty() {
                self.out.push(Block::Code {
                    language: c.language,
                    lines: c.lines,
                });
            }
        }
    }

    /// Move whatever `current` holds into `lines`, splitting on newlines.
    ///
    /// pulldown-cmark delivers a fenced block's text **including** its trailing newline,
    /// so the first version turned every one-line code block into two lines — the second
    /// being empty. An agent quoting `fn main() {}` should not get a blank line under it.
    fn split_code_lines(c: &mut CodeState) {
        while let Some(idx) = c.current.find('\n') {
            let line: String = c.current.drain(..=idx).collect();
            c.lines.push(
                line.trim_end_matches('\n')
                    .trim_end_matches('\r')
                    .to_string(),
            );
        }
    }

    fn feed(&mut self, event: pulldown_cmark::Event<'_>) {
        match event {
            pulldown_cmark::Event::Start(tag) => self.start(tag),
            pulldown_cmark::Event::End(tag) => self.end(tag),
            pulldown_cmark::Event::Text(text) => {
                if self.code.is_some() {
                    // Code is verbatim: no inline styles, no reflow, no re-wrapping.
                    if let Some(c) = self.code.as_mut() {
                        c.current.push_str(&text);
                    }
                    if let Some(c) = self.code.as_mut() {
                        Self::split_code_lines(c);
                    }
                    return;
                }
                if self.in_table {
                    self.table_cell.push_str(&text);
                    return;
                }
                self.push_text(&text);
            }
            pulldown_cmark::Event::Code(text) => self.push_code(&text),
            pulldown_cmark::Event::SoftBreak => self.push_text(" "),
            pulldown_cmark::Event::HardBreak => self.push_text("\n"),
            pulldown_cmark::Event::Rule => {
                self.flush();
                self.out.push(Block::Rule);
            }
            // HTML is not rendered. An agent that emits a tag should have its words
            // shown; rendering the markup would be showing it the tag.
            pulldown_cmark::Event::Html(_) | pulldown_cmark::Event::InlineHtml(_) => {}
            _ => {}
        }
    }

    fn start(&mut self, tag: pulldown_cmark::Tag<'_>) {
        match tag {
            pulldown_cmark::Tag::Heading { level, .. } => {
                self.flush();
                self.kind = Some(BlockKind::Heading(level as u8));
            }
            pulldown_cmark::Tag::Paragraph => {
                // A paragraph inside a list item is that item's *body*, not a separate
                // block. The first version let it overwrite the item's kind, so every
                // bullet rendered as a plain paragraph and the marker was lost.
                if self.kind.is_none() && !self.list_stack.is_empty() {
                    self.kind = Some(BlockKind::ListItem {
                        marker: "• ".to_string(),
                    });
                } else if self.kind.is_none() {
                    self.kind = Some(BlockKind::Paragraph);
                }
            }
            pulldown_cmark::Tag::BlockQuote(_) => {
                self.flush();
                self.quote_depth += 1;
            }
            pulldown_cmark::Tag::CodeBlock(kind) => {
                self.flush();
                let language = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(info) if !info.is_empty() => {
                        Some(info.to_string())
                    }
                    _ => None,
                };
                self.code = Some(CodeState {
                    language,
                    lines: Vec::new(),
                    current: String::new(),
                });
            }
            pulldown_cmark::Tag::List(start) => {
                self.flush();
                // `start` is `Some` for an ordered list and `None` for a bullet
                // one. The first version pushed `(true, ...)` unconditionally and rendered
                // every bullet as `1.`, `2.`, `3.`
                let ordered = start.is_some();
                self.list_stack.push((ordered, start.unwrap_or(1)));
            }
            pulldown_cmark::Tag::Item => {
                self.flush();
                let ordered = self.list_stack.last().map(|(o, _)| *o).unwrap_or(false);
                let marker = if ordered {
                    let n = self.list_stack.last().map(|(_, n)| *n).unwrap_or(1);
                    if let Some(last) = self.list_stack.last_mut() {
                        last.1 += 1;
                    }
                    format!("{n}. ")
                } else {
                    "• ".to_string()
                };
                self.kind = Some(BlockKind::ListItem { marker });
            }
            pulldown_cmark::Tag::Emphasis => self.styles.push(Style::Emph),
            pulldown_cmark::Tag::Strong => self.styles.push(Style::Strong),
            pulldown_cmark::Tag::Strikethrough => self.styles.push(Style::Strike),
            pulldown_cmark::Tag::Table(_) => {
                self.flush();
                self.in_table = true;
            }
            pulldown_cmark::Tag::TableRow => self.table_row.clear(),
            pulldown_cmark::Tag::TableCell => self.table_cell.clear(),
            _ => {}
        }
    }

    fn end(&mut self, tag: pulldown_cmark::TagEnd) {
        match tag {
            pulldown_cmark::TagEnd::Heading(_) | pulldown_cmark::TagEnd::Paragraph => {
                self.flush();
            }
            pulldown_cmark::TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            pulldown_cmark::TagEnd::CodeBlock => {
                self.close_code();
            }
            pulldown_cmark::TagEnd::List(_) => {
                self.flush();
                self.list_stack.pop();
            }
            pulldown_cmark::TagEnd::Item => {
                self.flush();
                self.kind = None;
            }
            pulldown_cmark::TagEnd::Emphasis
            | pulldown_cmark::TagEnd::Strong
            | pulldown_cmark::TagEnd::Strikethrough => {
                self.styles.pop();
            }
            pulldown_cmark::TagEnd::Table => {
                self.in_table = false;
            }
            pulldown_cmark::TagEnd::TableRow | pulldown_cmark::TagEnd::TableHead => {
                let row = self.table_row.join("  ");
                if !row.trim().is_empty() {
                    self.out.push(Block::Paragraph(vec![Run::plain(row)]));
                }
                self.table_row.clear();
            }
            pulldown_cmark::TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.table_cell);
                self.table_row.push(cell.trim().to_string());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text a reader would see, with runs joined.
    fn text_of(block: &Block) -> String {
        match block {
            Block::Heading { runs, .. }
            | Block::Paragraph(runs)
            | Block::Quote(runs)
            | Block::ListItem { runs, .. } => runs
                .iter()
                .map(|r| r.text.as_str())
                .collect::<Vec<_>>()
                .join(""),
            Block::Code { lines, .. } => lines.join("\n"),
            Block::Rule => "---".to_string(),
        }
    }

    #[test]
    fn a_plain_answer_is_one_paragraph() {
        let blocks = render("Hello there.");
        assert_eq!(blocks.len(), 1);
        assert_eq!(text_of(&blocks[0]), "Hello there.");
        assert!(matches!(blocks[0], Block::Paragraph(_)));
    }

    /// The reason this module exists. A model answer that shows `**bold**` is a window
    /// that has not rendered what it was given.
    #[test]
    fn emphasis_is_a_style_rather_than_visible_asterisks() {
        let blocks = render("a **bold** word");
        let joined = text_of(&blocks[0]);
        assert!(
            !joined.contains('*'),
            "the asterisks are still on screen: {joined:?}"
        );
        assert_eq!(joined, "a bold word");
    }

    #[test]
    fn the_bold_run_is_actually_strong() {
        let blocks = render("a **bold** word");
        let Block::Paragraph(runs) = &blocks[0] else {
            panic!("expected a paragraph: {:?}", blocks[0]);
        };
        let bold = runs
            .iter()
            .find(|r| r.text == "bold")
            .expect("the run survives");
        assert_eq!(
            bold.style,
            SpanStyle::Strong,
            "a run stripped of asterisks is not bold"
        );
    }

    #[test]
    fn inline_code_is_monospace_and_loses_its_backticks() {
        let blocks = render("run `cargo test` now");
        let Block::Paragraph(runs) = &blocks[0] else {
            panic!("expected a paragraph: {:?}", blocks[0]);
        };
        let code = runs
            .iter()
            .find(|r| r.text == "cargo test")
            .expect("the code run survives as its own run");
        assert_eq!(code.style, SpanStyle::Code);
        assert!(!text_of(&blocks[0]).contains('`'));
    }

    #[test]
    fn a_heading_keeps_its_level_and_loses_its_hashes() {
        let blocks = render("## What changed");
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Heading { level, runs } => {
                assert_eq!(*level, 2);
                assert_eq!(
                    runs.iter().map(|r| r.text.as_str()).collect::<String>(),
                    "What changed"
                );
            }
            other => panic!("expected a heading, got {other:?}"),
        }
    }

    #[test]
    fn a_bullet_list_is_items_not_literal_hyphens() {
        let blocks = render("- first\n- second\n");
        let items: Vec<String> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::ListItem { runs, .. } => {
                    Some(runs.iter().map(|r| r.text.as_str()).collect::<String>())
                }
                _ => None,
            })
            .collect();
        assert_eq!(items, vec!["first", "second"]);
        assert!(matches!(&blocks[0], Block::ListItem { marker, .. } if marker == "• "));
    }

    #[test]
    fn a_numbered_list_is_numbered() {
        let blocks = render("1. one\n2. two\n");
        let markers: Vec<&str> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::ListItem { marker, .. } => Some(marker.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(markers, vec!["1. ", "2. "]);
    }

    /// Code is verbatim: an agent quoting a file needs its indentation and its asterisks.
    #[test]
    fn a_fenced_block_is_verbatim_and_remembers_its_language() {
        let blocks = render("```rust\nfn main() { let x = **not bold**; }\n```\n");
        match &blocks[0] {
            Block::Code { language, lines } => {
                assert_eq!(language.as_deref(), Some("rust"));
                assert_eq!(
                    lines,
                    &vec!["fn main() { let x = **not bold**; }".to_string()]
                );
            }
            other => panic!("expected a code block, got {other:?}"),
        }
    }

    /// Inside a fence, `**x**` must stay literal — that is the whole point of a fence.
    #[test]
    fn markdown_inside_a_fence_is_not_styled() {
        let blocks = render("```\n**literal**\n```");
        match &blocks[0] {
            Block::Code { lines, .. } => assert_eq!(lines[0], "**literal**"),
            other => panic!("expected code, got {other:?}"),
        }
    }

    #[test]
    fn a_table_becomes_readable_rows_rather_than_one_column() {
        // The terminal renderer made this call for the same reason: a table laid out one
        // column wide is unreadable and reads as a rendering bug.
        let blocks = render("| a | b |\n|---|---|\n| 1 | 2 |\n");
        let text: Vec<String> = blocks.iter().map(text_of).collect();
        assert!(
            text.iter().all(|t| !t.contains('|')),
            "pipes must not survive into the reader's view: {text:?}"
        );
        assert!(
            text.iter().any(|t| t.contains('a') && t.contains('b')),
            "{text:?}"
        );
    }

    #[test]
    fn a_quote_is_marked_as_one() {
        let blocks = render("> careful here");
        assert!(
            blocks.iter().any(|b| matches!(b, Block::Quote(_))),
            "{blocks:?}"
        );
    }

    /// The streaming path re-parses on every delta, so it parses fragments: an
    /// unterminated fence, an unclosed emphasis, a heading with no text.
    #[test]
    fn a_streamed_half_answer_returns_without_panicking_or_emptying() {
        for fragment in [
            "",
            "**",
            "# ",
            "- ",
            "```rust\nfn ma",
            "```\n\nsome text after",
            "> quote without end",
            "| a | b |",
            "1. ",
        ] {
            let blocks = render(fragment);
            for b in &blocks {
                assert!(
                    !text_of(b).trim().is_empty(),
                    "an empty block from {fragment:?}: {blocks:?}"
                );
            }
        }
    }

    #[test]
    fn html_is_dropped_rather_than_rendered() {
        let blocks = render("<div>words</div>");
        let joined: String = blocks.iter().map(text_of).collect();
        assert!(
            !joined.contains("<div>"),
            "markup must not be shown: {joined:?}"
        );
    }

    #[test]
    fn a_hard_break_keeps_both_halves_readable() {
        let blocks = render("one  \ntwo");
        let joined: String = blocks.iter().map(text_of).collect();
        assert!(
            joined.contains("one") && joined.contains("two"),
            "{joined:?}"
        );
    }
}
