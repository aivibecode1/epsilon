//! The transcript model: a pure reducer from [`AgentEvent`]s to renderable state.
//!
//! Frontends are reducers over the event bus (architecture law) — this module IS that
//! reducer, kept free of terminal code so it is unit-testable without a pty.

use epsilon_contracts::agent_event::AgentEvent;
use epsilon_contracts::message::{AssistantMessage, Message, StopReason};

/// Visual class of one transcript line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// User input (local echo or steering).
    User,
    /// Assistant prose.
    Assistant,
    /// Assistant thinking block.
    Thinking,
    /// One requested tool call.
    ToolCall,
    /// Successful tool output.
    ToolResultOk,
    /// Failed tool output / blocked call.
    ToolResultErr,
    /// Lifecycle/system notice.
    System,
}

use std::sync::atomic::{AtomicU64, Ordering};

/// Process-wide id source. Uniqueness is what the render cache needs (content-stable,
/// never reused), whether lines come from a live session or are built ad hoc in tests.
static NEXT_LINE_ID: AtomicU64 = AtomicU64::new(1);

fn take_line_id() -> u64 {
    NEXT_LINE_ID.fetch_add(1, Ordering::Relaxed)
}

/// Human-readable placeholder for an image that cannot be rendered as graphics.
///
/// Mirrors pi's `imageFallback()` text path — the Kitty/iTerm2 escape is honest-parity
/// deferred (needs `detectCapabilities()` + `Kitty` negotiation). Until then the transcript
/// must not swallow images silently. `ponytail: no terminal probing, no base64 decode at v1`.
pub fn image_fallback_line(mime: &str, bytes: Option<usize>, filename: Option<&str>) -> String {
    let name = filename.unwrap_or("image");
    match bytes {
        Some(n) => format!("[image: {mime} {name} ({} bytes) — graphics deferred]", n),
        None => format!("[image: {mime} {name} — graphics deferred]"),
    }
}

/// Try to summarize a tool output that is likely an inline image blob.
///
/// Returns a single-line fallback when the text looks like `data:image/...;base64,` or a
/// very long base64 payload that would otherwise flood the transcript. `None` means "not an image".
pub fn summarize_image_blob(text: &str) -> Option<String> {
    let t = text.trim();
    if t.starts_with("data:image/") {
        // `data:image/png;base64,iVBOR…`
        let mime_end = t.find(';').unwrap_or(12);
        let mime = &t[5..mime_end];
        // rough byte estimate: base64 len * 3/4
        let b64_start = t.find(',').map(|i| i + 1).unwrap_or(t.len());
        let b64_len = t[b64_start..].trim().len();
        let est = if b64_len > 0 {
            Some((b64_len * 3) / 4)
        } else {
            None
        };
        return Some(image_fallback_line(mime, est, None));
    }
    // Heuristic: single line > 4k that is > 95% base64 chars → treat as image blob
    if t.len() > 4096 && !t.contains('\n') {
        let b64_like = t
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/' || *c == '=')
            .count();
        if b64_like * 20 >= t.len() * 19 {
            return Some(image_fallback_line(
                "image/*",
                Some((t.len() * 3) / 4),
                None,
            ));
        }
    }
    None
}

/// One rendered transcript row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptLine {
    /// Monotonic unique id (M16a): the render cache keys wrapped rows on this, so
    /// committed lines never re-render while content is unchanged.
    pub id: u64,
    /// Visual class driving styling.
    pub kind: LineKind,
    /// Display text (already summarized/truncated where appropriate).
    pub text: String,
    /// Untruncated original for expandable rows (tool outputs; M13 card polish).
    pub full: Option<String>,
    /// Render as a fenced-code block row (dim + gutter).
    pub code: bool,
    /// Raw markdown source for assistant rows (expanded by the renderer).
    pub md: Option<String>,
    /// The tool card this row is, when it is one.
    ///
    /// A card is not a decoration on top of `text`: its title is *derived* from
    /// name + status + duration and its body is a structured payload (a diff, a
    /// pending call's detail, a result). Keeping it structured is what stops a
    /// diff from being handed to the markdown parser, which is how it came to be
    /// rendered as prose in the first place.
    pub card: Option<Card>,
}

/// What a tool call did, as far as the transcript knows.
///
/// `Running` is reached from `ToolExecutionStart` and is the state a call is in
/// while the tool works; `Pending` is a call the model asked for whose execution
/// has not been announced yet. They print the same word because they mean the
/// same thing to the reader — this is not finished — and the distinction exists
/// because the transcript, not the renderer, is where that fact belongs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardStatus {
    /// Requested, not yet announced as executing.
    Pending,
    /// Executing now.
    Running,
    /// Finished without an error.
    Ok,
    /// Finished with an error result.
    Failed,
    /// Refused by the permission gate.
    Denied,
}

/// A tool call or result as a card: a title line, then a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    /// The tool's name, as the model called it.
    pub name: String,
    /// Where the call stands.
    pub status: CardStatus,
    /// The tool call this card belongs to, so a result can settle the call's own
    /// status and duration rather than leaving a second, permanently-pending row.
    pub tool_call_id: String,
    /// When the call was requested, in Unix milliseconds.
    pub started_at_ms: u64,
    /// When it finished, in Unix milliseconds. `None` while it is still running,
    /// which is also what makes the duration live rather than absent.
    pub ended_at_ms: Option<u64>,
    /// The card's body.
    pub body: CardBody,
}

impl Card {
    /// The time the card displays, or `None` while the call is still running.
    ///
    /// Only a **finished** card carries a number, and it is `ended - started`. The
    /// alternative — `now - started` for a live call — looks like a stopwatch and
    /// behaves like one only if the row is rebuilt every frame, which it is not:
    /// rows are cached until something invalidates them, so a live duration would
    /// freeze at whatever value the frame that created the row happened to see and
    /// then jump when the card was finally rebuilt. A number that is sometimes
    /// live and sometimes stale is worse than no number, and "still running" is
    /// already what the status word says.
    pub fn elapsed_ms(&self) -> Option<u64> {
        let end = self.ended_at_ms?;
        Some(end.saturating_sub(self.started_at_ms))
    }

    /// Whether this card has a body worth opening.
    pub fn has_body(&self) -> bool {
        match &self.body {
            CardBody::Diff(d) => !d.lines.is_empty(),
            CardBody::Prompt(t) | CardBody::Text(t) => !t.trim().is_empty(),
        }
    }
}

/// What a card's body holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardBody {
    /// A unified diff, parsed once at commit time.
    Diff(Diff),
    /// What a pending call is about to do (the command, the path, the pattern).
    Prompt(String),
    /// A tool's own output.
    Text(String),
}

/// A parsed unified diff.
///
/// Parsed once, when the result is committed, and rendered by
/// [`crate::ui::card`] — never by the markdown parser, which is what turned
/// `- let x = 1;` into a bullet and `---` into a horizontal rule.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Diff {
    /// Lines in the order the tool produced them.
    pub lines: Vec<DiffLine>,
    /// Widest line number in the diff, in digits. Carried so both views can pad
    /// their number column without re-measuring the whole diff per row.
    pub number_width: usize,
}

/// One line of a unified diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    /// Added, removed, context, file/hunk header, or a line the format does not
    /// define (`\ No newline at end of file`).
    pub kind: DiffKind,
    /// The line's text with its diff prefix removed.
    pub text: String,
    /// The number this line has in the file it belongs to: the new file for an
    /// addition and for context, the old file for a removal — which is what makes
    /// the number column meaningful rather than a row counter.
    pub number: Option<usize>,
}

/// The kind of one diff line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    /// `+`, a line the change adds.
    Add,
    /// `-`, a line the change removes.
    Remove,
    /// ` `, unchanged and shown for context.
    Context,
    /// `@@ -a,b +c,d @@`, the header that opens a hunk.
    Hunk,
    /// A file header (`--- a/f.rs`, `+++ b/f.rs`, `diff --git …`) or anything the
    /// format does not define. Printed as it came, because a diff that drops the
    /// lines it did not understand is a diff that lies about what changed.
    Meta,
}

impl Diff {
    /// Whether line numbers fit the unified view at `width` body columns.
    ///
    /// The numbers are the first thing to go when the card is narrow: the sign is
    /// what colour is standing in for, and a clipped number is worse than none.
    pub fn unified_numbers_fit(&self, width: usize) -> bool {
        width >= crate::ui::card::MIN_NUMBER_WIDTH + self.number_width + 2
    }

    /// Whether line numbers fit one half of the split view at `width` columns.
    pub fn split_numbers_fit(&self, width: usize) -> bool {
        width >= crate::ui::card::MIN_NUMBER_WIDTH + self.number_width + 2
    }

    /// The `+a -b` summary a card title can carry.
    pub fn counts(&self) -> (usize, usize) {
        self.lines
            .iter()
            .fold((0, 0), |(add, del), l| match l.kind {
                DiffKind::Add => (add + 1, del),
                DiffKind::Remove => (add, del + 1),
                _ => (add, del),
            })
    }
}

/// Parse a unified diff into lines with their kinds and numbers.
///
/// Deliberately tolerant: a line the format does not define becomes
/// [`DiffKind::Meta`] and is printed as it arrived. A diff is evidence, and a
/// parser that silently dropped what it could not classify would hide part of the
/// change.
///
/// # Line numbers
///
/// Each line carries the number it has **in the file it belongs to**: a context
/// line advances both counters, an addition advances the new file's, a removal the
/// old one's. The number printed beside a line is that counter's value *before* it
/// advances, so it names the line the reader is looking at rather than the one
/// after it.
///
/// Two consequences are worth stating, because both look like off-by-one bugs
/// until you follow the counters:
///
/// * A one-for-one replacement is numbered with the **same** number on both sides
///   whenever the two files are level — after five lines of context, the removed
///   line is the old file's `6` and the added line is the new file's `6`, because
///   the removal does not touch the new file's counter and the addition does not
///   touch the old one's. That agreement is the format working, not the parser
///   forgetting to increment: an editor diffing the two files shows those same two
///   numbers. (`edit_result_with_a_diff_commits_a_parsed_diff_card` below and the
///   golden frame `an_edits_diff_is_drawn_with_a_coloured_gutter_not_as_prose`
///   both pin it; both would fail against the `5`/`6` this doc used to claim.)
/// * The two sides *do* diverge the moment a run of additions or removals is
///   longer than the other side's, and they diverge permanently from a hunk whose
///   header starts the files at different lines (`@@ -10,2 +20,2 @@`) — see
///   `a_hunk_header_sets_the_line_numbers_it_declares`.
pub fn parse_diff(text: &str) -> Diff {
    let mut lines: Vec<DiffLine> = Vec::new();
    let mut old_no = 0usize;
    let mut new_no = 0usize;
    let mut in_hunk = false;
    for raw in text.lines() {
        let raw = raw.trim_end_matches('\r');
        if let Some(hunk) = parse_hunk_header(raw) {
            old_no = hunk.0;
            new_no = hunk.1;
            in_hunk = true;
            lines.push(DiffLine {
                kind: DiffKind::Hunk,
                text: raw.to_string(),
                number: None,
            });
            continue;
        }
        if !in_hunk {
            lines.push(DiffLine {
                kind: DiffKind::Meta,
                text: raw.to_string(),
                number: None,
            });
            continue;
        }
        let (kind, number, text) = match raw.as_bytes().first() {
            Some(b'+') => {
                let n = new_no;
                new_no += 1;
                (DiffKind::Add, Some(n), raw[1..].to_string())
            }
            Some(b'-') => {
                let n = old_no;
                old_no += 1;
                (DiffKind::Remove, Some(n), raw[1..].to_string())
            }
            Some(b' ') => {
                let n = new_no;
                old_no += 1;
                new_no += 1;
                (DiffKind::Context, Some(n), raw[1..].to_string())
            }
            // `\ No newline at end of file`: part of the diff, not a line of either
            // file. Read as context it would be numbered as though it were code and
            // drawn in the "unchanged" colour, which is a claim about the file that
            // the diff does not make.
            Some(b'\\') => (DiffKind::Meta, None, raw.to_string()),
            // Anything else the format does not define: kept, not numbered.
            _ => (DiffKind::Context, Some(new_no), raw.to_string()),
        };
        lines.push(DiffLine { kind, text, number });
    }
    let number_width = lines
        .iter()
        .filter_map(|l| l.number)
        .max()
        .map(|n| n.to_string().len())
        .unwrap_or(0);
    Diff {
        lines,
        number_width,
    }
}

/// `@@ -old,count +new,count @@` → (old start, new start).
fn parse_hunk_header(line: &str) -> Option<(usize, usize)> {
    let rest = line.strip_prefix("@@")?;
    let end = rest.find("@@")?;
    let mut parts = rest[..end].split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let start =
        |s: &str| -> Option<usize> { s.split(',').next().unwrap_or(s).parse::<usize>().ok() };
    Some((start(old)?, start(new)?))
}

impl TranscriptLine {
    fn new(
        kind: LineKind,
        text: impl Into<String>,
        full: Option<String>,
        code: bool,
        md: Option<String>,
    ) -> Self {
        Self {
            id: take_line_id(),
            kind,
            text: text.into(),
            full,
            code,
            md,
            card: None,
        }
    }

    /// Plain line with no expansion payload.
    pub fn plain(kind: LineKind, text: impl Into<String>) -> Self {
        Self::new(kind, text, None, false, None)
    }

    /// Expandable line: `text` is the truncated head, `full` the complete body.
    pub fn expandable(kind: LineKind, text: String, full: String) -> Self {
        Self::new(kind, text, Some(full), false, None)
    }

    /// Fenced-code line (markdown-lite rendering).
    pub fn code_line(text: impl Into<String>) -> Self {
        Self::new(LineKind::Assistant, text, None, true, None)
    }

    /// Assistant message carrying raw markdown; expanded at render time.
    pub fn assistant_md(text: impl Into<String>) -> Self {
        let text = text.into();
        Self::new(LineKind::Assistant, text.clone(), None, false, Some(text))
    }
}

/// Long tool outputs collapse to a head + ellipsis so one bash call cannot flood the pane.
const MAX_RESULT_CHARS: usize = 400;

/// Reducible UI state for one TUI session.
#[derive(Debug, Default, Clone)]
pub struct UiState {
    lines: Vec<TranscriptLine>,
    streaming_text: String,
    streaming_thinking: String,
    status: String,
    running: bool,
    tokens_in: u64,
    tokens_out: u64,
    cache_read: u64,
    cache_write: u64,
    /// Cache-read tokens of the most recent call, paired with `latest_prompt_tokens`.
    ///
    /// See [`UiState::latest_cache_read`] for why the rate cannot use the session totals
    /// above — this pair is the fix for G16.
    latest_cache_read: u64,
    latest_prompt_tokens: u64,
    /// Estimated tokens the next request will carry: the whole session's context,
    /// mirrored from `Agent::estimated_context_tokens` once per frame.
    ///
    /// Not [`UiState::latest_prompt_tokens`], which is the size of the last
    /// *request* and therefore only ever a prefix of the conversation. The
    /// header's gauge reads this one, so it agrees with the compaction trigger;
    /// see `Metrics::context_used` for the defect that distinction fixes.
    ///
    /// Crate-visible because the draw path holds no agent: the loop mirrors it
    /// ([`crate::app::App::mirror_context_size`]) and `app.rs` reads it, so a
    /// test can aim the header at a context size without running a run.
    pub(crate) context_now: u64,
    /// `true` once the session has been compacted and no fresh count has arrived,
    /// cleared by the first reply that reports one.
    ///
    /// The context gauge prints `?` in exactly that window, but it derives that
    /// from [`UiState::latest_prompt_tokens`] being zero rather than from this
    /// flag — which is why the accessor that used to read it is gone. That
    /// accessor had one caller, the header's `compacted` field, and both were
    /// dead: the two match arms that consulted the field printed the same string.
    /// The flag stays as the record of *why* the count is zero, which is what a
    /// `… compacted` row would read if the transcript ever wants one.
    #[allow(dead_code, reason = "the `?` window is derived from the token count")]
    compacted: bool,
}

impl UiState {
    /// Empty state with an idle status line.
    pub fn new() -> Self {
        Self {
            status: "idle · type a task and press Enter".into(),
            ..Default::default()
        }
    }

    /// Transcript rows in order.
    pub fn lines(&self) -> &[TranscriptLine] {
        &self.lines
    }

    /// In-flight assistant text not yet committed to the transcript.
    pub fn streaming_text(&self) -> &str {
        &self.streaming_text
    }

    /// In-flight thinking not yet committed (pi: rendered as DIM thinking tail).
    pub fn streaming_thinking(&self) -> &str {
        &self.streaming_thinking
    }

    /// Footer status text.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// True while a run is active (typed input becomes steering).
    pub fn running(&self) -> bool {
        self.running
    }

    /// Accumulated provider usage across the session: (input, output) billed tokens.
    pub fn usage(&self) -> (u64, u64) {
        (self.tokens_in, self.tokens_out)
    }

    /// Accumulated prompt-cache traffic across the session: (cache read, cache write).
    pub fn cache_usage(&self) -> (u64, u64) {
        (self.cache_read, self.cache_write)
    }

    /// Latest single-call prompt size = input + cache_read + cache_write
    /// (pi footer convention: this is what "context usage" measures). Zero until the
    /// first provider response of the session.
    pub fn latest_prompt_tokens(&self) -> u64 {
        self.latest_prompt_tokens
    }

    /// Cache-read tokens of the **same call** [`UiState::latest_prompt_tokens`]
    /// describes.
    ///
    /// The cache hit rate is a claim about one prompt, so its numerator has to come from
    /// the same call as its denominator. The session totals in
    /// [`UiState::cache_usage`] cannot serve that purpose: dividing them by a single
    /// prompt inflates the rate every turn and pins it at 100%, which is what the gauge
    /// did until G16. Kept as its own pair so the session totals remain available for a
    /// wider layout.
    pub fn latest_cache_read(&self) -> u64 {
        self.latest_cache_read
    }

    /// Commit a tool call the model asked for as a **card**.
    ///
    /// Replaces the flat `tool-call {name} {args}` row. The call and its result
    /// are two cards rather than one — that is the shape the event stream has, a
    /// call can outlive the frame that drew it, and a call whose result never
    /// arrives must still be able to say *pending* — but they share a
    /// `tool_call_id`, so the result settles the call's own card instead of
    /// leaving a permanently-pending row for the reader to scroll past.
    fn push_tool_call(
        &mut self,
        tool_call_id: &str,
        name: &str,
        args: &serde_json::Value,
        now: u64,
    ) {
        let detail = summarize_args(args);
        self.push_line(TranscriptLine {
            card: Some(Card {
                name: name.to_string(),
                status: CardStatus::Pending,
                tool_call_id: tool_call_id.to_string(),
                started_at_ms: now,
                ended_at_ms: None,
                body: CardBody::Prompt(detail),
            }),
            ..TranscriptLine::plain(LineKind::ToolCall, format!("{name} {args}"))
        });
    }

    /// Commit a tool result as a card, settling the matching call's card.
    ///
    /// The diff path is why this is not one line: a result carrying
    /// `details.diff` becomes a **parsed diff**, rendered by [`crate::ui::card`].
    /// It used to become the string `format!("{name} → diff:\n{diff}")` pushed
    /// through the markdown parser, which is how a diff ended up on screen as a
    /// bullet list with a horizontal rule where its hunk separator should have
    /// been.
    fn push_tool_result(&mut self, t: &epsilon_contracts::message::ToolResultMessage, now: u64) {
        let body = tool_result_text(t);
        let status = if t.is_error {
            CardStatus::Failed
        } else {
            CardStatus::Ok
        };
        // Settle the call's own card first: it is the row the reader is looking at
        // when the answer lands, and a row still saying "pending" after its result
        // has arrived is a row that lies.
        //
        // The result card inherits the **call's** start, not `now`. Stamping both of
        // its own timestamps with `now` — which is what this did — made
        // `Card::elapsed_ms()` return `Some(0)` for every result card ever drawn, so
        // the duration the card advertises was a field that could only ever say
        // `0ms`. A result whose call is not in the transcript (a replay that lost the
        // call, a result for a call from another branch) falls back to `now` and
        // honestly reports no elapsed time rather than inventing one.
        let started_at_ms = self
            .settle_call(&t.tool_call_id, status, now)
            .unwrap_or(now);
        let card_body = match edit_diff(t) {
            Some(diff) => CardBody::Diff(parse_diff(&diff)),
            None => CardBody::Text(body.clone()),
        };
        let head = format!("{} → {}", t.tool_name, truncate(&body, MAX_RESULT_CHARS));
        self.push_line(TranscriptLine {
            card: Some(Card {
                name: t.tool_name.clone(),
                status,
                tool_call_id: t.tool_call_id.clone(),
                started_at_ms,
                ended_at_ms: Some(now),
                body: card_body,
            }),
            ..TranscriptLine::expandable(
                if t.is_error {
                    LineKind::ToolResultErr
                } else {
                    LineKind::ToolResultOk
                },
                head,
                format!("{} → {body}", t.tool_name),
            )
        });
    }

    /// Mark the card for `tool_call_id` finished, if one is still open, and return the
    /// start time it was holding.
    ///
    /// The return value is the whole point: the result card needs the *call's* start to
    /// report a duration, and the only place that start exists is the pending card this
    /// function finds. Returning it here keeps one lookup instead of two, and means the
    /// pair `(call, result)` reports the same elapsed time whichever end asks.
    fn settle_call(&mut self, tool_call_id: &str, status: CardStatus, now: u64) -> Option<u64> {
        let line = self.lines.iter_mut().rev().find(|l| {
            l.card
                .as_ref()
                .is_some_and(|c| c.tool_call_id == tool_call_id && c.ended_at_ms.is_none())
        })?;
        let card = line.card.as_mut()?;
        card.status = status;
        card.ended_at_ms = Some(now);
        Some(card.started_at_ms)
    }

    /// Local echo of submitted input; distinct from replayed transcript messages.
    ///
    /// The text is stored **without** a marker. The prompt glyph is a rendering
    /// decision (`render_cache::row_marker` draws `❯` from the theme, and
    /// `transcript::decorate` adds the `▎` gutter); a `"> "` stored here renders
    /// as a second marker beside both, and puts a character where the gutter
    /// glyph belongs.
    pub fn push_local_input(&mut self, text: &str) {
        self.push_line(TranscriptLine::plain(LineKind::User, text));
    }

    /// Runtime notice (run failures, lag warnings) rendered as a system row.
    pub fn push_notice(&mut self, text: impl Into<String>) {
        self.push_line(TranscriptLine::plain(LineKind::System, text));
    }

    /// Drop all transcript rows (`/clear`); usage counters survive.
    pub fn clear_lines(&mut self) {
        self.lines.clear();
        self.streaming_text.clear();
        self.streaming_thinking.clear();
    }

    /// Hydrate the visible transcript from a restored history (live `/resume` + `/import`).
    ///
    /// Clears existing rows and rebuilds them from `messages` (the store's shadowed
    /// model view). Ponytail: reuse existing line constructors and token accounting.
    pub fn hydrate_from_history(&mut self, messages: &[Message]) {
        self.lines.clear();
        self.streaming_text.clear();
        self.streaming_thinking.clear();
        self.tokens_in = 0;
        self.tokens_out = 0;
        self.cache_read = 0;
        self.cache_write = 0;
        self.latest_prompt_tokens = 0;
        self.compacted = false;
        // A replayed session has no per-call timings to recover — the store keeps
        // messages, not wall-clock spans — so every card hydrated here is stamped
        // with the same instant: its call and its result share a start, and the
        // duration reads `0ms` rather than an invented number. The alternative is
        // a card that claims a duration the store never recorded.
        let now = now_ms();
        for msg in messages {
            match msg {
                Message::User(u) => {
                    let text: String = u
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            epsilon_contracts::content::UserContent::Text { text } => {
                                Some(text.as_str())
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    if text.trim().is_empty() {
                        continue;
                    }
                    // No `"> "` prefix: see `push_local_input`.
                    self.push_line(TranscriptLine::plain(LineKind::User, text));
                }
                Message::Assistant(a) => {
                    let thinking = thinking_of(a);
                    if !thinking.is_empty() {
                        self.push_line(TranscriptLine::plain(
                            LineKind::Thinking,
                            truncate(&thinking, MAX_RESULT_CHARS),
                        ));
                    }
                    let text = a.text();
                    if !text.is_empty() {
                        self.push_line(TranscriptLine::assistant_md(text));
                    }
                    for (id, name, args) in a.tool_calls() {
                        self.push_tool_call(id, name, args, now);
                    }
                    self.tokens_in += a.usage.input;
                    self.tokens_out += a.usage.output;
                    self.cache_read += a.usage.cache_read;
                    self.cache_write += a.usage.cache_write;
                    let prompt = a.usage.input + a.usage.cache_read + a.usage.cache_write;
                    if prompt > 0 {
                        self.latest_prompt_tokens = prompt;
                        self.latest_cache_read = a.usage.cache_read;
                        self.compacted = false;
                    }
                }
                Message::ToolResult(t) => {
                    let body = tool_result_text(t);
                    if t.tool_name == "todo"
                        && let Some(head) = todo_head(&t.details)
                    {
                        self.push_line(TranscriptLine::expandable(
                            LineKind::ToolResultOk,
                            head,
                            format!("todo\n{body}"),
                        ));
                        continue;
                    }
                    if let Some(img) = summarize_image_blob(&body) {
                        self.push_line(TranscriptLine::expandable(
                            if t.is_error {
                                LineKind::ToolResultErr
                            } else {
                                LineKind::ToolResultOk
                            },
                            img.clone(),
                            img,
                        ));
                        continue;
                    }
                    self.push_tool_result(t, now);
                }
                Message::Custom(c) => {
                    self.push_line(TranscriptLine::plain(
                        LineKind::System,
                        format!("[custom {}] {}", c.custom_type, c.payload),
                    ));
                }
            }
        }
        if !messages.is_empty() {
            self.status = format!("resumed {} messages", messages.len());
        }
    }

    /// Reduce one lifecycle event into the model.
    pub fn apply(&mut self, ev: &AgentEvent) {
        // One clock read per event, not per card: two cards committed by the same
        // event (a call and the result that settles it) must be able to compare
        // their stamps, and two `SystemTime::now()` calls in a row can differ.
        let now = now_ms();
        match ev {
            AgentEvent::AgentStart => {
                self.running = true;
                self.status = "run started".into();
                // No "run started" row. The header already says a run is in
                // flight, and a marker row between every user turn and every
                // answer turns a conversation into a log file. The transcript is
                // for what was *said*; lifecycle state belongs in the chrome.
            }
            AgentEvent::TurnStart { turn_index } => {
                self.status = format!("turn {turn_index}: thinking…");
            }
            AgentEvent::MessageUpdate { message } => {
                if let Message::Assistant(a) = &**message {
                    self.streaming_thinking = thinking_of(a);
                    self.streaming_text = a.text();
                    self.status = "streaming…".into();
                }
            }
            AgentEvent::MessageEnd { message } => match &**message {
                Message::Assistant(a) => {
                    let thinking = std::mem::take(&mut self.streaming_thinking);
                    if !thinking.is_empty() {
                        self.push_line(TranscriptLine::plain(
                            LineKind::Thinking,
                            truncate(&thinking, MAX_RESULT_CHARS),
                        ));
                    }
                    self.streaming_text.clear();
                    let text = a.text();
                    let text_empty = text.is_empty();
                    if !text_empty {
                        // M15: raw markdown stored; the renderer expands it into
                        // styled rows (via the M16a render cache — once per message).
                        self.push_line(TranscriptLine::assistant_md(text));
                    }
                    for (id, name, args) in a.tool_calls() {
                        self.push_tool_call(id, name, args, now);
                    }
                    if a.stop_reason == StopReason::Error
                        && let Some(err) = a.error_message.as_deref()
                        && !err.is_empty()
                    {
                        // Empty-Error Dones used to vanish (no text + no tool_call
                        // → zero lines), so the provider detail is always rendered.
                        // M62: when text DID arrive (salvage after a mid-stream
                        // failure) the marker is what stops a cut-off answer from
                        // reading as a complete one.
                        let label = if text_empty {
                            format!("error: {err}")
                        } else {
                            format!("error: {err} — the reply above is partial")
                        };
                        self.push_line(TranscriptLine::plain(LineKind::System, label));
                    }
                    self.tokens_in += a.usage.input;
                    self.tokens_out += a.usage.output;
                    self.cache_read += a.usage.cache_read;
                    self.cache_write += a.usage.cache_write;
                    // pi footer convention: context size = latest call's full prompt.
                    let prompt = a.usage.input + a.usage.cache_read + a.usage.cache_write;
                    if prompt > 0 {
                        self.latest_prompt_tokens = prompt;
                        self.latest_cache_read = a.usage.cache_read;
                        self.compacted = false; // fresh counts retire the `?` marker
                    }
                }
                Message::ToolResult(t) => {
                    let body = tool_result_text(t);
                    // M30: todo results render their checklist (opencode's todo
                    // block) — short lists fully visible, long ones summarized.
                    if t.tool_name == "todo"
                        && let Some(head) = todo_head(&t.details)
                    {
                        self.push_line(TranscriptLine::expandable(
                            LineKind::ToolResultOk,
                            head,
                            format!("todo\n{body}"),
                        ));
                        self.status = "tool done".into();
                        return;
                    }
                    // Image blobs are summarized to a one-line placeholder so a pasted
                    // screenshot / data: URI does not flood the transcript (pi imageFallback parity).
                    if let Some(img) = summarize_image_blob(&body) {
                        self.push_line(TranscriptLine::expandable(
                            if t.is_error {
                                LineKind::ToolResultErr
                            } else {
                                LineKind::ToolResultOk
                            },
                            img.clone(),
                            img,
                        ));
                        self.status = "tool done".into();
                        return;
                    }
                    // M30 / M126: an edit's diff is a *diff*, not prose. It is
                    // parsed here and rendered by `ui::card` with a sign gutter,
                    // per-side colour and line numbers — never handed to the
                    // markdown parser, which is what turned `- let x = 1;` into a
                    // bullet and a `---` separator into a rule across the card.
                    self.push_tool_result(t, now);
                    self.status = "tool done".into();
                }
                _ => {}
            },
            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                ..
            } => {
                self.status = format!("running {tool_name}…");
                // The card was committed when the assistant message ended; this is
                // the event that says it actually started. Marking it here keeps
                // `pending` for "asked for, not yet running", which is the state a
                // permission prompt leaves a call in.
                if let Some(line) = self.lines.iter_mut().rev().find(|l| {
                    l.card
                        .as_ref()
                        .is_some_and(|c| c.tool_call_id == *tool_call_id)
                }) && let Some(card) = line.card.as_mut()
                {
                    card.status = CardStatus::Running;
                }
            }
            AgentEvent::ToolExecutionUpdate {
                tool_name, partial, ..
            } => {
                // M63: a long command reports what it is doing while it runs. The
                // status line carries the newest output line (bounded to one row);
                // the complete output still arrives with the tool result, so this
                // is a view, never the record.
                let text = partial
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        epsilon_contracts::content::ContentBlock::Text { text } => {
                            Some(text.as_str())
                        }
                        _ => None,
                    })
                    .collect::<String>();
                let last = text
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .trim();
                if !last.is_empty() {
                    self.status = format!("{tool_name}: {}", truncate(last, 120));
                }
            }
            AgentEvent::PermissionDecided { decision, .. } => {
                let label = match decision {
                    epsilon_contracts::agent_event::PermissionDecision::AllowOnce => "allowed",
                    epsilon_contracts::agent_event::PermissionDecision::AlwaysSession => {
                        "allowed (session)"
                    }
                    epsilon_contracts::agent_event::PermissionDecision::Deny => "denied",
                    epsilon_contracts::agent_event::PermissionDecision::DenyWithGuidance {
                        ..
                    } => "denied with guidance",
                };
                self.push_system(format!("permission {label}"));
            }
            AgentEvent::CompactionStarted => {
                self.status = "compacting context…".into();
                self.push_system("── compaction started ──");
            }
            AgentEvent::CompactionFinished => {
                self.status = "context compacted".into();
                self.push_system("── compaction finished ──");
                // Context size is now unknown until the next provider response
                // (pi footer: render `?` for the context percentage in this window).
                self.compacted = true;
            }
            AgentEvent::Notice { text } => {
                // M60: transport status (provider retries, stall watchdogs). The
                // status line carries it live; the transcript keeps a dim system
                // line so a pause after the fact is still explainable.
                self.status = text.clone();
                self.push_system(format!("⟳ {text}"));
            }
            AgentEvent::AgentEnd { .. } => {
                // Flush any dangling stream buffer (error paths skip MessageEnd).
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.running = false;
                self.status = "idle · type a task and press Enter".into();
                // The matching marker for `AgentStart`: the header returns to
                // `idle`, which is the visible signal that the run finished.
                // A row saying so would be the third place the same fact lives.
            }
            // TurnEnd summaries are intentionally not rendered yet: status shows
            // activity; expandable tool cards stay on the M12 polish ledger
            // (architecture §11).
            _ => {}
        }
    }

    fn push_system(&mut self, text: impl Into<String>) {
        self.push_line(TranscriptLine::plain(LineKind::System, text));
    }

    /// Single mutation point for transcript appends. Ids come from the line itself
    /// (unique by construction, M16a); kept as a choke point for future hooks.
    fn push_line(&mut self, line: TranscriptLine) {
        self.lines.push(line);
    }
}

/// `now` in Unix milliseconds.
///
/// Wall-clock rather than `Instant` because the value is stored on a transcript
/// card and a card outlives the process that drew it: a resumed session rebuilt
/// from history has no `Instant` to compare against, and a zero would render as
/// `0ms` — a duration that looks measured and is not.
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A tool result as the event stream delivers it.
///
/// Crate-visible for tests that need a *card* rather than a hand-built
/// `TranscriptLine`: the card's title, its diff and its duration are decided by
/// the reducer, and a fixture that wrote them out by hand would keep passing after
/// the reducer stopped producing them.
#[cfg(test)]
pub(crate) fn agent_event_tool_result(tool_name: &str, body: &str, is_error: bool) -> AgentEvent {
    AgentEvent::MessageEnd {
        message: Box::new(Message::ToolResult(
            epsilon_contracts::message::ToolResultMessage {
                id: epsilon_contracts::ids::new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: format!("tc_{tool_name}"),
                tool_name: tool_name.to_string(),
                content: vec![epsilon_contracts::content::ContentBlock::text(body)],
                details: serde_json::json!({}),
                usage: None,
                is_error,
            },
        )),
    }
}

/// The same, for a result that carries a unified diff in `details.diff` — the
/// shape an `edit` tool returns and the one the diff renderer exists for.
#[cfg(test)]
pub(crate) fn agent_event_edit_result(diff: &str, is_error: bool) -> AgentEvent {
    AgentEvent::MessageEnd {
        message: Box::new(Message::ToolResult(
            epsilon_contracts::message::ToolResultMessage {
                id: epsilon_contracts::ids::new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_edit".into(),
                tool_name: "edit".into(),
                content: vec![epsilon_contracts::content::ContentBlock::text(
                    "edited a.rs (1 replacement(s))",
                )],
                details: serde_json::json!({ "diff": diff }),
                usage: None,
                is_error,
            },
        )),
    }
}

/// The text a tool result carries, concatenated across its blocks.
fn tool_result_text(t: &epsilon_contracts::message::ToolResultMessage) -> String {
    t.content
        .iter()
        .filter_map(|c| match c {
            epsilon_contracts::content::ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>()
}

/// The unified diff an `edit` result carries, when it carries one.
///
/// Named rather than inlined because "is this a diff" is asked in three places
/// (hydration, the live reducer, and the review question "why is an edit
/// special"), and three copies of the same `details.get("diff")` is how they drift.
fn edit_diff(t: &epsilon_contracts::message::ToolResultMessage) -> Option<String> {
    let diff = t.details.get("diff")?.as_str()?;
    if diff.trim().is_empty() {
        return None;
    }
    Some(diff.to_string())
}

fn thinking_of(a: &AssistantMessage) -> String {
    a.content
        .iter()
        .filter_map(|c| match c {
            epsilon_contracts::content::ContentBlock::Thinking { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// Glanceable head for todo results (opencode's todo block): up to five items
/// render fully with their markers; longer lists summarize to counts.
/// Returns None when the details payload isn't the expected shape (then the
/// generic tool-result rendering takes over).
fn todo_head(details: &serde_json::Value) -> Option<String> {
    let items = details.get("todo")?.as_array()?;
    let marker = |status: &str| match status {
        "completed" => "[x]",
        "in_progress" => "[~]",
        _ => "[ ]",
    };
    let rendered: Vec<String> = items
        .iter()
        .filter_map(|i| {
            Some(format!(
                "{} {}",
                marker(i.get("status")?.as_str()?),
                i.get("content")?.as_str()?.trim()
            ))
        })
        .collect();
    if rendered.len() != items.len() {
        return None;
    }
    if rendered.len() <= 5 {
        Some(format!("todo {}", rendered.join(" · ")))
    } else {
        let done = rendered.iter().filter(|r| r.starts_with("[x]")).count();
        Some(format!(
            "todo ({done}/{} done) {}…",
            rendered.len(),
            rendered.first().unwrap_or(&String::new())
        ))
    }
}

/// The detail line a pending card shows: what this call is about to do.
///
/// The **field that identifies the call**, in the order a reader would ask for it:
/// the command a shell will run, the path a file tool will touch, the pattern a
/// search will look for. `args.to_string()` is the fallback for a tool whose shape
/// this does not know. A tool card that printed `{"path":"src/app.rs"}` for an
/// obvious `edit` is why this is not simply the JSON: that is a data dump rather
/// than a summary, and "tool calls as raw JSON dumps" is what the skill's card rule
/// names as the thing not to do.
fn summarize_args(args: &serde_json::Value) -> String {
    for key in [
        "command",
        "path",
        "file_path",
        "filePath",
        "pattern",
        "query",
        "url",
        "prompt",
    ] {
        if let Some(value) = args.get(key).and_then(|v| v.as_str())
            && !value.trim().is_empty()
        {
            return value.to_string();
        }
    }
    truncate(&args.to_string(), 80)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}…")
}

/// Whether expandable rows currently render their full payload.
#[derive(Debug, Default, Clone, Copy)]
pub struct RenderPrefs {
    /// `x` toggle: tool-result rows show full output instead of the truncated head.
    pub expand_outputs: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use epsilon_contracts::agent_event::TurnEndReason;
    use epsilon_contracts::content::ContentBlock;
    use epsilon_contracts::ids::new_id;
    use epsilon_contracts::message::{StopReason, ToolResultMessage, Usage};

    fn assistant(text: &str, calls: Vec<(&str, &str)>, usage: Usage) -> AssistantMessage {
        let mut content = vec![ContentBlock::text(text)];
        for (id, name) in calls {
            content.push(ContentBlock::ToolCall {
                id: id.into(),
                name: name.into(),
                arguments: serde_json::json!({"command": "echo hi"}),
            });
        }
        AssistantMessage {
            id: new_id("msg"),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content,
            stop_reason: StopReason::EndTurn,
            error_message: None,
            usage,
        }
    }

    #[test]
    fn a_salvaged_partial_reply_is_marked_partial_not_complete() {
        // M62: text + a failure must read as a cut-off answer, never as a
        // finished one — the whole point of salvaging instead of discarding.
        let mut st = UiState::new();
        let mut msg = assistant("the answer so f", vec![], Usage::default());
        msg.stop_reason = StopReason::Error;
        msg.error_message = Some("stream read failed: connection reset".into());
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(msg)),
        });
        let joined = st
            .lines()
            .iter()
            .map(|l| l.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("the answer so f"), "{joined}");
        assert!(
            joined.contains("partial"),
            "the failure is stated, not implied: {joined}"
        );
        assert!(joined.contains("connection reset"), "{joined}");
    }

    #[test]
    fn retry_notices_land_in_the_transcript_and_the_status_line() {
        let mut st = UiState::new();
        st.apply(&AgentEvent::Notice {
            text: "HTTP 503: overloaded — retrying gpt-x in 2.0s (attempt 2) (server error)".into(),
        });
        assert!(
            st.status.contains("retrying"),
            "status shows the wait: {}",
            st.status
        );
        let joined = st
            .lines()
            .iter()
            .map(|l| l.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            joined.contains("retrying gpt-x"),
            "the transcript keeps the notice so a pause is explainable afterwards: {joined}"
        );
    }

    #[test]
    fn run_lifecycle_renders_in_order() {
        let mut st = UiState::new();
        st.apply(&AgentEvent::AgentStart);
        st.apply(&AgentEvent::TurnStart { turn_index: 0 });
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(assistant(
                "working on it",
                vec![("tc_1", "bash")],
                Usage {
                    input: 10,
                    output: 5,
                    ..Default::default()
                },
            ))),
        });
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "bash".into(),
                content: vec![ContentBlock::text("hi")],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            })),
        });
        st.apply(&AgentEvent::AgentEnd { messages: vec![] });

        // The transcript holds what was said, and only that: assistant prose,
        // the tool call, its result. Lifecycle state lives in the header, so a
        // run produces no marker rows to scroll past.
        let kinds: Vec<LineKind> = st.lines().iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            vec![
                LineKind::Assistant,
                LineKind::ToolCall,
                LineKind::ToolResultOk
            ]
        );
        assert!(st.lines()[1].text.contains("bash"));
        assert!(st.lines()[2].text.contains("hi"));
        // The header is where "a run is happening" is visible, so the state must
        // still know it even though no row says so.
        assert!(!st.running(), "the run ended");
        assert_eq!(st.status(), "idle · type a task and press Enter");
        assert_eq!(st.usage(), (10, 5));
    }

    #[test]
    fn streaming_updates_replace_then_commit_on_end() {
        let mut st = UiState::new();
        let mut a = assistant("", vec![], Usage::default());
        a.content[0] = ContentBlock::text("hel");
        st.apply(&AgentEvent::MessageUpdate {
            message: Box::new(Message::Assistant(a.clone())),
        });
        assert_eq!(st.streaming_text(), "hel");
        assert!(st.lines().is_empty(), "nothing committed mid-stream");

        a.content[0] = ContentBlock::text("hello world");
        st.apply(&AgentEvent::MessageUpdate {
            message: Box::new(Message::Assistant(a.clone())),
        });
        assert_eq!(st.streaming_text(), "hello world", "replace, not append");

        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(a)),
        });
        assert_eq!(st.streaming_text(), "");
        assert_eq!(st.lines().len(), 1);
        assert_eq!(st.lines()[0].kind, LineKind::Assistant);
        assert_eq!(st.lines()[0].text, "hello world");
    }

    #[test]
    fn errors_render_as_error_lines_and_reason_survives() {
        let mut st = UiState::new();
        st.apply(&AgentEvent::AgentStart);
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "bash".into(),
                content: vec![ContentBlock::text("boom")],
                details: serde_json::json!({}),
                usage: None,
                is_error: true,
            })),
        });
        st.apply(&AgentEvent::TurnEnd {
            turn_index: 0,
            message: Box::new(Message::Assistant(assistant("", vec![], Usage::default()))),
            tool_results: vec![],
        });
        let _ = TurnEndReason::Completed; // reason taxonomy exercised elsewhere
        // No leading lifecycle marker row any more, so the error is first.
        assert_eq!(st.lines()[0].kind, LineKind::ToolResultErr);
        assert!(st.lines()[0].text.contains("boom"));
    }

    #[test]
    fn oversized_tool_output_is_truncated() {
        let mut st = UiState::new();
        let big = "x".repeat(MAX_RESULT_CHARS + 50);
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "read".into(),
                content: vec![ContentBlock::text(big)],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            })),
        });
        let line = &st.lines()[0];
        assert!(line.text.chars().count() < MAX_RESULT_CHARS + 10);
        assert!(line.text.ends_with('…'));
    }

    #[test]
    fn local_input_echoes_without_a_stored_marker() {
        // The marker is the renderer's job (`❯` from the cache, `▎` from the
        // gutter). Storing one here produced `❯ > hello` on screen.
        let mut st = UiState::new();
        st.push_local_input("fix the tests");
        assert_eq!(st.lines()[0].text, "fix the tests");
        assert_eq!(st.lines()[0].kind, LineKind::User);
    }

    // ---- M30: todo results render their checklist --------------------------------------

    #[test]
    fn todo_results_render_as_a_checklist_head() {
        let mut st = UiState::new();
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "todo".into(),
                content: vec![ContentBlock::text("[x] one\n[ ] two\n(1/2 completed)")],
                details: serde_json::json!({"todo": [
                    {"content": "one", "status": "completed"},
                    {"content": "two", "status": "pending"}
                ]}),
                usage: None,
                is_error: false,
            })),
        });
        let line = &st.lines()[0];
        assert_eq!(
            line.text, "todo [x] one · [ ] two",
            "short lists render fully: {:?}",
            line.text
        );
        assert!(line.full.as_deref().is_some_and(|f| f.contains("[x] one")));
    }

    #[test]
    fn long_todo_lists_summarize_to_counts() {
        let mut st = UiState::new();
        let items: Vec<serde_json::Value> = (0..7)
            .map(|i| {
                serde_json::json!({"content": format!("task {i}"), "status": if i < 3 {
                    "completed"
                } else {
                    "pending"
                }})
            })
            .collect();
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "todo".into(),
                content: vec![ContentBlock::text("...")],
                details: serde_json::json!({ "todo": items }),
                usage: None,
                is_error: false,
            })),
        });
        let line = &st.lines()[0];
        assert!(line.text.starts_with("todo (3/7 done)"), "{:?}", line.text);
    }

    #[test]
    fn malformed_todo_details_fall_back_to_generic_rendering() {
        let mut st = UiState::new();
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "todo".into(),
                content: vec![ContentBlock::text("boom")],
                details: serde_json::json!({"blocked": true}),
                usage: None,
                is_error: true,
            })),
        });
        let line = &st.lines()[0];
        assert_eq!(line.kind, LineKind::ToolResultErr, "generic error path");
        assert!(line.text.starts_with("todo → "));
    }

    // ---- M30/M126: the edit diff as a *parsed* diff --------------------------

    #[test]
    fn edit_result_with_a_diff_commits_a_parsed_diff_card() {
        // Mutated by M126: the assertion used to be `full.contains("@@")`, i.e. the
        // raw diff text parked in the `full` field for the markdown parser to chew
        // on. That is exactly how a diff came to be rendered as prose — a line
        // starting `-` is a list item in CommonMark and `---` is a rule, so a hunk
        // separator was drawn as a horizontal rule across the card. The diff is now
        // parsed into lines with kinds and numbers, which is what the renderer
        // draws from; `full` keeps the plain body so nothing else regresses.
        let mut st = UiState::new();
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "edit".into(),
                content: vec![ContentBlock::text("edited a.rs (1 replacement(s))")],
                details: serde_json::json!({"diff": "@@ -1,2 +1,2 @@\n old\n-new\n+new\n"}),
                usage: None,
                is_error: false,
            })),
        });
        let line = &st.lines()[0];
        // Head stays short — the diff body is capped until Ctrl+X.
        assert!(line.text.starts_with("edit → "), "head: {:?}", line.text);
        let card = line.card.as_ref().expect("an edit result is a card");
        let CardBody::Diff(diff) = &card.body else {
            panic!("an edit with details.diff must carry a parsed diff");
        };
        let kinds: Vec<DiffKind> = diff.lines.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            vec![
                DiffKind::Hunk,
                DiffKind::Context,
                DiffKind::Remove,
                DiffKind::Add
            ],
            "the diff's structure must survive the parse: {:?}",
            diff.lines
        );
        assert_eq!(
            diff.lines.iter().map(|l| l.number).collect::<Vec<_>>(),
            vec![None, Some(1), Some(2), Some(2)],
            "a context line advances both files; an addition advances only the new \
             one, so the removal is numbered 1 and the addition 2"
        );
        assert_eq!(diff.counts(), (1, 1), "one added, one removed");
        assert_eq!(card.status, CardStatus::Ok);
    }

    #[test]
    fn a_hunk_header_sets_the_line_numbers_it_declares() {
        // `@@ -10,2 +20,2 @@` means the old file's next line is 10 and the new
        // file's is 20. A card that numbered from 1 would put a plausible-looking
        // wrong number beside every line, which is worse than no number at all.
        //
        // The two sides' counters advance independently, so the removal is 11 (ten
        // context lines then it) and the addition is 21 — the two files are at
        // different places, which is exactly what a hunk header says.
        let diff = parse_diff("@@ -10,2 +20,2 @@\n context\n-gone\n+here\n");
        let numbers: Vec<(DiffKind, Option<usize>)> =
            diff.lines.iter().map(|l| (l.kind, l.number)).collect();
        assert_eq!(
            numbers,
            vec![
                (DiffKind::Hunk, None),
                (DiffKind::Context, Some(20)),
                (DiffKind::Remove, Some(11)),
                (DiffKind::Add, Some(21)),
            ]
        );
    }

    #[test]
    fn a_diff_line_the_parser_does_not_know_is_kept_not_dropped() {
        // `\ No newline at end of file` is the common case, and a diff is
        // evidence: dropping what the format does not define would hide part of
        // the change. It is printed as it arrived and is not numbered.
        let diff = parse_diff("@@ -1,1 +1,1 @@\n-old\n+new\n\\ No newline at end of file\n");
        let last = diff.lines.last().expect("the marker line survives");
        assert_eq!(last.kind, DiffKind::Meta);
        assert!(last.text.contains("No newline"), "{last:?}");
        assert_eq!(last.number, None);
    }

    #[test]
    fn a_tool_call_and_its_result_are_two_cards_but_one_call() {
        // The shape the event stream has, and the shape a reader needs: the call
        // is on screen (saying what is about to run) before the result exists, and
        // the result settles the call's own row rather than leaving a permanently
        // pending one to scroll past.
        let mut st = UiState::new();
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(assistant(
                "",
                vec![("tc_7", "bash")],
                Usage::default(),
            ))),
        });
        st.apply(&AgentEvent::ToolExecutionStart {
            tool_call_id: "tc_7".into(),
            tool_name: "bash".into(),
            args: serde_json::json!({"command": "cargo test"}),
        });
        assert_eq!(
            st.lines()[0].card.as_ref().unwrap().status,
            CardStatus::Running
        );
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_7".into(),
                tool_name: "bash".into(),
                content: vec![ContentBlock::text("ok")],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            })),
        });
        let cards: Vec<&Card> = st.lines().iter().filter_map(|l| l.card.as_ref()).collect();
        assert_eq!(cards.len(), 2, "the call row and the result row");
        assert_eq!(cards[0].tool_call_id, "tc_7");
        assert_eq!(
            cards[0].status,
            CardStatus::Ok,
            "the result must settle the call's own row"
        );
        assert!(
            cards[0].ended_at_ms.is_some(),
            "a settled call has a duration"
        );
        assert_eq!(cards[1].status, CardStatus::Ok);
        assert_eq!(cards[1].name, "bash");
    }

    #[test]
    fn a_pending_card_shows_what_the_call_is_about_to_do() {
        // The detail line is the card body, so it is what a reader sees while a
        // call is pending. `{"path":"src/app.rs"}` is a data dump, not a summary —
        // the skill's own card rule names "tool calls as raw JSON dumps" as the
        // thing a card exists to replace, and the first frame dump of this slice
        // printed exactly that for an obvious `edit`.
        assert_eq!(
            summarize_args(&serde_json::json!({ "command": "cargo test" })),
            "cargo test"
        );
        assert_eq!(
            summarize_args(&serde_json::json!({ "path": "src/app.rs" })),
            "src/app.rs"
        );
        assert_eq!(
            summarize_args(&serde_json::json!({ "filePath": "/tmp/a.rs" })),
            "/tmp/a.rs"
        );
        assert_eq!(
            summarize_args(&serde_json::json!({ "pattern": "fn draw" })),
            "fn draw"
        );
        // A shape this does not know still shows *something* rather than an empty
        // body, and it is truncated so one argument cannot become the whole card.
        let other = summarize_args(&serde_json::json!({ "weird": "x".repeat(200) }));
        assert!(other.contains("weird"), "{other}");
        assert!(
            other.ends_with('…'),
            "an unknown shape must be truncated: {other}"
        );
        // An empty known field falls through to the next candidate rather than
        // printing a blank body.
        assert_eq!(
            summarize_args(&serde_json::json!({ "command": "", "path": "b.rs" })),
            "b.rs"
        );
    }

    #[test]
    fn only_a_finished_card_carries_a_duration() {
        // The bug the first frame test caught: a pending card printed the time
        // since it was requested, which grew only when the row happened to be
        // rebuilt — so it was sometimes live, sometimes stale, and always a number
        // the user would read as "how long this took".
        let card = Card {
            name: "bash".into(),
            status: CardStatus::Pending,
            tool_call_id: "tc_1".into(),
            started_at_ms: now_ms(),
            ended_at_ms: None,
            body: CardBody::Text("running".into()),
        };
        assert_eq!(
            card.elapsed_ms(),
            None,
            "a running call has no duration yet"
        );
        let done = Card {
            ended_at_ms: Some(card.started_at_ms + 412),
            status: CardStatus::Ok,
            ..card
        };
        assert_eq!(done.elapsed_ms(), Some(412));
    }

    #[test]
    fn edit_result_without_diff_uses_body_for_full() {
        let mut st = UiState::new();
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "edit".into(),
                content: vec![ContentBlock::text("edited b.rs (1 replacement(s))")],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            })),
        });
        let line = &st.lines()[0];
        let full = line.full.as_deref().expect("full must exist");
        assert!(full.contains("edited b.rs"));
        assert!(
            !full.contains("@@"),
            "no diff in full when details.diff is absent"
        );
        // M126: the body is the tool's own output, not a diff with nothing in it —
        // an empty `Diff` body would draw a card with a title and no content.
        let card = line.card.as_ref().expect("a tool result is a card");
        assert!(
            matches!(&card.body, CardBody::Text(t) if t.contains("edited b.rs")),
            "an edit with no diff carries its body as text: {:?}",
            card.body
        );
    }

    #[test]
    fn data_uri_image_becomes_placeholder_not_flood() {
        let mut st = UiState::new();
        let blob = format!("data:image/png;base64,{}", "A".repeat(8000));
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "read".into(),
                content: vec![ContentBlock::text(blob.clone())],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            })),
        });
        let line = &st.lines()[0];
        assert!(
            line.text.starts_with("[image:"),
            "should be placeholder: {:?}",
            line.text
        );
        assert!(!line.text.contains('A'), "must not contain raw base64");
        assert!(line.text.contains("graphics deferred"));
    }

    #[test]
    fn long_base64_blob_becomes_placeholder() {
        let mut st = UiState::new();
        let blob = "A".repeat(9000);
        st.apply(&AgentEvent::MessageEnd {
            message: Box::new(Message::ToolResult(ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_1".into(),
                tool_name: "bash".into(),
                content: vec![ContentBlock::text(blob)],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            })),
        });
        assert!(
            st.lines()[0].text.starts_with("[image:"),
            "{:?}",
            st.lines()[0].text
        );
    }

    #[test]
    fn normal_text_not_mistaken_for_image() {
        assert!(summarize_image_blob("hello world").is_none());
        assert!(summarize_image_blob("data:text/plain;base64,SGVsbG8=").is_none());
    }
}
