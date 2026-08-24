//! Session text search over the JSONL store (pi `packages/agent/src/search` parity).
//!
//! pi factors session search into a dedicated module: given a storage backend, scan
//! entries for a query and return typed hits with context. Epsilon's store already
//! keeps an offset index, so the scan is a single sequential pass of
//! [`SessionStore::read_entry`] calls — O(entries) with no extra memory beyond the hits.
//!
//! Semantics ported from pi's scanning contract:
//! - case-insensitive substring match over user text, assistant text/thinking,
//!   tool-call arguments, tool-result output, and error messages;
//! - results capped (`SearchOptions::limit`) with newest-first ordering so "what did I
//!   recently do about X" answers in one screen;
//! - each hit carries a short snippet around the match, the entry `seq`, and the role.

use crate::store::{SessionStore, StoreError};
use epsilon_contracts::message::Message;
use epsilon_contracts::session_entry::EntryKind;

/// One search hit.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSearchHit {
    /// Seq of the entry containing the match (usable with [`SessionStore::read_entry`]).
    pub seq: u64,
    /// Role label of the matched message (`user` / `assistant` / `tool_result`).
    pub role: &'static str,
    /// Short excerpt around the first match (±40 chars of context).
    pub snippet: String,
}

/// Search options.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    /// Maximum number of hits returned. `0` means unlimited.
    pub limit: usize,
    /// Restrict the scan to the active branch only (skip abandoned forks).
    pub active_branch_only: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            limit: 50,
            active_branch_only: false,
        }
    }
}

/// Extract every searchable haystack from one message, labeled by role.
fn haystacks(message: &Message) -> Vec<(&'static str, String)> {
    match message {
        Message::User(m) => vec![(
            "user",
            m.content
                .iter()
                .filter_map(|c| match c {
                    epsilon_contracts::content::UserContent::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        )],
        Message::Assistant(m) => {
            let mut out = Vec::new();
            let mut text = String::new();
            for block in &m.content {
                match block {
                    epsilon_contracts::content::ContentBlock::Text { text: t }
                    | epsilon_contracts::content::ContentBlock::Thinking { text: t } => {
                        text.push_str(t);
                    }
                    epsilon_contracts::content::ContentBlock::ToolCall {
                        name, arguments, ..
                    } => {
                        out.push(("assistant", format!("{name} {arguments}")));
                    }
                    _ => {}
                }
            }
            if !text.is_empty() {
                out.push(("assistant", text));
            }
            if let Some(err) = &m.error_message {
                out.push(("assistant", err.clone()));
            }
            out
        }
        Message::ToolResult(m) => vec![(
            "tool_result",
            m.content
                .iter()
                .filter_map(|c| match c {
                    epsilon_contracts::content::ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        )],
        Message::Custom(_) => Vec::new(),
    }
}

/// Build a ±40-char excerpt around the (case-insensitive) match inside `haystack`.
///
/// Plan 002: the window is computed and sliced on ONE string (the lowercased copy).
/// Mapping offsets from the lowercased copy back to the original is unsound —
/// `to_lowercase()` can change byte length (e.g. U+0130 İ expands) so original-string
/// slicing could split a char or panic. The returned snippet is therefore lowercase;
/// for scanning UX that is acceptable and the hit text itself stays intact on disk.
fn snippet_for(haystack: &str, needle_lower: &str) -> Option<String> {
    let lower = haystack.to_lowercase();
    let byte_pos = lower.find(needle_lower)?;
    // Snap to a char boundary so multi-byte characters are never split.
    let start = lower[..byte_pos]
        .char_indices()
        .rev()
        .nth(39)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let end_pos = byte_pos + needle_lower.len();
    let end = lower[end_pos..]
        .char_indices()
        .nth(39)
        .map(|(i, _)| end_pos + i)
        .unwrap_or(lower.len());
    let mut snippet = String::new();
    if start > 0 {
        snippet.push('…');
    }
    snippet.push_str(lower[start..end].trim());
    if end < haystack.len() {
        snippet.push('…');
    }
    Some(snippet)
}

/// Search all (or active-branch-only) message entries for `query`, newest hits first.
///
/// Empty/whitespace queries return no hits rather than matching everything.
pub fn search_session(
    store: &SessionStore,
    query: &str,
    opts: &SearchOptions,
) -> Result<Vec<SessionSearchHit>, StoreError> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let needle = query.trim().to_lowercase();

    let seqs: Vec<u64> = if opts.active_branch_only {
        store.branch_entries()?.into_iter().map(|e| e.seq).collect()
    } else {
        (0..store.len()).collect()
    };

    let mut hits = Vec::new();
    for seq in seqs.into_iter().rev() {
        let Some(entry) = store.read_entry(seq)? else {
            continue;
        };
        let EntryKind::Message { message, .. } = &entry.kind else {
            continue;
        };
        for (role, hay) in haystacks(message) {
            if let Some(snippet) = snippet_for(&hay, &needle) {
                hits.push(SessionSearchHit { seq, role, snippet });
                break; // one hit per entry is enough for scanning UX
            }
        }
        if opts.limit > 0 && hits.len() >= opts.limit {
            break;
        }
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use epsilon_contracts::agent_event::TurnEndReason;
    use epsilon_contracts::content::{ContentBlock, UserContent};
    use epsilon_contracts::message::{
        AssistantMessage, StopReason, ToolResultMessage, Usage, UserMessage,
    };

    fn user_msg(text: &str) -> Message {
        Message::User(UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            content: vec![UserContent::Text {
                text: text.to_string(),
            }],
        })
    }

    fn assistant_msg(text: &str) -> Message {
        Message::Assistant(AssistantMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![ContentBlock::Text { text: text.into() }],
            stop_reason: StopReason::EndTurn,
            error_message: None,
            usage: Usage::default(),
        })
    }

    fn seed(path: &std::path::Path) -> SessionStore {
        let mut s = SessionStore::open(path).unwrap();
        s.append(EntryKind::Message {
            message: user_msg("fix the flaky auth tests"),
            api_request_sha256: None,
        })
        .unwrap();
        s.append(EntryKind::Message {
            message: assistant_msg("reading tests/auth_test.rs"),
            api_request_sha256: None,
        })
        .unwrap();
        s.append(EntryKind::Message {
            message: Message::ToolResult(ToolResultMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: epsilon_contracts::new_id("call"),
                tool_name: "bash".into(),
                content: vec![ContentBlock::Text {
                    text: "test result: ok. 12 passed".into(),
                }],
                details: serde_json::json!({}),
                usage: None,
                is_error: false,
            }),
            api_request_sha256: None,
        })
        .unwrap();
        s.append(EntryKind::TurnEnd {
            reason: TurnEndReason::Completed,
        })
        .unwrap();
        s
    }

    #[test]
    fn finds_user_assistant_and_tool_hits_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let store = seed(&tmp.path().join("s.jsonl"));
        let hits = search_session(&store, "test", &SearchOptions::default()).unwrap();
        assert!(hits.len() >= 3);
        assert_eq!(hits[0].role, "tool_result"); // newest entry containing "test"
        assert!(hits.iter().any(|h| h.role == "user" && h.seq == 0));
        assert!(
            hits.iter()
                .all(|h| h.snippet.to_lowercase().contains("test"))
        );
    }

    #[test]
    fn case_insensitive_with_snippet_context() {
        let tmp = tempfile::tempdir().unwrap();
        let store = seed(&tmp.path().join("s.jsonl"));
        let hits = search_session(&store, "FLAKY", &SearchOptions::default()).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].seq, 0);
        assert_eq!(hits[0].snippet, "fix the flaky auth tests");
    }

    #[test]
    fn empty_query_and_misses_return_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = seed(&tmp.path().join("s.jsonl"));
        assert!(
            search_session(&store, "   ", &SearchOptions::default())
                .unwrap()
                .is_empty()
        );
        assert!(
            search_session(&store, "quantum-unicorn", &SearchOptions::default())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn limit_caps_results() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("s.jsonl");
        let mut store = SessionStore::open(&path).unwrap();
        for _ in 0..10 {
            store
                .append(EntryKind::Message {
                    message: user_msg("needle here"),
                    api_request_sha256: None,
                })
                .unwrap();
        }
        let hits = search_session(
            &store,
            "needle",
            &SearchOptions {
                limit: 3,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].seq, 9); // newest first
    }

    #[test]
    fn snippet_after_expanding_lowercase_char_does_not_panic() {
        // U+0130 (İ) expands to 3 bytes when lowercased; a needle after it used to
        // produce mismatched offsets between the lowercased and original strings.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("s.jsonl");
        let mut store = SessionStore::open(&path).unwrap();
        store
            .append(EntryKind::Message {
                message: user_msg("İstanbul report: needle here"),
                api_request_sha256: None,
            })
            .unwrap();
        let hits = search_session(&store, "needle", &SearchOptions::default()).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.contains("needle"));
    }

    #[test]
    fn skips_non_message_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("s.jsonl");
        let mut store = SessionStore::open(&path).unwrap();
        store
            .append(EntryKind::ModelChange {
                provider: "anthropic".into(),
                model_id: "claude-x".into(),
            })
            .unwrap();
        let hits = search_session(&store, "anthropic", &SearchOptions::default()).unwrap();
        assert!(hits.is_empty());
    }
}
