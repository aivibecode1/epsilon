//! Context compaction (ADR-007): prune → clamp tail → strict-template summary →
//! amend prior summaries. Model view only; the human transcript is untouched.
//!
//! Triggers live in the loop; this module provides the pure transformation plus a token
//! estimator for pressure math.

use epsilon_contracts::content::{ContentBlock, UserContent};
use epsilon_contracts::message::Message;

/// Chars-per-token heuristic. Real BPE counting is an optional refinement; the estimate
/// only needs to be stable and roughly right for threshold math.
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// Estimated tokens across all LLM-visible message content.
pub fn estimate_messages(messages: &[Message]) -> usize {
    messages.iter().map(estimate_message).sum()
}

/// Provider-reported prompt size from the last completed turn, plus how many
/// messages that request covered (pi `compaction.ts:207-250`).
///
/// The char heuristic above is all a fresh process has, but a provider reports
/// the *true* size of every request it bills. Anchoring the estimate to the
/// last report and estimating only what came after it is the difference between
/// compacting at the right moment and compacting 3-4x early or late: prose
/// over-counts at 4 chars/token, code and CJK under-count by the same factor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageAnchor {
    /// Prompt tokens the provider billed for that request
    /// (`input + cache_read + cache_write`).
    pub prompt_tokens: usize,
    /// How many messages that request covered, counted from the front of the
    /// history it was built from.
    pub covered: usize,
}

/// Context size for the compaction decision: provider truth for the prefix, the
/// heuristic for whatever was appended since.
///
/// Falls back to the heuristic alone when there is no anchor, when the provider
/// reported nothing (`prompt_tokens == 0`), or when the transcript has *shrunk*
/// below the anchored length (compaction or a session switch happened, so the
/// anchor no longer describes this history).
pub fn anchored_tokens(history: &[Message], anchor: Option<UsageAnchor>) -> usize {
    match anchor {
        Some(a) if a.prompt_tokens > 0 && a.covered <= history.len() => {
            a.prompt_tokens + estimate_messages(&history[a.covered..])
        }
        _ => estimate_messages(history),
    }
}

/// Whether the retained tail may start at `index`: every message except a tool
/// result is a legal boundary.
///
/// A tail whose first message is a tool *result* is an orphan — its call lives in
/// the part that was just summarized — and several provider wires reject a
/// request shaped that way outright, so the whole turn would fail after paying
/// for the summary.
fn valid_cut(messages: &[Message], index: usize) -> bool {
    !matches!(messages.get(index), None | Some(Message::ToolResult(_)))
}

/// Move a tentative tail start to the nearest legal boundary (pi
/// `compaction.ts:312-422`, DSH `tool-pairing.ts:110`).
///
/// Forward first: dropping a few extra messages costs less than an orphaned
/// tail. If the whole remainder is one tool batch, walk *back* instead — a tail
/// slightly larger than the budget is still a valid request, while a tail that
/// starts mid-batch is not.
pub fn legal_cut_point(messages: &[Message], tentative: usize) -> usize {
    if tentative >= messages.len() {
        return tentative;
    }
    if valid_cut(messages, tentative) {
        return tentative;
    }
    if let Some(forward) = (tentative + 1..messages.len()).find(|&i| valid_cut(messages, i)) {
        return forward;
    }
    (0..tentative)
        .rev()
        .find(|&i| valid_cut(messages, i))
        .unwrap_or(tentative)
}

fn estimate_message(m: &Message) -> usize {
    match m {
        Message::User(u) => u.content.iter().map(user_len).sum(),
        Message::Assistant(a) => a.content.iter().map(block_len).sum::<usize>() + 8,
        Message::ToolResult(t) => t.content.iter().map(block_len).sum::<usize>() + 4,
        Message::Custom(_) => 0,
    }
}

fn user_len(c: &UserContent) -> usize {
    match c {
        UserContent::Text { text } => estimate_tokens(text),
        UserContent::Image(_) => 512, // vision tokens vary wildly; conservative flat cost
    }
}

fn block_len(b: &ContentBlock) -> usize {
    match b {
        ContentBlock::Text { text } | ContentBlock::Thinking { text, .. } => estimate_tokens(text),
        ContentBlock::Image(_) => 512,
        ContentBlock::ToolCall { arguments, .. } => estimate_tokens(&arguments.to_string()) + 8,
    }
}

/// Protected-tail clamp bounds from opencode (ADR-007).
pub const MIN_TAIL_TOKENS: usize = 2_000;
pub const MAX_TAIL_TOKENS: usize = 15_000;

/// Build the summarization request context: strict template + serialized transcript.
///
/// `prior_summary` triggers amend-mode (Hermes rule): we ask for a DELTA update rather than
/// re-summarizing everything, which keeps summaries stable across successive compactions.
///
/// `ledger` rides outside the template: the model is told not to reproduce it because
/// it is harness-maintained fact, not something to remember (`FileLedger`).
pub fn summary_context(
    transcript: &[Message],
    prior_summary: Option<&str>,
    ledger: &FileLedger,
) -> epsilon_llm::LlmContext {
    let mut body = String::with_capacity(16 * 1024);
    if let Some(prev) = prior_summary {
        body.push_str("<prior_summary>\n");
        body.push_str(prev);
        body.push_str(
            "\n</prior_summary>\n\nUpdate this summary with anything important from \
                       the NEW conversation below. Keep every section.\n\n",
        );
    }
    let block = ledger.render();
    if !block.is_empty() {
        body.push_str(
            "The harness maintains this exact record of files touched so far; do \
                       NOT copy it into your summary:\n",
        );
        body.push_str(&block);
        body.push_str("\n\n");
    }
    body.push_str(
        "Do not paraphrase the user's instructions: the harness keeps their exact words \
         separately, so quote rather than restate.\n\n",
    );
    body.push_str("<conversation>\n");
    for m in transcript {
        match m {
            Message::User(u) => {
                let t = u
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        UserContent::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                body.push_str(&format!("[user] {t}\n"));
            }
            Message::Assistant(a) => {
                if !a.text().is_empty() {
                    body.push_str(&format!("[assistant] {}\n", a.text()));
                }
                for (_, name, args) in a.tool_calls() {
                    body.push_str(&format!("[assistant tool call] {name} {args}\n"));
                }
            }
            Message::ToolResult(t) => {
                let txt = t
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<String>();
                // Tool outputs get a hard cap here — they dominate transcripts.
                // Char-boundary floor walk: byte 2000 can land mid-character on CJK
                // or emoji output and `&txt[..CAP]` would panic (plan 002).
                const CAP: usize = 2_000;
                let clipped = if txt.len() > CAP {
                    let cut = txt
                        .char_indices()
                        .map(|(i, _)| i)
                        .take_while(|i| *i <= CAP)
                        .last()
                        .unwrap_or(CAP);
                    format!("{}…[clipped]", &txt[..cut])
                } else {
                    txt
                };
                body.push_str(&format!(
                    "[tool {} result{}] {clipped}\n",
                    t.tool_name,
                    if t.is_error { " ERROR" } else { "" }
                ));
            }
            Message::Custom(_) => {}
        }
    }
    body.push_str("</conversation>\n");

    epsilon_llm::LlmContext {
        // Summarization calls never need reasoning (ADR-007: isolated, cheap).
        thinking_level: epsilon_contracts::ThinkingLevel::Off,
        system_prompt: summary_system_prompt(),
        messages: vec![Message::User(epsilon_contracts::message::UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            content: vec![UserContent::Text { text: body }],
        })],
        tools: vec![],
    }
}

/// The six sections every summary is required to have.
///
/// The order is the order a reader reloads context in: what was being done, then
/// the rules they set, then where it got to, then why, then what is next, then
/// whatever else would be lost. `Critical Context` is last on purpose — it is the
/// catch-all, and putting it earlier invites the model to dump everything there
/// and leave the specific sections thin.
pub const SUMMARY_SECTIONS: [(&str, &str); 6] = [
    ("Goal", "what the user is ultimately trying to achieve"),
    (
        "Constraints & Preferences",
        "exact instructions, style rules, and prohibitions",
    ),
    (
        "Progress",
        "what has been done, what is verified, what is in flight",
    ),
    ("Key Decisions", "choices made and the reason for each"),
    (
        "Next Steps",
        "the immediate next actions, specific enough to resume without guessing",
    ),
    (
        "Critical Context",
        "facts, paths, errors or state that would otherwise be lost",
    ),
];

/// The system prompt for a summarization call.
///
/// Built from [`SUMMARY_SECTIONS`] so the template the model is asked to fill in
/// is the template it is actually sent. The earlier version asserted "Output ONLY
/// the template" while sending no template at all, which produced a different
/// summary shape on every compaction — and a summary whose shape changes is one a
/// reader has to re-learn each time.
pub fn summary_system_prompt() -> String {
    let mut s = String::from(
        "You compress coding-agent transcripts into a structured summary. \
         Output ONLY the sections below, in this order, using these exact headings.\n\n",
    );
    for (index, (name, what)) in SUMMARY_SECTIONS.iter().enumerate() {
        s.push_str(&format!("{}. ## {name} — {what}\n", index + 1));
    }
    s.push_str(
        "\nRules:\n\
         - Quote the user's instructions rather than paraphrasing them.\n\
         - Prefer specific facts (paths, commands, error text) over narrative.\n\
         - Say what is UNVERIFIED as clearly as what is done.\n\
         - Never invent progress that the transcript does not show.\n\
         - Omit a section only if the transcript truly says nothing about it.",
    );
    s
}

/// Extract the assistant's summary text from a completed provider turn.
///
/// `None` when the turn produced nothing usable, and there are two ways that
/// happens:
///
/// * **The answer is empty.** A compaction must never replace useful context with
///   a placeholder, so the caller skips the commit and reports the failure rather
///   than substituting one.
/// * **The answer was cut off by the output limit** (`StopReason::MaxTokens`).
///   A half-written summary is the more dangerous of the two, because it is not
///   obviously broken: it has a Goal and a Progress section, so it reads as a
///   complete record of a session that simply had less in it, and the transcript
///   it replaced is gone. pi refuses a `length`-stopped summary for the same
///   reason; the caller falls back to context-free pruning, which loses detail
///   but never fabricates a record.
///
/// The empty-answer check is Epsilon's own rule rather than a claimed parity
/// behaviour — pi's inspected helper returns the extracted text without one.
pub fn summary_text(answer: &epsilon_contracts::message::AssistantMessage) -> Option<String> {
    if answer.stop_reason == epsilon_contracts::message::StopReason::MaxTokens {
        return None;
    }
    let t = answer.text();
    if t.trim().is_empty() { None } else { Some(t) }
}

/// The model-free pass: drop old tool RESULTS beyond the protected tail window.
///
/// Tool outputs are the bulk of long transcripts and are reconstructible on demand;
/// protecting only the recent window keeps the tail informative while shrinking fast.
/// Returns the slimmed history plus how many results were pruned.
pub fn prune_old_tool_results(
    messages: &[Message],
    tail_protect_tokens: usize,
) -> (Vec<Message>, usize) {
    // Walk from the end accumulating protected budget.
    let mut protected = 0usize;
    let mut cut_index = 0usize; // first index considered prunable
    for (i, m) in messages.iter().enumerate().rev() {
        protected += estimate_message(m);
        if protected >= tail_protect_tokens {
            cut_index = i;
            break;
        }
        cut_index = i;
    }

    let mut out = Vec::with_capacity(messages.len());
    let mut pruned = 0usize;
    for (i, m) in messages.iter().enumerate() {
        match m {
            Message::ToolResult(t) if i < cut_index && !t.is_error => {
                // Replace with a tombstone user-role note so provider shape stays valid.
                out.push(Message::ToolResult(
                    epsilon_contracts::message::ToolResultMessage {
                        id: t.id.clone(),
                        timestamp_ms: t.timestamp_ms,
                        tool_call_id: t.tool_call_id.clone(),
                        tool_name: t.tool_name.clone(),
                        content: vec![ContentBlock::text("[old tool output cleared]")],
                        details: serde_json::json!({"pruned": true}),
                        usage: None,
                        is_error: false,
                    },
                ));
                pruned += 1;
            }
            other => out.push(other.clone()),
        }
    }
    (out, pruned)
}

/// Files the session has read and changed, in first-touch order (pi
/// `compaction/utils.ts:24-72`).
///
/// A summary is prose and prose is lossy. After a compaction the model reliably
/// forgets which files it has already opened and which it has already changed,
/// so it re-reads them — the most expensive way to learn something the harness
/// knew exactly. This ledger is extracted from the tool calls themselves, so it
/// is a fact rather than a recollection, and it rides along with every summary.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FileLedger {
    /// Files opened with `read`.
    pub read: Vec<String>,
    /// Files changed with `write` / `edit`.
    pub modified: Vec<String>,
}

/// Per-list cap. Long sessions touch hundreds of files; the block has to stay
/// cheap enough to sit in the context of every subsequent turn.
const LEDGER_CAP: usize = 24;

impl FileLedger {
    /// Collect the paths touched by the tool calls in `messages`.
    pub fn scan(messages: &[Message]) -> Self {
        let mut ledger = Self::default();
        for m in messages {
            let Message::Assistant(a) = m else { continue };
            for block in &a.content {
                let ContentBlock::ToolCall {
                    name, arguments, ..
                } = block
                else {
                    continue;
                };
                let Some(path) = arguments.get("path").and_then(|p| p.as_str()) else {
                    continue;
                };
                match name.as_str() {
                    "read" => push_once(&mut ledger.read, path),
                    "write" | "edit" => push_once(&mut ledger.modified, path),
                    // `grep` / `ls` / `glob` paths are directories as often as files,
                    // so they would put noise in a list whose value is its precision.
                    _ => {}
                }
            }
        }
        ledger
    }

    /// Recover the ledger a previous compaction embedded in its summary, so
    /// entries survive successive compactions instead of restarting at each one.
    pub fn from_summary(summary: &str) -> Self {
        let mut ledger = Self::default();
        let Some(block) = summary
            .split_once("<file_ledger>")
            .and_then(|(_, rest)| rest.split_once("</file_ledger>"))
            .map(|(body, _)| body)
        else {
            return ledger;
        };
        for line in block.lines() {
            let line = line.trim();
            let (target, rest) = if let Some(rest) = line.strip_prefix("read: ") {
                (&mut ledger.read, rest)
            } else if let Some(rest) = line.strip_prefix("modified: ") {
                (&mut ledger.modified, rest)
            } else {
                continue;
            };
            for path in rest.split(", ").map(str::trim).filter(|p| !p.is_empty()) {
                push_once(target, path);
            }
        }
        ledger
    }

    /// Fold `other` in, keeping first-touch order and dropping duplicates.
    pub fn merge(&mut self, other: &Self) {
        for path in &other.read {
            push_once(&mut self.read, path);
        }
        for path in &other.modified {
            push_once(&mut self.modified, path);
        }
    }

    /// The block as it appears in a summary; empty when nothing was touched.
    pub fn render(&self) -> String {
        if self.read.is_empty() && self.modified.is_empty() {
            return String::new();
        }
        let mut out = String::from("<file_ledger>\n");
        for (label, list) in [("read", &self.read), ("modified", &self.modified)] {
            if list.is_empty() {
                continue;
            }
            let shown = list.len().min(LEDGER_CAP);
            out.push_str(label);
            out.push_str(": ");
            out.push_str(&list[..shown].join(", "));
            if list.len() > shown {
                out.push_str(&format!(" (+{} more)", list.len() - shown));
            }
            out.push('\n');
        }
        out.push_str("</file_ledger>");
        out
    }
}

/// Append the ledger block to a summary before it becomes context.
pub fn summary_with_ledger(summary: &str, ledger: &FileLedger) -> String {
    let block = ledger.render();
    if block.is_empty() {
        summary.to_string()
    } else {
        format!("{summary}\n\n{block}")
    }
}

/// The instruction lines a previous summary carried forward, oldest first.
///
/// Without this, the block only survives one compaction: the next pass would summarize the
/// summary and take the user's words with it. Lines the harness itself added (the omission note)
/// are skipped.
fn carried_instructions(summary_text: &str) -> Vec<String> {
    let Some((_, rest)) = summary_text.split_once("<user_instructions>") else {
        return Vec::new();
    };
    let Some((body, _)) = rest.split_once("</user_instructions>") else {
        return Vec::new();
    };
    body.lines()
        .map(str::trim)
        // The omission note is the harness talking, not the user.
        .filter(|l| !l.is_empty() && !l.starts_with('['))
        .map(str::to_string)
        .collect()
}

/// Cap on the verbatim user-instruction block. Instructions are short by nature; this bounds
/// what a long session with many turns can add to every subsequent request.
const MAX_INSTRUCTION_CHARS: usize = 4_000;

/// The user's own words from the summarized range, verbatim (Hermes `docs/micro-compaction.md:63`).
///
/// A summary is the model's paraphrase, and a paraphrased instruction is one that gets followed
/// approximately — "don't touch the parser" becomes "the parser was discussed". These are the
/// turns as the user typed them, kept outside the prose where nothing can soften them.
///
/// Collected newest-first so the cap drops the oldest instructions rather than the most recent
/// ones, then rendered oldest-first for reading order. Harness-injected messages (a previous
/// compaction summary) are not the user's words and are skipped.
pub fn user_instruction_block(messages: &[Message]) -> String {
    // Owned: a turn's text can be the join of several blocks, which does not live long enough to
    // borrow. Instructions are small, and the block is capped.
    let mut kept: Vec<String> = Vec::new();
    let mut used = 0usize;
    let mut omitted = 0usize;
    let push = |text: &str, kept: &mut Vec<String>, used: &mut usize, omitted: &mut usize| {
        let text = text.trim();
        if text.is_empty() || kept.iter().any(|k| k == text) {
            return;
        }
        if *used + text.len() > MAX_INSTRUCTION_CHARS {
            *omitted += 1;
            return;
        }
        *used += text.len();
        kept.push(text.to_string());
    };
    for m in messages.iter().rev() {
        let Message::User(u) = m else { continue };
        let text = u
            .content
            .iter()
            .filter_map(|c| match c {
                UserContent::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if text.starts_with("<compaction_summary>") {
            // A previous compaction's summary: whatever *it* carried forward is older than this
            // range, and would otherwise be lost the second time around. Recover it from the
            // block the harness wrote (same idea as `FileLedger::from_summary`).
            for line in carried_instructions(text) {
                push(&line, &mut kept, &mut used, &mut omitted);
            }
            continue;
        }
        push(text, &mut kept, &mut used, &mut omitted);
    }
    if kept.is_empty() {
        return String::new();
    }
    kept.reverse();
    let mut out = String::with_capacity(used + 64);
    out.push_str("<user_instructions>\n");
    if omitted > 0 {
        out.push_str(&format!(
            "[{omitted} earlier turn(s) omitted for length — read the session file if they matter]\n"
        ));
    }
    for text in kept {
        out.push_str(&text);
        out.push('\n');
    }
    out.push_str("</user_instructions>");
    out
}

/// Append the verbatim user-instruction block to a summary before it becomes context.
pub fn summary_with_instructions(summary: &str, instructions: &str) -> String {
    if instructions.is_empty() {
        summary.to_string()
    } else {
        format!("{summary}\n\n{instructions}")
    }
}

fn push_once(list: &mut Vec<String>, path: &str) {
    if !list.iter().any(|p| p == path) {
        list.push(path.to_string());
    }
}

#[cfg(test)]
mod anchor_and_cut_tests {
    use super::*;
    use epsilon_contracts::content::{ContentBlock, UserContent};
    use epsilon_contracts::message::AssistantMessage;
    use epsilon_contracts::message::{Message, ToolResultMessage, UserMessage};

    fn user(text: &str) -> Message {
        Message::User(UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            content: vec![UserContent::Text {
                text: text.to_string(),
            }],
        })
    }

    fn tool_result(text: &str) -> Message {
        Message::ToolResult(ToolResultMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            tool_call_id: epsilon_contracts::new_id("call"),
            tool_name: "bash".into(),
            content: vec![ContentBlock::text(text)],
            details: serde_json::json!({}),
            usage: None,
            is_error: false,
        })
    }

    /// A short name for a message's shape, for assertion messages.
    fn shape(m: Option<&Message>) -> &'static str {
        match m {
            Some(Message::User(_)) => "user",
            Some(Message::Assistant(_)) => "assistant",
            Some(Message::ToolResult(_)) => "tool-result",
            Some(Message::Custom(_)) => "custom",
            None => "end-of-history",
        }
    }

    /// A summary turn that stopped for a named reason.
    fn answer_with(stop_reason: epsilon_contracts::message::StopReason) -> AssistantMessage {
        AssistantMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            provider: "test".into(),
            model_id: "test".into(),
            content: vec![ContentBlock::text("## Goal\nlooks complete")],
            stop_reason,
            error_message: None,
            usage: Default::default(),
        }
    }

    #[test]
    fn a_summary_cut_off_by_the_output_limit_is_refused() {
        // The dangerous case is not the empty answer — that one is obviously
        // broken. A truncated summary has a Goal and a Progress section, so it
        // reads as a complete record of a shorter session, and committing it
        // replaces the real transcript with it.
        let truncated = answer_with(epsilon_contracts::message::StopReason::MaxTokens);
        assert!(
            summary_text(&truncated).is_none(),
            "a length-stopped summary must not be committed"
        );
    }

    #[test]
    fn a_complete_summary_is_still_accepted() {
        // The refusal must not spread: an ordinary end-of-turn summary is exactly
        // what this function exists to return.
        let complete = answer_with(epsilon_contracts::message::StopReason::EndTurn);
        assert_eq!(
            summary_text(&complete).as_deref(),
            Some("## Goal\nlooks complete")
        );
    }

    #[test]
    fn an_empty_summary_is_still_refused() {
        let mut empty = answer_with(epsilon_contracts::message::StopReason::EndTurn);
        empty.content = vec![ContentBlock::text("   ")];
        assert!(summary_text(&empty).is_none());
    }

    fn assistant(text: &str) -> Message {
        Message::Assistant(epsilon_contracts::message::AssistantMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            provider: "test".into(),
            model_id: "test".into(),
            content: vec![ContentBlock::text(text)],
            stop_reason: epsilon_contracts::message::StopReason::EndTurn,
            error_message: None,
            usage: Default::default(),
        })
    }

    #[test]
    fn anchor_replaces_the_heuristic_for_the_prefix_it_covers() {
        // 4 KB of prose: the char heuristic calls that 1000 tokens, the provider
        // says 250. The anchored estimate must use the provider's number and add
        // only what arrived after the request.
        let history = vec![user(&"a".repeat(4000)), user("tail message")];
        let heuristic = estimate_messages(&history);
        assert!(heuristic > 1000, "fixture assumption: {heuristic}");

        let anchored = anchored_tokens(
            &history,
            Some(UsageAnchor {
                prompt_tokens: 250,
                covered: 1,
            }),
        );
        assert!(anchored < heuristic, "the anchor must correct the estimate");
        assert_eq!(
            anchored,
            250 + estimate_messages(&history[1..]),
            "provider truth for the prefix, heuristic for the tail"
        );

        // Nothing appended since the request: the provider's number stands alone.
        assert_eq!(
            anchored_tokens(
                &history,
                Some(UsageAnchor {
                    prompt_tokens: 250,
                    covered: 2
                })
            ),
            250
        );
    }

    #[test]
    fn anchor_is_ignored_when_it_cannot_describe_this_history() {
        let history = vec![user("short")];
        let heuristic = estimate_messages(&history);

        // No anchor, a provider that reports nothing, or a transcript that shrank
        // under the anchored length (compaction / session switch) all fall back to
        // the heuristic rather than trusting a stale number.
        assert_eq!(anchored_tokens(&history, None), heuristic);
        assert_eq!(
            anchored_tokens(
                &history,
                Some(UsageAnchor {
                    prompt_tokens: 0,
                    covered: 0
                })
            ),
            heuristic
        );
        assert_eq!(
            anchored_tokens(
                &history,
                Some(UsageAnchor {
                    prompt_tokens: 9_000,
                    covered: 7
                })
            ),
            heuristic
        );
    }

    #[test]
    fn a_tail_never_starts_with_an_orphaned_tool_result() {
        // [user, assistant, result, result, user, assistant]
        //                 ^ a tentative cut here would orphan both results.
        let history = vec![
            user("do it"),
            assistant("calling"),
            tool_result("one"),
            tool_result("two"),
            user("next"),
            assistant("done"),
        ];
        assert_eq!(legal_cut_point(&history, 0), 0, "a user turn is legal");
        assert_eq!(
            legal_cut_point(&history, 1),
            1,
            "an assistant turn is legal"
        );
        assert_eq!(
            legal_cut_point(&history, 2),
            4,
            "a cut inside the batch moves forward to the next turn"
        );
        assert_eq!(legal_cut_point(&history, 3), 4);
        assert_eq!(legal_cut_point(&history, 5), 5);
        assert_eq!(
            legal_cut_point(&history, history.len()),
            history.len(),
            "no tail at all stays no tail"
        );
    }

    #[test]
    fn a_batch_running_to_the_end_grows_the_tail_instead_of_orphaning_it() {
        // [user, assistant, result, result] — every boundary from index 2 on is a
        // tool result, so the cut walks *back* to the assistant turn rather than
        // producing a tail that starts with an orphan.
        let history = vec![
            user("do it"),
            assistant("calling"),
            tool_result("one"),
            tool_result("two"),
        ];
        assert_eq!(legal_cut_point(&history, 2), 1);
        assert_eq!(legal_cut_point(&history, 3), 1);
    }

    #[test]
    fn a_history_of_only_tool_results_is_left_alone() {
        // Cannot happen from the loop (a batch always follows an assistant turn),
        // but a hand-edited or repaired log could look like this; the cut must not
        // loop forever or panic.
        let history = vec![tool_result("a"), tool_result("b")];
        assert_eq!(legal_cut_point(&history, 1), 1);
    }

    /// The roadmap's acceptance criterion for legal cuts: *every* retained tail starts at a
    /// turn boundary, over generated transcripts rather than the three hand-written ones.
    ///
    /// A 32-bit LCG keeps this deterministic and dependency-free; the sequence is arbitrary
    /// but identical on every run, so a failure is reproducible.
    #[test]
    fn no_generated_transcript_can_produce_an_illegal_tail() {
        let mut seed: u32 = 0x5EED_1234;
        let mut next = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as usize
        };

        for case in 0..400 {
            // Build a plausible transcript: user turns, assistant turns, and batches of 1-3
            // tool results after some of them.
            let mut history: Vec<Message> = Vec::new();
            let turns = 1 + next() % 12;
            for t in 0..turns {
                if next() % 4 != 0 {
                    history.push(user(&format!("turn {t}")));
                }
                history.push(assistant("ok"));
                let batch = next() % 4;
                for r in 0..batch {
                    history.push(tool_result(&format!("result {r}")));
                }
            }
            if history.is_empty() {
                continue;
            }

            for tentative in 0..=history.len() {
                let cut = legal_cut_point(&history, tentative);
                assert!(
                    cut <= history.len(),
                    "case {case}: cut {cut} is out of bounds for {}",
                    history.len()
                );
                assert!(
                    !matches!(history.get(cut), Some(Message::ToolResult(_))),
                    "case {case}: tentative {tentative} produced a tail starting with an \
                     orphaned tool result: {:?}",
                    shape(history.get(cut))
                );
                if cut != tentative {
                    assert!(
                        matches!(
                            history.get(cut),
                            Some(Message::User(_) | Message::Assistant(_))
                        ),
                        "case {case}: an adjusted cut must land on a real turn, not {:?}",
                        shape(history.get(cut))
                    );
                }
            }
        }
    }

    #[test]
    fn an_adjusted_cut_never_loses_a_tool_result_its_call() {
        // The invariant that matters to a provider: everything in the tail must be
        // self-contained. Scan generated transcripts and check that no tail begins with a
        // result whose call was summarized away.
        let mut seed: u32 = 0xA11C_E001;
        let mut next = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 5) as usize
        };
        for _ in 0..200 {
            let mut history = Vec::new();
            for t in 0..(1 + next() % 8) {
                history.push(user(&format!("u{t}")));
                history.push(assistant("a"));
                for r in 0..(next() % 3) {
                    history.push(tool_result(&format!("r{r}")));
                }
            }
            let cut = legal_cut_point(&history, next() % (history.len() + 1));
            let tail = &history[cut..];
            assert!(
                !matches!(tail.first(), Some(Message::ToolResult(_))),
                "tail opens with an orphan: {cut}"
            );
        }
    }
}

#[cfg(test)]
mod plan002_tests {
    use super::*;
    use epsilon_contracts::content::{ContentBlock, UserContent};
    use epsilon_contracts::message::{Message, ToolResultMessage, UserMessage};

    fn tool_result_with_output(text: &str) -> Message {
        Message::ToolResult(ToolResultMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            tool_call_id: epsilon_contracts::new_id("call"),
            tool_name: "bash".into(),
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            details: serde_json::json!({}),
            usage: None,
            is_error: false,
        })
    }

    #[test]
    fn clipping_multi_byte_tool_output_does_not_panic() {
        // CJK chars are 3 bytes each; byte 2000 lands mid-character. The tombstone
        // path (prune) and the summary clip path must both be panic-free.
        let cjk: String = "好".repeat(1000); // 3000 bytes
        let msgs = vec![tool_result_with_output(&cjk)];
        // prune with tail protection 0: cut_index stays 0 → no pruning, passthrough.
        let (out, pruned) = prune_old_tool_results(&msgs, 0);
        assert_eq!(pruned, 0);
        assert!(matches!(&out[0], Message::ToolResult(_)));
        // The panic used to happen inside summary_context's clip path:
        let ctx = summary_context(&msgs, None, &FileLedger::default());
        assert!(!ctx.system_prompt.is_empty() || !ctx.messages.is_empty());
    }

    #[test]
    fn summary_context_clips_multi_byte_without_panicking() {
        let cjk: String = "🌍".repeat(1500); // ~6000 bytes, crosses the 2000-byte cap
        let msgs = vec![tool_result_with_output(&cjk)];
        // Must not panic; body contains a clipped marker.
        let ctx = summary_context(&msgs, None, &FileLedger::default());
        assert!(!ctx.system_prompt.is_empty() || !ctx.messages.is_empty());
    }

    #[test]
    fn summary_context_amend_mode_includes_prior_summary_marker() {
        let user = Message::User(UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            content: vec![UserContent::Text {
                text: "progress note".into(),
            }],
        });
        let ctx = summary_context(&[user], Some("PREV"), &FileLedger::default());
        let rendered = format!("{:?}", ctx.messages);
        assert!(
            rendered.contains("<prior_summary>") && rendered.contains("PREV"),
            "amend prompt must embed the prior summary"
        );
    }
}

#[cfg(test)]
mod instruction_block_tests {
    use super::*;
    use epsilon_contracts::content::UserContent;
    use epsilon_contracts::message::{Message, UserMessage};

    fn user(text: &str) -> Message {
        Message::User(UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            content: vec![UserContent::Text {
                text: text.to_string(),
            }],
        })
    }

    #[test]
    fn the_users_own_words_survive_compaction_verbatim() {
        let history = vec![
            user("Do NOT touch the parser, only the formatter."),
            user("now fix the typo in line 3"),
            user("and keep the public API stable"),
        ];
        let block = user_instruction_block(&history);
        assert!(block.starts_with("<user_instructions>\n"));
        assert!(block.ends_with("</user_instructions>"));
        assert!(
            block.contains("Do NOT touch the parser, only the formatter."),
            "exact words, not a summary of them: {block}"
        );
        // Reading order: oldest first, with the newest last.
        let first = block.find("Do NOT touch").unwrap();
        let last = block.find("public API stable").unwrap();
        assert!(first < last, "instructions read in order: {block}");
    }

    #[test]
    fn a_previous_summary_is_not_mistaken_for_something_the_user_said() {
        let history = vec![
            user("<compaction_summary>\n## Objective\n(old prose)\n</compaction_summary>"),
            user("real instruction"),
        ];
        let block = user_instruction_block(&history);
        assert!(block.contains("real instruction"));
        assert!(
            !block.contains("(old prose)"),
            "harness prose is not the user's: {block}"
        );
    }

    #[test]
    fn the_cap_drops_the_oldest_turns_and_says_how_many() {
        let mut history = Vec::new();
        for i in 0..12 {
            history.push(user(&format!("instruction {i}: {}", "d".repeat(600))));
        }
        let block = user_instruction_block(&history);
        assert!(
            block.contains("instruction 11"),
            "the newest always survives"
        );
        assert!(block.contains("omitted for length"), "{block}");
        assert!(
            !block.contains("instruction 0:"),
            "the oldest is what gets dropped: {block}"
        );
        assert!(
            block.len() < MAX_INSTRUCTION_CHARS + 200,
            "bounded: {}",
            block.len()
        );
    }

    #[test]
    fn nothing_to_keep_means_nothing_is_added() {
        assert_eq!(user_instruction_block(&[]), "");
        let assistant = Message::Assistant(epsilon_contracts::message::AssistantMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            provider: "t".into(),
            model_id: "t".into(),
            content: vec![epsilon_contracts::content::ContentBlock::text("hello")],
            stop_reason: epsilon_contracts::message::StopReason::EndTurn,
            error_message: None,
            usage: Default::default(),
        });
        assert_eq!(user_instruction_block(&[assistant]), "");
        assert_eq!(summary_with_instructions("prose", ""), "prose");
        assert_eq!(
            summary_with_instructions("prose", "<user_instructions>\nx\n</user_instructions>"),
            "prose\n\n<user_instructions>\nx\n</user_instructions>"
        );
    }
}

#[cfg(test)]
mod ledger_tests {
    use super::*;
    use epsilon_contracts::content::{ContentBlock, UserContent};
    use epsilon_contracts::message::{AssistantMessage, Message};

    fn call(name: &str, args: serde_json::Value) -> Message {
        Message::Assistant(AssistantMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            provider: "test".into(),
            model_id: "test".into(),
            content: vec![ContentBlock::ToolCall {
                id: epsilon_contracts::new_id("call"),
                name: name.into(),
                arguments: args,
            }],
            stop_reason: epsilon_contracts::message::StopReason::ToolUse,
            error_message: None,
            usage: Default::default(),
        })
    }

    fn user(text: &str) -> Message {
        Message::User(epsilon_contracts::message::UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            content: vec![UserContent::Text {
                text: text.to_string(),
            }],
        })
    }

    #[test]
    fn the_ledger_comes_from_the_tool_calls_themselves() {
        let msgs = vec![
            user("start"),
            call("read", serde_json::json!({"path": "src/a.rs"})),
            call("read", serde_json::json!({"path": "src/a.rs"})), // opened twice
            call("edit", serde_json::json!({"path": "src/b.rs"})),
            call("write", serde_json::json!({"path": "src/c.rs"})),
            call("grep", serde_json::json!({"path": "src"})), // a directory, not a file
            call("read", serde_json::json!({"limit": 10})),   // no path at all
        ];
        let ledger = FileLedger::scan(&msgs);
        assert_eq!(ledger.read, vec!["src/a.rs"], "first touch wins, no dupes");
        assert_eq!(ledger.modified, vec!["src/b.rs", "src/c.rs"]);
        assert!(
            !ledger.read.iter().any(|p| p == "src"),
            "directory paths would make the list useless"
        );
    }

    #[test]
    fn a_ledger_rides_into_the_next_summary_and_comes_back_out() {
        let ledger = FileLedger {
            read: vec!["a.rs".into()],
            modified: vec!["b.rs".into()],
        };
        let summary = summary_with_ledger("## Objective\nship it", &ledger);
        assert!(
            summary.starts_with("## Objective"),
            "model text stays first"
        );
        assert!(summary.contains("<file_ledger>"));
        assert_eq!(
            FileLedger::from_summary(&summary),
            ledger,
            "the next compaction must recover exactly what was recorded"
        );

        // A summary without a block recovers nothing rather than inventing entries.
        assert_eq!(
            FileLedger::from_summary("## Objective\nship it"),
            FileLedger::default()
        );
        // An empty ledger adds nothing to the message at all.
        assert_eq!(
            summary_with_ledger("text", &FileLedger::default()),
            "text",
            "a session that touched no files pays no tokens for the block"
        );
    }

    #[test]
    fn successive_compactions_accumulate_instead_of_restarting() {
        let older = FileLedger {
            read: vec!["old.rs".into()],
            modified: vec!["shared.rs".into()],
        };
        let mut carried = FileLedger::from_summary(&summary_with_ledger("s1", &older));
        carried.merge(&FileLedger {
            read: vec!["shared.rs".into(), "new.rs".into()],
            modified: vec![],
        });
        assert_eq!(
            carried.read,
            vec!["old.rs", "shared.rs", "new.rs"],
            "earlier entries keep their place, new ones append once"
        );
        assert_eq!(carried.modified, vec!["shared.rs"]);
    }

    #[test]
    fn a_long_ledger_is_capped_and_says_how_much_it_hid() {
        let read: Vec<String> = (0..30).map(|i| format!("f{i}.rs")).collect();
        let out = FileLedger {
            read,
            modified: Vec::new(),
        }
        .render();
        assert!(out.contains("(+6 more)"), "cap must be visible: {out}");
        assert!(out.contains("f23.rs"), "the 24th entry still fits");
        assert!(!out.contains("f24.rs"));
        assert!(out.ends_with("</file_ledger>"));
    }

    #[test]
    fn the_summary_request_shows_the_ledger_but_forbids_copying_it() {
        let msgs = vec![call("read", serde_json::json!({"path": "x.rs"}))];
        let mut ledger = FileLedger::default();
        ledger.merge(&FileLedger::scan(&msgs));
        let ctx = summary_context(&msgs, None, &ledger);
        let rendered = format!("{:?}", ctx.messages);
        assert!(
            rendered.contains("x.rs"),
            "the summarizer sees what was read"
        );
        assert!(
            rendered.contains("do") && rendered.contains("NOT copy it"),
            "and is told the block is harness-maintained, not summary material"
        );
    }

    #[test]
    fn the_summarizer_is_sent_the_template_it_is_told_to_follow() {
        // The bug this pins: the system prompt said "output ONLY the template"
        // while the request carried no template, so every compaction produced a
        // differently shaped summary.
        let ctx = summary_context(&[], None, &FileLedger::default());
        for (name, _) in SUMMARY_SECTIONS {
            assert!(
                ctx.system_prompt.contains(name),
                "the template must name the {name} section: {}",
                ctx.system_prompt
            );
        }
        assert!(
            ctx.system_prompt.contains("Output ONLY"),
            "the instruction and the template have to travel together"
        );
        // Order is part of the contract: a reader reloads context in this order.
        let positions: Vec<usize> = SUMMARY_SECTIONS
            .iter()
            .map(|(name, _)| ctx.system_prompt.find(name).expect("section present"))
            .collect();
        assert!(
            positions.windows(2).all(|w| w[0] < w[1]),
            "sections must be listed in template order: {positions:?}"
        );
    }

    #[test]
    fn the_section_list_is_the_one_the_prompt_renders() {
        // Guards against the two drifting apart: the constant is the template.
        let prompt = summary_system_prompt();
        for (index, (name, what)) in SUMMARY_SECTIONS.iter().enumerate() {
            assert!(
                prompt.contains(&format!("{}. ## {name} — {what}", index + 1)),
                "section {index} rendered differently from the constant"
            );
        }
    }
}
