//! Tool cards, and the one thing in this crate that must never be prose: a diff.
//!
//! # Why this module exists
//!
//! A tool call used to be one flat row — `tool-call bash {"command":"cargo test"}` —
//! and a tool result `{name} → {truncated body}`. The state that actually matters
//! was implied rather than carried: nothing said *pending*, nothing said how long
//! the call took, and an `edit` result rendered its diff by pushing
//! `format!("{name} → diff:\n{diff}")` through the **markdown** parser. So the
//! audit's finding was literal: a diff arrived on screen as prose. `- let x = 1;`
//! became a bullet-list item (a line starting with `-` is a list in CommonMark)
//! and `---` became a horizontal rule across the card.
//!
//! A card is therefore two things a flat row cannot be:
//!
//! ```text
//! ▸ edit · pending                                      <- title: name, status, time
//!   src/ui/layout.rs                                    <- body: the detail line
//!
//! ✓ edit · ok · 12ms                                    <- the result card
//!   @@ -4,7 +4,9 @@                                     <- hunk header, accent
//!    12 fn draw(f: &mut Frame) {                        <- context, dim
//!   -13     let rows = cache.rows();                    <- removed, danger
//!   +13     let rows = cache.rows(width);               <- added, success
//!   … 43 more lines (ctrl+x to open)
//! ```
//!
//! * **A title line** carrying the tool's name, its status and its duration, so
//!   the reader knows what happened without parsing the body.
//! * **A body** capped at [`BODY_ROWS`], with an explicit affordance naming the
//!   key that opens it — and a diff body that is *rendered*, not parsed.
//!
//! # Colour is never the only signal
//!
//! The `+`/`-` sign and the line number carry added/removed on their own; colour
//! reinforces them. That is what keeps the diff readable under `NO_COLOR` and on a
//! 16-colour terminal, and it is why the *numbers* are what gets dropped when the
//! card is narrow while the sign always stays: a clipped number is worse than no
//! number, while a missing sign makes two different things identical.
//!
//! The diff deliberately uses no background bands at all. The transcript's
//! `surface_ok`/`surface_err` already say whether the call as a whole succeeded,
//! and a second band behind every diff line is the "four signals for one state"
//! the skill's clutter audit cuts first. Per-side colour plus the sign is the
//! whole treatment, which is also what pi does (`components/diff.ts:104-141`) —
//! and it means nothing here has to ask [`Theme::has_surfaces`], because nothing
//! here would use a band if it had one.
//!
//! # Split or unified
//!
//! [`SPLIT_MIN_WIDTH`] is 120 body columns, and the reasoning is one line long: a
//! split diff halves the width, so below ~120 columns each side is under 55 cells
//! and ordinary Rust (`        let rendered = self.rendered.get(&id);`) wraps — at
//! which point four wrapped rows are showing what two unified rows would have. A
//! unified diff that fits beats a split one that wraps. The threshold is checked
//! against the **body** width rather than the frame width, because the body is
//! what gets halved.

use ratatui::style::{Modifier, Style};

use crate::state::{Card, CardBody, CardStatus, Diff, DiffKind, DiffLine};
use crate::ui::theme::{Glyphs, Theme};

/// Rows a card body may occupy before it is capped and an affordance is appended.
///
/// Twelve is the skill's number (`references/ai-harness.md`: "Cap the card at
/// roughly 12 visible lines"), and it is a *row* budget rather than a line count:
/// a long line that wraps costs its wrapped rows, so a body can never overflow
/// the cap by wrapping under it.
pub const BODY_ROWS: usize = 12;

/// Below this body width a diff is drawn unified, at or above it split.
pub const SPLIT_MIN_WIDTH: usize = 120;

/// Columns a pair of line numbers needs (two digits of sign, two of number, two
/// of padding) before they are worth printing at all.
pub const MIN_NUMBER_WIDTH: usize = 8;

/// One rendered card row: styled spans, already wrapped/cut to the width it was
/// built for.
pub type CardRow = Vec<(String, Style)>;

/// The title line: `edit · pending`, `bash · ok · 12ms`.
///
/// **The status word, not the status glyph.** The glyph is already in the gutter:
/// `ui::transcript::row_style` paints it at column 0, and it is the *same*
/// character `status_word` would have put here — a pending call is `▸` in both
/// places, a result `✓` in both, a failure `✗` in both. Printing it twice put the
/// same character in the same row three columns apart, which reads as a
/// rendering fault rather than as a marker.
///
/// What is lost by dropping the glyph is nothing the reader had: the gutter still
/// carries the glance-level signal, and the **word** beside it is the part that
/// survives `NO_COLOR`, a 16-colour terminal, and a font that cannot draw the
/// glyph. `partial` and `denied` are the cases that prove the word is doing the
/// work — neither has a distinct gutter glyph, and both are unambiguous here.
///
/// (`status_word` still returns the glyph: the row renderer uses it for the
/// padding/cut width arithmetic, and dropping it there would change the layout
/// rather than the text.)
pub fn title(card: &Card, theme: &Theme) -> String {
    let (_glyph, word) = status_word(card.status, &theme.glyphs);
    // M153: the headline comes from the renderer, so a `read` card names the file and the
    // line range instead of repeating 400 characters of it. It is optional: a tool the
    // renderer knows nothing about still gets a readable card, just a less specific one.
    let head = card.head.trim();
    // The renderer's fallback already leads with the tool's name, because that string is
    // also the collapsed transcript line, where the name is the only thing saying which tool
    // ran. The card title prints the name too, so prefixing it again gave `bash bash → boom`.
    // Strip the redundant prefix rather than discarding the head with it: dropping the whole
    // headline instead lost `→ boom`, which is the part that says what happened.
    let prefix = format!("{} \u{2192} ", card.name);
    let head = head.strip_prefix(prefix.as_str()).unwrap_or(head);
    let name = if head.is_empty() {
        card.name.clone()
    } else {
        format!("{} {head}", card.name)
    };
    match card.elapsed_ms() {
        Some(ms) => format!("{name} · {word} · {}", ms_text(ms)),
        None => format!("{name} · {word}"),
    }
}

/// The glyph *and* the word for one status. Both, always: the glyph is what a
/// glance catches and the word is what survives a font that cannot draw it.
///
/// `Pending` and `Running` are two states and now say so. They used to share
/// the word `pending`, so a card streaming output read `▸ bash $ cargo build ·
/// pending` beside a header saying `running bash…` — three rows, three words,
/// one tool. `Pending` is "asked for, not yet started" (the state a permission
/// prompt leaves a call in, which `state.rs` keeps it in deliberately);
/// `Running` is the one whose body is filling. The glyph is unchanged, so the
/// gutter still looks the same at a glance.
fn status_word(status: CardStatus, glyphs: &Glyphs) -> (&'static str, &'static str) {
    match status {
        CardStatus::Pending => (glyphs.tool_call, "pending"),
        CardStatus::Running => (glyphs.tool_call, "running"),
        CardStatus::Ok => (glyphs.tick, "ok"),
        // Deliberately not the tick, and deliberately not the cross. The tick would say the
        // result is whole when the harness knows it is short; the cross would say the command
        // failed when it succeeded. A warning glyph with the word "partial" is the only one of
        // the three that is not a lie, and the word is what survives `NO_COLOR` or a font that
        // cannot draw the glyph.
        CardStatus::Partial => (glyphs.warn, "partial"),
        CardStatus::Failed => (glyphs.cross, "failed"),
        CardStatus::Denied => (glyphs.cross, "denied"),
        // "stopped", not "failed": the tool did not fail, the turn was stopped.
        // The warn glyph rather than the cross, for the same reason — a cross
        // reads as a defect where there was a decision (C-110's wording).
        CardStatus::Stopped => (glyphs.warn, "stopped"),
    }
}

/// A duration in the shortest honest unit. Sub-second work is the common case for
/// a tool card, so `412ms` beats the whole-second `0s` a general formatter would
/// print for nearly every call — which reads as a field that does not work.
pub fn ms_text(ms: u64) -> String {
    if ms < 10_000 {
        format!("{ms}ms")
    } else if ms < 600_000 {
        format!("{}s", (ms + 500) / 1000)
    } else {
        crate::ui::format::format_duration(std::time::Duration::from_millis(ms))
    }
}

/// The affordance that names the key which opens a capped body.
///
/// `key` is resolved from the live binding table by the caller
/// ([`crate::keybindings::KeybindingsManager::key_text`]), so a rebound
/// `app.tools.expand` moves this text with it. `None` means the binding does not
/// resolve; the affordance then states the count and promises nothing, because a
/// hint naming a dead key is worse than a bare count.
///
/// `hidden_below` says which end is hidden, and the default is the honest one
/// for the body kind that keeps its head. A tail-kept card passes `false` so the
/// sentence reads `N earlier lines` — see ``body_keeps_tail`` for why a card
/// would keep its tail at all.
pub fn more_lines(remaining: usize, key: Option<&str>, hidden_below: bool) -> String {
    // The singular is not pedantry: an expandable **event** row hides its whole
    // digest, and a one-line digest is the common case — "… 1 more lines" was
    // the row's own text every time a compaction summary fit one line.
    let unit = if remaining == 1 { "line" } else { "lines" };
    let direction = if hidden_below { "more" } else { "earlier" };
    match key {
        Some(key) => format!("… {remaining} {direction} {unit} ({key} to open)"),
        None => format!("… {remaining} {direction} {unit}"),
    }
}

/// Every row of a card: the title, then the body, capped at `budget`.
///
/// `expanded` is this card's *effective* state — the global "expand all" toggle
/// already folded in by the caller — and the rows come back wrapped to `width`,
/// so the caller only decides whether it has room for them.
///
/// An expanded card is **not capped at all**, rather than capped at a bigger
/// number: the affordance the cap produces is the promise that the key shows the
/// rest, and a second, larger cap would print `… 6 more lines (ctrl+x to open)`
/// under a card that is already open, telling the reader to press the key they
/// just pressed. The transcript is virtualized, so a long body costs the rows it
/// draws and nothing else.
pub fn rows(
    card: &Card,
    expanded: bool,
    width: usize,
    budget: usize,
    theme: &Theme,
    more_key: Option<&str>,
) -> Vec<CardRow> {
    let body = match &card.body {
        CardBody::Diff(diff) => diff_rows(diff, width, theme),
        CardBody::Prompt(text) => prompt_rows(text, expanded, theme),
        // **One row per line, or the lines run together (M312).** This was
        // `vec![vec![(sanitize(text), …)]]` — the whole body on one row — and
        // [`sanitize`] **strips the newline**, because it strips control characters
        // and `\n` is one (this is the same `text_safety` rule that keeps a model's
        // output from steering the screen). A tool that printed two lines therefore
        // rendered as one sentence: `running 74 teststest result: ok. 74 passed`.
        //
        // **The product deliberately stores multi-line bodies**, so this was a renderer that
        // could not draw what it was given: the streaming path keeps a bounded **tail of
        // lines** (`state.rs`, `CardBody::Text(tail.join("\n"))`), and every line after the
        // first was being lost. [`cap`] then does what it exists for and prints
        // `N more lines (ctrl+x)` for the rest, so a long body costs rows rather than
        // becoming one unreadable row.
        //
        // This is the M198 class this module's own header is about — a card that is
        // correct in every field and unreadable on screen.
        CardBody::Text(text) => text
            .lines()
            .map(|l| vec![(sanitize(l), theme.dim())])
            .collect(),
    };
    let mut rows: Vec<CardRow> = vec![vec![(title(card, theme), theme.text())]];
    // Which end the cap keeps, decided from the body's kind before it is built.
    let keep_tail = body_keeps_tail(match &card.body {
        CardBody::Diff(_) => "diff",
        CardBody::Prompt(_) => "prompt",
        CardBody::Text(_) => "text",
    });
    // The budgets the skill asks for are *visible* rows, so the wrap happens first
    // and the cap counts what the reader would actually see.
    //
    // **A row that already fits is not re-wrapped.** The diff's rows are cut and
    // padded to the body width by their own renderer — that is what makes a split
    // row two columns rather than two lines — and `wrap::wrap_line` is not
    // idempotent on them: it drops trailing whitespace and collapses a whitespace
    // run to one space, which is right for prose ("screen convention", `wrap.rs`)
    // and wrong for a row whose alignment *is* the padding. Re-wrapping a padded
    // 138-cell split row squeezed it to 25 cells, put the right half 13 columns
    // after the left instead of 69, and erased the blank half that says "this line
    // has no counterpart". Only a split diff (body width ≥ `SPLIT_MIN_WIDTH`) could
    // show it, and until `the_split_diff_draws_both_sides_at_a_wide_frame` no test
    // rendered one.
    let wrapped: Vec<CardRow> = body
        .into_iter()
        .flat_map(|row| {
            if row_width(&row) <= width {
                vec![row]
            } else {
                // Borrowed, so the spans are wrapped in place rather than copied
                // into a second owned vector first. This is the one path that
                // wraps *tool output* — the widest text the UI ever handles.
                crate::wrap::wrap_line(
                    &row.iter()
                        .map(|(text, style)| (text.as_str(), *style))
                        .collect::<Vec<_>>(),
                    width,
                )
            }
        })
        .collect();
    let budget = if expanded { usize::MAX } else { budget };
    rows.extend(cap(
        wrapped,
        budget,
        theme,
        more_key,
        crate::ui::transcript::GUTTER,
        keep_tail,
    ));
    rows
}

/// Which end of a capped body the reader keeps.
///
/// **Text keeps the tail; a diff keeps the head**, and the split is a decision
/// rather than a preference. The most-read long card in a coding agent is a
/// build or test log, and its verdict is at the bottom — a head-kept cap shows
/// `Compiling crate-0 …` and hides `test result: FAILED`, which is the entire
/// reason the card exists. Pi is deliberately non-uniform in exactly this way:
/// its generic fallback keeps the **first** ten lines
/// (`tool-execution.ts:40`, `:166`) while its bash card keeps the **last**
/// twenty (`bash-execution.ts:131`, `.slice(-PREVIEW_LINES)`) and its bash tool
/// preview keeps `keep: "end"` (`renderers/bash.ts:74`). Epsilon has one card,
/// so it takes the bash card's rule — ours *is* Pi's bash card.
///
/// A diff keeps the head and this is the control on the rule: a change reads
/// from its first hunk downwards, so a diff whose top is hidden cannot be
/// reviewed at all. Widening the tail rule to every body kind would fix the log
/// and break the diff.
fn body_keeps_tail(kind: &str) -> bool {
    kind == "text"
}

/// A pending call: what is about to run.
fn prompt_rows(text: &str, expanded: bool, theme: &Theme) -> Vec<CardRow> {
    let body = sanitize(text);
    if expanded {
        return body
            .lines()
            .map(|l| vec![(l.to_string(), theme.text())])
            .collect();
    }
    // Collapsed: the detail that names the call, on one row. A ten-line heredoc is
    // not what a reader is looking for in a pending card, and the count under the
    // cap says the rest is there.
    let first = body.lines().next().unwrap_or("").to_string();
    vec![vec![(first, theme.text())]]
}

/// Body rows for an expandable **event** row — today the compaction digest.
///
/// The card machinery owns the convention (cap, affordance wording, the chord's
/// resolved text), so the event row borrows it here rather than growing a second
/// copy in the renderer — with one deliberate difference from a tool card's body:
///
/// **A digest is hidden whole, not sampled.** A tool result is output you want a
/// taste of, so its collapsed form keeps twelve rows behind the affordance. A
/// compaction summary is prose about prose: its first lines are the oldest
/// context, its last are the current state, and a head of it reads as a
/// truncated thought rather than as a preview. Pi hides it the same way — the
/// collapsed row is the token count and the chord
/// (`compaction-summary-message.ts:49-51`) — so the closed form names how much
/// is behind the key and shows none of it.
pub(crate) fn event_body(
    full: &str,
    open: bool,
    theme: &Theme,
    more_key: Option<&str>,
) -> Vec<CardRow> {
    // The count names source lines — what is hidden — and stays put whether the open
    // body draws plain or parsed: a heading parses to one row but hides one line.
    let hidden = full.lines().count();
    if open {
        // The digest is model prose, so the open body is parsed, not printed: headers,
        // lists and code spans draw as the assistant's own markdown does (same parser,
        // `md_to_lines`). Plain text parses to itself, so a digest with no markup
        // draws exactly as it did — the markdown half changes structure, never words.
        return crate::markdown::md_to_lines(full, theme)
            .into_iter()
            .map(|l| {
                l.spans
                    .into_iter()
                    .map(|s| (s.content.to_string(), s.style))
                    .collect()
            })
            .collect();
    }
    vec![vec![(
        format!(
            "{}{}",
            " ".repeat(crate::ui::transcript::GUTTER),
            // The digest hides **below**, so the affordance says "more", never
            // "earlier": the reader is looking at the end of the transcript and
            // the key opens what is under the row.
            more_lines(hidden, more_key, true)
        ),
        theme.faint(),
    )]]
}

/// Cap a body at `budget` rows, appending the "how much is left" affordance.
///
/// The affordance sits *outside* the cap, so a capped card is `budget + 1` rows
/// and the number it prints is the true count of rows the reader cannot see.
///
/// `keep_tail` decides which end is kept — see [`body_keeps_tail`]. The
/// affordance names the end too: a reader told only `N more lines` assumes the
/// missing ones are *below*, which is the opposite of what a tail-kept card did.
fn cap(
    rows: Vec<CardRow>,
    budget: usize,
    theme: &Theme,
    more_key: Option<&str>,
    indent: usize,
    keep_tail: bool,
) -> Vec<CardRow> {
    if budget == 0 || rows.len() <= budget {
        return rows;
    }
    let remaining = rows.len() - budget;
    let mut out: Vec<CardRow> = if keep_tail {
        rows.into_iter().skip(remaining).collect()
    } else {
        rows.into_iter().take(budget).collect()
    };
    out.push(vec![(
        format!(
            "{}{}",
            " ".repeat(indent),
            more_lines(remaining, more_key, !keep_tail)
        ),
        theme.faint(),
    )]);
    out
}

// ---- the diff ----------------------------------------------------------------

fn diff_rows(diff: &Diff, width: usize, theme: &Theme) -> Vec<CardRow> {
    if width >= SPLIT_MIN_WIDTH {
        split_rows(diff, width, theme)
    } else {
        unified_rows(diff, width, theme)
    }
}

/// The unified view: one row per diff line, ` 12   content`, `-13   let x = 1;`.
fn unified_rows(diff: &Diff, width: usize, theme: &Theme) -> Vec<CardRow> {
    let numbers = diff.unified_numbers_fit(width);
    let mut out = Vec::with_capacity(diff.lines.len());
    for run in runs(&diff.lines) {
        match run {
            Run::Meta(line) => out.push(meta_row(line, theme)),
            Run::Context(line) => out.push(unified_context(line, numbers, width, theme)),
            Run::Change { removed, added } => {
                // Intra-line highlighting is meaningful for exactly one removed
                // line paired with one added one: that is a small edit of a line.
                // More on either side and the reader is comparing blocks, not
                // words — which is pi's rule too (`diff.ts:113-134`).
                let pair = intra_pair(&removed, &added);
                for line in &removed {
                    out.push(unified_line(line, numbers, width, theme, pair.as_ref()));
                }
                for line in &added {
                    out.push(unified_line(line, numbers, width, theme, pair.as_ref()));
                }
            }
        }
    }
    out
}

/// A hunk header or a line the parser did not recognise (`\ No newline at end of
/// file`), printed as the diff carried it.
fn meta_row(line: &DiffLine, theme: &Theme) -> CardRow {
    let style = match line.kind {
        DiffKind::Hunk => theme.accent(),
        _ => theme.faint(),
    };
    vec![(sanitize(&line.text), style)]
}

/// One row of the unified view.
fn unified_line(
    line: &DiffLine,
    numbers: bool,
    width: usize,
    theme: &Theme,
    pair: Option<&IntraPair>,
) -> CardRow {
    let base = side_style(line.kind, theme);
    let lead = sign(line.kind, &theme.glyphs);
    let number = if numbers {
        number_text(line.number, theme)
    } else {
        Vec::new()
    };
    let head_width: usize = 2 + number.iter().map(|(t, _)| cols(t)).sum::<usize>();
    let body_width = width.saturating_sub(head_width);
    let content = match (line.kind, pair) {
        (DiffKind::Remove, Some(pair)) => emphasize(&pair.removed, base, body_width),
        (DiffKind::Add, Some(pair)) => emphasize(&pair.added, base, body_width),
        _ => vec![(
            cut(&sanitize(&line.text), body_width),
            base.add_modifier(emphasis(line.kind)),
        )],
    };
    let mut row: CardRow = vec![(format!("{lead} "), base)];
    row.extend(number);
    row.extend(content);
    pad_to_cell(row, width, base)
}

/// A context row: the sign column is blank, so the eye can run down the signs.
fn unified_context(line: &DiffLine, numbers: bool, width: usize, theme: &Theme) -> CardRow {
    let base = theme.dim_strong();
    let number = if numbers {
        number_text(line.number, theme)
    } else {
        Vec::new()
    };
    let head_width: usize = 2 + number.iter().map(|(t, _)| cols(t)).sum::<usize>();
    let body_width = width.saturating_sub(head_width);
    let mut row: CardRow = vec![("  ".to_string(), base)];
    row.extend(number);
    row.push((cut(&sanitize(&line.text), body_width), base));
    pad_to_cell(row, width, base)
}

/// The split view: old side then new side, one row per paired diff line.
fn split_rows(diff: &Diff, width: usize, theme: &Theme) -> Vec<CardRow> {
    let left_width = width / 2;
    let right_width = width - left_width;
    let numbers = diff.split_numbers_fit(left_width);
    let mut out = Vec::with_capacity(diff.lines.len());
    for run in runs(&diff.lines) {
        match run {
            Run::Meta(line) => out.push(meta_row(line, theme)),
            Run::Context(line) => {
                out.push(context_split(line, numbers, left_width, right_width, theme));
            }
            Run::Change { removed, added } => {
                let pair = intra_pair(&removed, &added);
                for i in 0..removed.len().max(added.len()) {
                    let old = removed.get(i).copied();
                    let new = added.get(i).copied();
                    let highlighted = pair.as_ref().map(|p| (&p.removed, &p.added));
                    let mut row = split_column(
                        old,
                        highlighted.map(|(r, _)| r),
                        DiffKind::Remove,
                        numbers,
                        left_width,
                        theme,
                    );
                    row.extend(split_column(
                        new,
                        highlighted.map(|(_, a)| a),
                        DiffKind::Add,
                        numbers,
                        right_width,
                        theme,
                    ));
                    out.push(row);
                }
            }
        }
    }
    out
}

/// A context line in the split view: the same text on both sides.
fn context_split(
    line: &DiffLine,
    numbers: bool,
    left_width: usize,
    right_width: usize,
    theme: &Theme,
) -> CardRow {
    let mut row = split_column(
        Some(line),
        None,
        DiffKind::Context,
        numbers,
        left_width,
        theme,
    );
    row.extend(split_column(
        Some(line),
        None,
        DiffKind::Context,
        numbers,
        right_width,
        theme,
    ));
    row
}

/// One half of the split view: sign, number, then the line cut to fit.
///
/// A side with no line of its own is drawn blank but still occupies its columns —
/// an empty half is what makes a split diff readable, because it is where "this
/// line has no counterpart" is said.
fn split_column(
    line: Option<&DiffLine>,
    highlight: Option<&Vec<(String, Style)>>,
    expect: DiffKind,
    numbers: bool,
    width: usize,
    theme: &Theme,
) -> CardRow {
    let kind = line.map(|l| l.kind).unwrap_or(expect);
    let base = match line {
        Some(_) => side_style(kind, theme),
        None => theme.faint(),
    };
    let lead = if line.is_some() {
        sign(kind, &theme.glyphs)
    } else {
        " "
    };
    let number = if numbers {
        number_text(line.and_then(|l| l.number), theme)
    } else {
        Vec::new()
    };
    let head_width: usize = 2 + number.iter().map(|(t, _)| cols(t)).sum::<usize>();
    let body_width = width.saturating_sub(head_width);
    let content = match (line, highlight) {
        (Some(_), Some(spans)) => emphasize(spans, base, body_width),
        (Some(l), None) => vec![(
            cut(&sanitize(&l.text), body_width),
            base.add_modifier(emphasis(kind)),
        )],
        (None, _) => vec![(String::new(), base)],
    };
    let mut row: CardRow = vec![(format!("{lead} "), base)];
    row.extend(number);
    row.extend(content);
    pad_to(row, width, base)
}

/// The line number column, padded so the body starts at the same cell on every
/// row. `Diff::number_width` is what makes that possible.
fn number_text(number: Option<usize>, theme: &Theme) -> CardRow {
    match number {
        Some(n) => vec![(format!("{n} "), theme.faint())],
        None => vec![("  ".to_string(), theme.faint())],
    }
}

/// The style for a diff line's kind. Only these three are ever used, so the diff
/// cannot quietly acquire a fourth colour.
fn side_style(kind: DiffKind, theme: &Theme) -> Style {
    match kind {
        DiffKind::Add => theme.success(),
        DiffKind::Remove => theme.danger(),
        _ => theme.dim_strong(),
    }
}

/// The sign that introduces a row: `+` for an addition, `-` for a removal, a
/// space for context, and always from the glyph repertoire so a legacy console
/// gets an ASCII character rather than mojibake.
fn sign(kind: DiffKind, glyphs: &Glyphs) -> &'static str {
    match kind {
        DiffKind::Add => glyphs.plus,
        DiffKind::Remove => glyphs.minus,
        _ => " ",
    }
}
/// The modifier that carries a line's kind when colour cannot. Context is dimmed;
/// added and removed are *not* "the other one in a different colour", so nothing
/// distinguishes them but the sign — which is the point.
fn emphasis(kind: DiffKind) -> Modifier {
    match kind {
        DiffKind::Add | DiffKind::Remove => Modifier::empty(),
        _ => Modifier::DIM,
    }
}

/// Pad a row out to `width` with the row's own style, so a removed line's colour
/// runs to the edge of its column rather than stopping at the last character and
/// leaving a ragged right edge down the card.
///
/// **Padding is a split-view device.** Its purpose is to hold one row's columns in
/// step with another row's, which is what a side-by-side diff is; a unified row has
/// no other column to line up with, so its trailing spaces mark nothing — the
/// terminal's background is there anyway. The asymmetry matters because a wrapper
/// downstream cannot tell "trailing spaces that mean nothing" from "trailing spaces
/// that *are* the layout": `wrap_line` deletes trailing whitespace by design
/// (correctly, for prose), so a padded unified row lost its padding and looked
/// unchanged, while a padded split row lost the alignment that is the whole feature.
/// Never emitting the meaningless ones is what lets the meaningful ones survive —
/// see [`pad_to_cell`].
fn pad_to(mut row: CardRow, width: usize, style: Style) -> CardRow {
    let used: usize = row.iter().map(|(t, _)| cols(t)).sum();
    if used < width {
        row.push((" ".repeat(width - used), style));
    }
    row
}

/// Pad a unified row to the end of the **cell** its own content defines, not to the
/// card.
///
/// A unified row's colour has to reach the edge of the change, which is the one
/// separator cell past the text — `unified_line` emits `{lead} ` and
/// `unified_context` emits `"  "`, so the row's last cell is that separator. Past it
/// the colour marks nothing, and padding further is what put a meaningless run of up
/// to sixty spaces on every unified diff row. The row then measured exactly the body
/// width, so the "does this need wrapping" test one layer up accepted it and its
/// padding survived — which is how a cosmetic difference became the split diff's two
/// halves landing on top of each other. Padding only what a row's own layout needs
/// fixes both, and leaves the unified frame byte-identical to the one the golden
/// tests already pin.
fn pad_to_cell(row: CardRow, width: usize, style: Style) -> CardRow {
    let used: usize = row.iter().map(|(t, _)| cols(t)).sum();
    pad_to(row, used.saturating_sub(1).max(width), style)
}

/// Display width of a card row, for the "does this need wrapping" question.
///
/// Shared with [`crate::render_cache`], which asks the same question about the same
/// rows one layer up: a renderer that pads a row to its column width is making a
/// claim about the layout, and the next wrapper in the chain has to be able to see
/// it. The measurement is the same one [`crate::wrap::wrap_line`] makes — `cols` is
/// `unicode-width` — so a row this accepts is a row the wrap would leave alone.
pub(crate) fn row_width(row: &CardRow) -> usize {
    row.iter().map(|(text, _)| cols(text)).sum()
}

// ---- runs --------------------------------------------------------------------

/// Consecutive removals followed by consecutive additions are one *change*; a
/// lone added line is a change with an empty old side.
enum Run<'a> {
    Meta(&'a DiffLine),
    Context(&'a DiffLine),
    Change {
        removed: Vec<&'a DiffLine>,
        added: Vec<&'a DiffLine>,
    },
}

fn runs(lines: &[DiffLine]) -> Vec<Run<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        match lines[i].kind {
            DiffKind::Remove => {
                let start = i;
                while i < lines.len() && lines[i].kind == DiffKind::Remove {
                    i += 1;
                }
                let removed: Vec<&DiffLine> = lines[start..i].iter().collect();
                let adds = i;
                while i < lines.len() && lines[i].kind == DiffKind::Add {
                    i += 1;
                }
                out.push(Run::Change {
                    removed,
                    added: lines[adds..i].iter().collect(),
                });
            }
            DiffKind::Add => {
                out.push(Run::Change {
                    removed: Vec::new(),
                    added: vec![&lines[i]],
                });
                i += 1;
            }
            DiffKind::Context => {
                out.push(Run::Context(&lines[i]));
                i += 1;
            }
            _ => {
                out.push(Run::Meta(&lines[i]));
                i += 1;
            }
        }
    }
    out
}

// ---- intra-line highlighting -------------------------------------------------

/// The changed span of a one-for-one edit, as (removed spans, added spans).
///
/// Word-level over the *tokens* of the two changed fragments, which is what keeps
/// this honest: pairing whole lines would highlight the indentation and every
/// unchanged identifier around a one-word edit, which reads as "all of this
/// changed" — the opposite of the signal, and the inverse of what the reader needs
/// when the line is long and the edit is small. pi strips the leading whitespace
/// of the first changed part for the same reason (`components/diff.ts:26-66`).
struct IntraPair {
    removed: Vec<(String, Style)>,
    added: Vec<(String, Style)>,
}

fn intra_pair(removed: &[&DiffLine], added: &[&DiffLine]) -> Option<IntraPair> {
    if removed.len() != 1 || added.len() != 1 {
        return None;
    }
    let old = sanitize(&removed[0].text);
    let new = sanitize(&added[0].text);
    if old == new {
        return None;
    }
    let (o, n) = changed_spans(&old, &new);
    let inverse = Style::default().add_modifier(Modifier::REVERSED);
    let plain = Style::default();
    if o.1.is_empty() && n.1.is_empty() {
        return None;
    }
    // The shared prefix of two indented lines is the indentation plus whatever
    // identifier precedes the edit — and on a diff row the indentation is not
    // content: it is the *column* the code was at in the file, drawn here after a
    // sign and a line number, so it pushes the code right by the sum of two
    // indentations. `wrap_line` used to swallow it by accident (it collapses a
    // leading whitespace run), which is exactly why this has to be said here rather
    // than left to a wrapper's whitespace rule: the same rule also deleted the split
    // view's padding, and a layout that depends on it is a layout that breaks when
    // the rule changes. `Diff::number_width` is what keeps the columns aligned, and
    // it cannot do that if a highlight adds leading blanks of its own.
    let strip_lead = |spans: Vec<(String, Style)>| -> Vec<(String, Style)> {
        spans
            .into_iter()
            .enumerate()
            .map(|(index, (text, style))| {
                if index == 0 {
                    (text.trim_start().to_string(), style)
                } else {
                    (text, style)
                }
            })
            .collect()
    };
    Some(IntraPair {
        removed: strip_lead(vec![(o.0, plain), (o.1, inverse), (o.2, plain)]),
        added: strip_lead(vec![(n.0, plain), (n.1, inverse), (n.2, plain)]),
    })
}

/// Split two changed fragments into (prefix, changed, suffix) over whole tokens.
///
/// Whole tokens, so `let` is never "half changed" into `letx`; and the longest
/// common prefix and suffix rather than a full LCS, because the input here is two
/// *single lines* — the interesting case is a small edit in the middle of a long
/// line, and that is exactly what a prefix/suffix split isolates.
fn changed_spans(old: &str, new: &str) -> ((String, String, String), (String, String, String)) {
    let o = split_tokens(old);
    let n = split_tokens(new);
    let mut head = 0;
    while head < o.len() && head < n.len() && o[head] == n[head] {
        head += 1;
    }
    let mut tail = 0;
    while tail < o.len() - head
        && tail < n.len() - head
        && o[o.len() - 1 - tail] == n[n.len() - 1 - tail]
    {
        tail += 1;
    }
    let build = |v: &[String]| {
        (
            v[..head].concat(),
            v[head..v.len() - tail].concat(),
            v[v.len() - tail..].concat(),
        )
    };
    (build(&o), build(&n))
}

/// Split into word tokens and whitespace runs, keeping both: whitespace is what
/// makes the concatenation back into a line exact.
fn split_tokens(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_ws: Option<bool> = None;
    for ch in s.chars() {
        let ws = ch.is_whitespace();
        if cur_ws.is_some_and(|w| w != ws) {
            out.push(std::mem::take(&mut cur));
        }
        cur_ws = Some(ws);
        cur.push(ch);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Render a highlighted half: changed spans keep the side's colour and gain
/// `REVERSED`, which is visible on a terminal with no colour at all and on a
/// 16-colour one — the same reason pi uses inverse video (`diff.ts:45`).
fn emphasize(spans: &[(String, Style)], base: Style, width: usize) -> CardRow {
    let mut out: Vec<(String, Style)> = Vec::new();
    let mut left = width;
    for (text, style) in spans {
        if left == 0 {
            break;
        }
        let text = cut(text, left);
        left = left.saturating_sub(cols(&text));
        let style = if style.add_modifier.contains(Modifier::REVERSED) {
            base.add_modifier(Modifier::REVERSED)
        } else {
            base
        };
        out.push((text, style));
    }
    out
}

// ---- helpers -----------------------------------------------------------------

/// Tabs become spaces and a lone `\r` is dropped, once, before anything measures
/// a line: a tab is one `char` and up to eight columns, so a width computed over
/// raw text is wrong in the direction that wraps rows.
pub fn sanitize(text: &str) -> String {
    text.replace('\t', "   ").replace('\r', "")
}

/// Cut to `width` columns, appending `…` when something was dropped.
pub fn cut(text: &str, width: usize) -> String {
    if cols(text) <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = cols(&ch.to_string());
        if used + w > width.saturating_sub(1) {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

/// Display columns, never `chars().count()`: `界` is one `char` and two cells.
///
/// Measured over sanitized text (T-002): a tab is three columns, not one
/// `char` and not a tab stop — the same expansion the renderer draws, so a
/// width computed here can never disagree with what reaches the screen. Every
/// production caller sanitizes first (so this changes no drawn row); the
/// function is correct on its own now, not only in context.
fn cols(text: &str) -> usize {
    // The renderer's rule, not `unicode-width`'s (see [`crate::ui::layout::display_width`]).
    // This was the fourth site still measuring with the uncorrected rule, and it is the
    // one that *gates* wrapping: `render_cache` asks `row_width` whether a row needs
    // wrapping, so a row measured one column short per sound mark was accepted as
    // fitting — and then drawn twice that width and clipped by ratatui.
    crate::ui::layout::display_width(&sanitize(text))
}
#[cfg(test)]
mod tests {
    use super::{CardStatus, intra_pair, status_word};
    use crate::state::{DiffKind, DiffLine};
    use crate::ui::theme::{ASCII, UNICODE};

    /// The word matters as much as the glyph. `status_word` is documented to return both
    /// because the glyph is what a glance catches and the word is what survives a font that
    /// cannot draw it — and "partial" is the only one of these three words that tells the
    /// truth about output the harness knows it did not finish collecting.
    #[test]
    fn a_partial_result_says_partial_in_both_themes() {
        for (name, glyphs) in [("unicode", &UNICODE), ("ascii", &ASCII)] {
            let (glyph, word) = status_word(CardStatus::Partial, glyphs);
            assert_eq!(word, "partial", "{name} theme word");
            assert_ne!(
                glyph, glyphs.tick,
                "{name}: a tick would claim the output is whole"
            );
            assert_ne!(
                glyph, glyphs.cross,
                "{name}: a cross would claim the command failed"
            );
        }
    }

    /// The control: the tick and the cross must keep their own words, so `Partial` did not
    /// quietly steal one of them.
    #[test]
    fn ok_and_failed_keep_their_own_glyphs_and_words() {
        assert_eq!(status_word(CardStatus::Ok, &UNICODE).1, "ok");
        assert_eq!(status_word(CardStatus::Ok, &ASCII).0, ASCII.tick);
        assert_eq!(status_word(CardStatus::Failed, &UNICODE).1, "failed");
        assert_eq!(status_word(CardStatus::Failed, &ASCII).0, ASCII.cross);
    }

    /// **Two states, two words.** A card streaming its output used to read `pending`,
    /// the same word as a call that had not started — so a frame showed a header
    /// saying `running bash…` over a card saying `▸ bash $ cargo build · pending` for
    /// one tool. The glyph is shared on purpose (the gutter's shape does not change
    /// mid-call); the word is what tells the two apart, and it is what survives a
    /// font that cannot draw the glyph.
    #[test]
    fn pending_and_running_are_two_words_over_one_glyph() {
        for glyphs in [&UNICODE, &ASCII] {
            let (pending_glyph, pending_word) = status_word(CardStatus::Pending, glyphs);
            let (running_glyph, running_word) = status_word(CardStatus::Running, glyphs);
            assert_eq!(pending_word, "pending");
            assert_eq!(running_word, "running");
            assert_eq!(
                pending_glyph, running_glyph,
                "the gutter glyph is the same for both: only the word separates them"
            );
        }
    }

    /// A tool's output keeps its line breaks on screen (M312).    ///
    /// Found by rendering a frame with a tool card in it and reading it: a card whose body was
    /// `running 74 tests\ntest result: ok. 74 passed; 0 failed` came out as **one row**,
    /// `running 74 teststest result: ok. 74 passed; 0 failed` — two sentences run together, with
    /// the newline *gone* rather than turned into anything. `sanitize` strips it, because it
    /// strips control characters and `\n` is one.
    ///
    /// **The store already keeps the lines** (`state.rs` accumulates a bounded tail and joins it
    /// with `\n`), so this was a renderer that could not draw what it was handed — and the rows
    /// after the first were simply lost.
    #[test]
    fn a_text_card_draws_one_row_per_line_of_its_body() {
        use crate::ui::theme::Theme;
        let theme = Theme::new(crate::ui::theme::ColorMode::Ansi16, UNICODE);
        let card = super::Card {
            tool_call_id: "c1".into(),
            name: "bash".into(),
            status: CardStatus::Ok,
            body: super::CardBody::Text(
                "running 74 tests\ntest result: ok. 74 passed; 0 failed".into(),
            ),
            started_at_ms: 0,
            ended_at_ms: Some(1),
            args: serde_json::json!({}),
            head: String::new(),
        };
        let rows = super::rows(&card, false, 100, 20, &theme, Some("ctrl+x"));
        // One row is the title; the body follows it.
        assert!(
            rows.len() >= 3,
            "a two-line body must draw two rows and the title a third: {rows:?}"
        );
        let joined = rows
            .iter()
            .map(|r| {
                r.iter()
                    .map(|(s, _)| s.as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(
            joined.contains("running 74 tests") && joined.contains("test result: ok. 74 passed"),
            "both lines must be on screen, separately: {joined}"
        );
        assert!(
            !joined.contains("74 teststest result"),
            "**and the two must not be run together** — that is the defect this is about: {joined}"
        );
    }

    /// Tabs expand to three spaces, like the reference (P-164).
    ///
    /// The value matches; what was missing was the test. A tab is one
    /// `char` and up to eight columns, so a width computed over raw text
    /// is wrong in the direction that wraps rows — and a two-space or
    /// eight-space expansion would pass a test that only checked "no tabs".
    #[test]
    fn tabs_expand_to_three_spaces_before_anything_measures() {
        assert_eq!(super::sanitize("\tindented"), "   indented");
        assert_eq!(super::sanitize("a\tb"), "a   b");
        assert_eq!(super::sanitize("no tabs"), "no tabs");
    }

    /// A tab measures as three columns (T-002), not one char and not a stop.
    ///
    /// `cols` measures sanitized text, so this holds with or without a prior
    /// `sanitize` call — the function cannot disagree with the renderer.
    #[test]
    fn a_tab_measures_three_columns() {
        assert_eq!(super::cols("\t"), 3);
        assert_eq!(super::cols("\ta"), 4);
        assert_eq!(super::cols("ab"), 2);
    }

    /// An opened digest renders its markdown (P-183, markdown half).
    ///
    /// The digest is model prose — headers, lists, code spans — and the open body
    /// drew it as plain text, so structure the summarizer wrote was flattened on
    /// the one screen that shows the whole digest. RED first: every span draws
    /// in the plain body style.
    #[test]
    fn an_opened_digest_renders_its_markdown() {
        use crate::ui::theme::{ColorMode, Theme};
        let theme = Theme::new(ColorMode::Ansi16, UNICODE);
        let rows = super::event_body("# Done\n\nshipped it", true, &theme, Some("ctrl+x"));
        let text: Vec<String> = rows
            .iter()
            .map(|r| r.iter().map(|(s, _)| s.clone()).collect::<String>())
            .collect();
        assert!(
            text.iter().any(|r| r.contains("Done"))
                && text.iter().any(|r| r.contains("shipped it")),
            "markdown must not eat the words: {text:?}"
        );
        assert_ne!(
            rows[0][0].1,
            theme.text(),
            "a heading must not draw as plain body text"
        );
    }

    /// The collapsed digest still hides whole behind the affordance.
    ///
    /// The control on the markdown half: rendering the open body as markdown must
    /// not leak structure (or rows) into the closed form, which names only the
    /// count and the chord.
    #[test]
    fn a_collapsed_digest_still_hides_whole() {
        use crate::ui::theme::{ColorMode, Theme};
        let theme = Theme::new(ColorMode::Ansi16, UNICODE);
        let rows = super::event_body("# Done\n\nshipped it", false, &theme, Some("ctrl+x"));
        assert_eq!(rows.len(), 1, "one affordance row: {rows:?}");
        assert!(
            rows[0][0].0.contains("… 3 more lines (ctrl+x to open)"),
            "count plus chord, nothing else: {:?}",
            rows[0][0].0
        );
    }

    /// Intra-line highlighting fires only for a one-for-one replacement (P-162):
    /// two removals against one addition get no pairing, one-against-one does.
    #[test]
    fn intra_highlight_needs_one_removed_and_one_added_line() {
        let removed = |text: &str| DiffLine {
            kind: DiffKind::Remove,
            text: text.to_string(),
            number: None,
        };
        let added = |text: &str| DiffLine {
            kind: DiffKind::Add,
            text: text.to_string(),
            number: None,
        };
        let (r1, r2, a1) = (
            removed("let x = 1;"),
            removed("let y = 2;"),
            added("let x = 3;"),
        );
        assert!(
            intra_pair(&[&r1, &r2], &[&a1]).is_none(),
            "two-against-one must not pair"
        );
        let (r3, a2) = (removed("let x = 1;"), added("let x = 2;"));
        assert!(
            intra_pair(&[&r3], &[&a2]).is_some(),
            "one-against-one with a real change pairs"
        );
        let (r4, a3) = (removed("same;"), added("same;"));
        assert!(
            intra_pair(&[&r4], &[&a3]).is_none(),
            "identical lines are not a change"
        );
    }

    /// Leading whitespace never enters the highlight (P-163): the indent is the
    /// column the code was at, not content that changed.
    #[test]
    fn intra_highlight_excludes_leading_whitespace() {
        let removed = DiffLine {
            kind: DiffKind::Remove,
            text: "    let x = 1;".to_string(),
            number: None,
        };
        let added = DiffLine {
            kind: DiffKind::Add,
            text: "    let x = 2;".to_string(),
            number: None,
        };
        let pair = intra_pair(&[&removed], &[&added]).expect("indented one-for-one pairs");
        for (text, _) in pair.removed.iter().chain(pair.added.iter()).take(2) {
            let first_char = text.chars().next();
            assert!(
                first_char.is_none_or(|c| !c.is_whitespace()),
                "highlight must not open with whitespace, got {text:?}"
            );
        }
        let changed: String = pair.added.iter().map(|(t, _)| t.as_str()).collect();
        assert!(
            changed.contains('2'),
            "the changed token survives the strip: {changed:?}"
        );
    }
}
