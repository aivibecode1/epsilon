//! Stuck-run guards (port plan item 7; Hermes `agent/tool_guardrails.py:186,334`,
//! `agent/empty_response_guard.py:234`, `agent/repetition_guard.py:43`).
//!
//! The doom-loop brake in `exec.rs` catches one failure mode: the *same call* repeated. A stuck
//! run usually looks different — four different commands that all fail the same way, or a model
//! that answers with nothing and retries forever. Both burn the same resource, and neither is
//! visible to a call-identity check.
//!
//! Three predicates, all pure:
//!
//! - [`ProgressGuard`] hashes the *result* the model just received. Identical results in a row
//!   mean the model is not learning anything, whatever the commands were.
//! - [`empty_turn_reason`] decides whether a "successful" final turn actually said anything, and
//!   whether a long reply is degenerate repetition rather than prose.
//!
//! Guidance is appended to the result the model reads, at the point the streak is noticed — the
//! cheapest place to intervene, because it arrives in the same breath as the thing it is about.

use std::collections::HashMap;

/// How much of a result's text participates in the signature. Enough to distinguish a compiler
/// error from a different compiler error; short enough that trailing noise does not.
const SIGNATURE_CHARS: usize = 200;

/// A result must be at least this long before degenerate-repetition detection applies: a short
/// answer that happens to repeat a word is not a loop.
const REPETITION_MIN_LINES: usize = 20;

/// The share of lines one repeated line must take before a reply counts as degenerate.
const REPETITION_LINE_SHARE: f64 = 0.5;

/// Stable hash of what the model would see. FNV-1a: no dependency, and the exact function does
/// not matter as long as it is stable within a process.
fn hash_bytes(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Signature of one tool result: error-ness plus a normalized head of its text.
///
/// Normalization is the point: whitespace and casing differ between two runs that told the model
/// exactly the same thing, and a byte-diff would call those progress.
pub fn result_signature(is_error: bool, text: &str) -> u64 {
    let normalized: String = text
        .chars()
        .take(SIGNATURE_CHARS)
        .flat_map(|c| c.to_lowercase())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    hash_bytes(&format!("{is_error}\u{0}{normalized}"))
}

/// What the guard concluded from one result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// The model received something it had not just received.
    New,
    /// The same result as last time; `repeats` counts the consecutive occurrences (1 = the first
    /// time this signature was seen, 2 = a repeat, …).
    Repeated {
        /// Consecutive occurrences of this signature, including this one.
        repeats: usize,
    },
}

/// Counts consecutive identical tool results.
#[derive(Debug, Default)]
pub struct ProgressGuard {
    last: Option<u64>,
    repeats: usize,
}

impl ProgressGuard {
    /// A fresh guard: nothing observed yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold in one result.
    pub fn observe(&mut self, is_error: bool, text: &str) -> Progress {
        let sig = result_signature(is_error, text);
        if self.last == Some(sig) {
            self.repeats += 1;
        } else {
            self.last = Some(sig);
            self.repeats = 1;
        }
        if self.repeats == 1 {
            Progress::New
        } else {
            Progress::Repeated {
                repeats: self.repeats,
            }
        }
    }

    /// Whether the model is currently stuck on a repeated result.
    pub fn stalled_at(&self, threshold: usize) -> bool {
        threshold > 0 && self.repeats >= threshold
    }

    /// Consecutive repeats so far (1 = the last result was new).
    pub fn streak(&self) -> usize {
        self.repeats
    }
}

/// The guidance appended to a repeated result, at two severities.
///
/// The model is told what was observed and what to do about it — never that the tool failed,
/// because it did not.
pub fn stuck_guidance(repeats: usize, hard: bool) -> String {
    if hard {
        format!(
            "[harness] this result has now been identical {repeats} times in a row. Repeating \
             this approach will not change it: report the blocker to the user, or change \
             strategy (different tool, different arguments, or a smaller step)."
        )
    } else {
        format!(
            "[harness] this result is identical to the previous one ({repeats} times in a row) \
             — whatever was just tried did not move anything. Consider a different approach."
        )
    }
}

/// Why a final turn said nothing useful.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmptyReason {
    /// No text and no tool calls: a completed stream that produced nothing.
    NoContent,
    /// One line repeated through the reply.
    Repetition {
        /// The line that dominates.
        line: String,
        /// How many times it appears.
        count: usize,
        /// Total non-empty lines in the reply.
        total: usize,
    },
}

impl EmptyReason {
    /// A one-line explanation for the run's ending, in the model's own terms.
    pub fn describe(&self) -> String {
        match self {
            EmptyReason::NoContent => {
                "the model returned an empty turn (no text, no tool calls)".to_string()
            }
            EmptyReason::Repetition { count, total, line } => format!(
                "the model's reply was degenerate repetition ({count} of {total} lines are the \
                 same: {:?})",
                line.chars().take(60).collect::<String>()
            ),
        }
    }
}

/// Whether a finished turn is empty or degenerate.
///
/// `tool_calls` is the count of calls in the turn: a turn that called a tool did something, even
/// if it said nothing, so it is never "empty".
pub fn empty_turn_reason(text: &str, tool_calls: usize) -> Option<EmptyReason> {
    if tool_calls > 0 {
        return None;
    }
    if text.trim().is_empty() {
        return Some(EmptyReason::NoContent);
    }
    repetition_reason(text)
}

/// Degenerate repetition: one non-empty line accounting for most of the reply's lines.
fn repetition_reason(text: &str) -> Option<EmptyReason> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() < REPETITION_MIN_LINES {
        return None;
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for line in &lines {
        *counts.entry(line).or_insert(0) += 1;
    }
    let (line, count) = counts.into_iter().max_by_key(|(_, c)| *c)?;
    let share = count as f64 / lines.len() as f64;
    if share >= REPETITION_LINE_SHARE {
        Some(EmptyReason::Repetition {
            line: line.to_string(),
            count,
            total: lines.len(),
        })
    } else {
        None
    }
}

/// The nudge injected when a turn came back empty. Rides the ordinary user-message path, so it is
/// visible in the transcript and in the session file.
pub fn nudge_for(reason: &EmptyReason, attempt: usize) -> String {
    match reason {
        EmptyReason::NoContent => format!(
            "[harness] your last reply was empty (attempt {attempt}). If the task is finished, \
             say what you did and stop. Otherwise continue with the next concrete step."
        ),
        EmptyReason::Repetition { line, .. } => format!(
            "[harness] your last reply repeated one line instead of answering (attempt \
             {attempt}): {:?}. Answer the question directly, or state what is blocking you.",
            line.chars().take(60).collect::<String>()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn different_commands_with_the_same_failure_are_detected_as_stuck() {
        // The failure mode the doom-loop brake cannot see: the *call* differs every time.
        let mut guard = ProgressGuard::new();
        let failure = "error: could not compile `foo`\n  --> src/lib.rs:3:1";
        assert_eq!(guard.observe(true, failure), Progress::New);
        assert_eq!(
            guard.observe(true, &format!("{failure}   ")),
            Progress::Repeated { repeats: 2 },
            "trailing whitespace is not progress"
        );
        assert_eq!(
            guard.observe(true, &failure.to_uppercase()),
            Progress::Repeated { repeats: 3 },
            "and neither is case"
        );
        assert!(guard.stalled_at(3));
    }

    #[test]
    fn a_different_result_resets_the_streak() {
        let mut guard = ProgressGuard::new();
        guard.observe(true, "first failure");
        assert_eq!(guard.observe(true, "a different failure"), Progress::New);
        assert_eq!(guard.streak(), 1, "progress clears the streak");
        assert!(!guard.stalled_at(2));
    }

    #[test]
    fn an_error_and_a_success_with_the_same_text_are_not_the_same_result() {
        let mut guard = ProgressGuard::new();
        guard.observe(true, "no matches found");
        assert_eq!(
            guard.observe(false, "no matches found"),
            Progress::New,
            "a command that started working is progress, even with identical output"
        );
    }

    #[test]
    fn the_signature_ignores_noise_past_the_head() {
        // The shared part has to be longer than the hashed window, or the difference would be
        // inside the signature and this would be testing nothing.
        let head = "identical preamble ".repeat(20); // ~400 chars, beyond SIGNATURE_CHARS
        let mut guard = ProgressGuard::new();
        guard.observe(false, &format!("{head}tail one"));
        assert_eq!(
            guard.observe(false, &format!("{head}tail two, which is quite different")),
            Progress::Repeated { repeats: 2 },
            "a wall of logs that differs only in its tail is the same result to the model"
        );
        // ...but a difference inside the window is progress, and resets the streak.
        guard.observe(false, "first failure: missing file");
        assert_eq!(
            guard.observe(false, "second failure: permission denied"),
            Progress::New
        );
    }

    #[test]
    fn guidance_says_what_was_observed_and_never_that_the_tool_failed() {
        let soft = stuck_guidance(3, false);
        assert!(soft.contains("identical to the previous one"));
        assert!(soft.contains("different approach"));
        assert!(!soft.contains("failed"), "{soft}");
        let hard = stuck_guidance(6, true);
        assert!(hard.contains("6 times in a row"));
        assert!(hard.contains("report the blocker"), "{hard}");
    }

    #[test]
    fn an_empty_turn_is_empty_but_a_silent_tool_call_is_not() {
        assert_eq!(
            empty_turn_reason("", 0),
            Some(EmptyReason::NoContent),
            "nothing said and nothing done"
        );
        assert_eq!(
            empty_turn_reason("   \n  ", 0),
            Some(EmptyReason::NoContent)
        );
        assert_eq!(
            empty_turn_reason("", 1),
            None,
            "a turn that called a tool did something"
        );
        assert_eq!(empty_turn_reason("a normal short answer", 0), None);
    }

    #[test]
    fn a_reply_that_repeats_one_line_is_degenerate() {
        let text = format!("working on it\n{}", "let me try again\n".repeat(30));
        match empty_turn_reason(&text, 0) {
            Some(EmptyReason::Repetition { count, total, .. }) => {
                assert_eq!(count, 30);
                assert_eq!(total, 31);
            }
            other => panic!("expected repetition, got {other:?}"),
        }
        // A short reply is never judged, however repetitive.
        assert_eq!(empty_turn_reason(&"x\n".repeat(5), 0), None);
        // Nor is a long, varied one.
        let varied: String = (0..40).map(|i| format!("line {i}\n")).collect();
        assert_eq!(empty_turn_reason(&varied, 0), None);
    }

    #[test]
    fn the_nudge_tells_the_model_what_was_wrong() {
        let nudge = nudge_for(&EmptyReason::NoContent, 1);
        assert!(nudge.contains("empty") && nudge.contains("attempt 1"));
        let nudge = nudge_for(
            &EmptyReason::Repetition {
                line: "let me try again".into(),
                count: 30,
                total: 31,
            },
            2,
        );
        assert!(nudge.contains("repeated one line"), "{nudge}");
        assert!(nudge.contains("state what is blocking you"), "{nudge}");
    }

    #[test]
    fn a_zero_threshold_never_reports_a_stall() {
        let mut guard = ProgressGuard::new();
        for _ in 0..10 {
            guard.observe(false, "same");
        }
        assert!(
            !guard.stalled_at(0),
            "a disabled threshold must not fire (misconfiguration is not a stall)"
        );
    }
}
