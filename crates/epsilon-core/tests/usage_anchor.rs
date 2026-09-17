//! M64: the compaction trigger has to *measure*, not guess.
//!
//! The roadmap's Phase 2 acceptance criterion for usage-anchored accounting
//! (`audit/04-roadmap.md`) is that a scripted long session compacts within ±10 % of the
//! provider's true count. The unit tests in `compact.rs` pin the arithmetic; these run the
//! real loop over a provider that reports what a real wire reports — usage computed from the
//! request it was handed — so the wiring is under test too: the anchor is recorded, kept while
//! it still describes the history, and dropped when it does not.
//!
//! The provider is *honest by construction*: it counts the request it received, and its ratio
//! is a parameter. A prose-like ratio (few tokens per character) and a dense ratio (about one
//! token per character) are the two directions the old `chars / 4` heuristic got wrong, and
//! each scenario asserts the decision now follows the provider rather than the heuristic.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use epsilon_contracts::agent_event::AgentEvent;
use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage, UserMessage};
use epsilon_contracts::stream::StreamEvent;
use epsilon_core::{AutoAllow, LoopLimits, ToolRegistry};
use epsilon_llm::{BoxEventStream, EventStream, LlmContext, ModelRef, StreamFn};

/// A provider whose reported prompt size is `chars / per_token_chars` over the request it
/// received, and whose answer is one short line.
///
/// `per_token_chars == 4` reproduces the heuristic exactly (a useful control: if the test
/// passes there, it proves nothing). The scenarios use ratios on either side of it.
struct Honest {
    per_token_chars: usize,
    /// Every prompt size this provider reported, oldest first.
    reported: Arc<Mutex<Vec<usize>>>,
}

struct Fixed {
    events: VecDeque<StreamEvent>,
}

#[async_trait]
impl EventStream for Fixed {
    async fn next(&mut self) -> Option<StreamEvent> {
        self.events.pop_front()
    }
}

#[async_trait]
impl StreamFn for Honest {
    async fn stream(&self, model: &ModelRef, ctx: LlmContext) -> BoxEventStream {
        // What the wire would bill for: the whole request, counted the way this provider
        // counts. Real wires differ in the ratio, not in the principle.
        let chars: usize = ctx
            .messages
            .iter()
            .map(|m| serde_json::to_string(m).map(|s| s.len()).unwrap_or(0))
            .sum::<usize>()
            + ctx.system_prompt.len();
        let input = (chars / self.per_token_chars).max(1) as u64;
        if let Ok(mut log) = self.reported.lock() {
            log.push(input as usize);
        }

        let message = AssistantMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            provider: "honest".into(),
            model_id: model.id.clone(),
            content: vec![epsilon_contracts::content::ContentBlock::text("ok")],
            stop_reason: StopReason::EndTurn,
            error_message: None,
            usage: Usage {
                input,
                output: 1,
                total_tokens: input + 1,
                ..Default::default()
            },
        };
        Box::new(Fixed {
            events: vec![StreamEvent::Done { message }].into(),
        })
    }
}

fn user(text: &str) -> Message {
    Message::User(UserMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: 0,
        content: vec![UserContent::Text {
            text: text.to_string(),
        }],
    })
}

/// One assistant turn that says `text` — the loop appends it, so the next request grows.
fn turns(n: usize, chars: usize) -> Vec<Message> {
    (0..n)
        .map(|i| user(&format!("turn {i} {}", "x".repeat(chars))))
        .collect()
}

fn agent_with(
    per_token_chars: usize,
    limits: LoopLimits,
) -> (Arc<epsilon_core::Agent>, Arc<Mutex<Vec<usize>>>) {
    let reported = Arc::new(Mutex::new(Vec::new()));
    let stream = Arc::new(Honest {
        per_token_chars,
        reported: Arc::clone(&reported),
    }) as Arc<dyn StreamFn>;
    let agent = epsilon_core::Agent::new(
        limits,
        stream,
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    (agent, reported)
}

fn compactions(rx: &mut tokio::sync::broadcast::Receiver<Arc<AgentEvent>>) -> usize {
    let mut started = 0;
    loop {
        match rx.try_recv() {
            Ok(ev) => {
                if matches!(*ev, AgentEvent::CompactionStarted) {
                    started += 1;
                }
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
    started
}

/// Characters of LLM-visible text in a transcript, counted the way the crate's fallback
/// heuristic counts them. Used to state — in a test — what a char-based trigger would have
/// concluded *without* re-implementing the crate's rounding.
fn visible_chars(history: &[Message]) -> usize {
    fn block(b: &epsilon_contracts::content::ContentBlock) -> usize {
        match b {
            epsilon_contracts::content::ContentBlock::Text { text }
            | epsilon_contracts::content::ContentBlock::Thinking { text } => text.chars().count(),
            epsilon_contracts::content::ContentBlock::ToolCall { arguments, .. } => {
                arguments.to_string().chars().count()
            }
            epsilon_contracts::content::ContentBlock::Image(_) => 2_048,
        }
    }
    history
        .iter()
        .map(|m| match m {
            Message::User(u) => u
                .content
                .iter()
                .map(|c| match c {
                    UserContent::Text { text } => text.chars().count(),
                    UserContent::Image(_) => 2_048,
                })
                .sum(),
            Message::Assistant(a) => a.content.iter().map(block).sum(),
            Message::ToolResult(t) => t.content.iter().map(block).sum(),
            Message::Custom(_) => 0,
        })
        .sum()
}

/// Prose: the provider bills far fewer tokens than `chars / 4` predicts.
///
/// Without the anchor the char heuristic sees ~32 000 characters and compacts away context
/// that the provider says fits comfortably. With it, the session runs to completion intact.
#[tokio::test]
async fn a_prose_like_session_is_not_compacted_early() {
    let threshold = 6_000;
    let limits = LoopLimits {
        // Well above what the provider reports (~32 000 chars / 16 ≈ 2 000) and well below
        // what the char heuristic claims (~8 000).
        compaction_threshold_tokens: threshold,
        compaction_tail_tokens: 300,
        max_turns: 40,
        ..Default::default()
    };
    let (agent, reported) = agent_with(16, limits);
    let mut rx = agent.subscribe();

    for msg in turns(8, 4_000) {
        agent.run(msg).await.expect("run");
    }

    let history = agent.history();
    let truth = *reported.lock().unwrap().last().expect("a report");
    let by_chars = visible_chars(&history) / 4;
    let estimate = agent.estimated_context_tokens();

    assert!(
        by_chars > threshold,
        "fixture assumption: a char-based trigger would have compacted ({by_chars} > {threshold})"
    );
    assert_eq!(
        compactions(&mut rx),
        0,
        "the provider's own count was under budget, so nothing should have been compacted"
    );
    assert_eq!(
        history.len(),
        16,
        "and none of the eight turns was dropped from the transcript"
    );
    assert!(
        truth < threshold && estimate < by_chars / 2,
        "the estimate must follow the provider ({estimate}, reported {truth}) rather than \
         the heuristic ({by_chars})"
    );
    assert!(
        estimate >= truth,
        "and it may never fall below the provider's last report"
    );
}

/// Dense content (code, CJK, minified JSON): the provider bills *more* than `chars / 4` — the
/// direction that overflows a window if the harness trusts the heuristic.
#[tokio::test]
async fn a_dense_session_is_compacted_when_the_provider_says_so() {
    let threshold = 6_000;
    let limits = LoopLimits {
        // Already past the budget after ONE turn of dense content; a char-based trigger would
        // need four more turns to get there.
        compaction_threshold_tokens: threshold,
        compaction_tail_tokens: 300,
        max_turns: 40,
        ..Default::default()
    };
    let (agent, reported) = agent_with(1, limits);
    let mut rx = agent.subscribe();

    agent
        .run(user(&format!("turn 0 {}", "x".repeat(4_000))))
        .await
        .expect("run");

    let after_one = agent.estimated_context_tokens();
    let by_chars = visible_chars(&agent.history()) / 4;
    let truth = *reported.lock().unwrap().last().expect("a report");
    assert!(
        after_one > threshold && by_chars < threshold,
        "after one dense turn the provider says {truth} (estimate {after_one}) while the char \
         heuristic says {by_chars}: only the anchored number is over budget"
    );

    for msg in turns(6, 4_000).into_iter().skip(1) {
        agent.run(msg).await.expect("run");
    }

    assert!(
        compactions(&mut rx) >= 1,
        "the provider's count crossed the budget, so compaction must have fired"
    );
}

/// The estimate tracks the provider across a long session: after each completed turn the
/// harness's number is within 10 % of what the provider just billed (nothing was appended
/// between the report and the read, so the only slack is the anchor's own tail term).
#[tokio::test]
async fn the_estimate_tracks_the_provider_within_ten_percent() {
    let limits = LoopLimits {
        compaction_enabled: false, // isolate the accounting from compaction itself
        max_turns: 60,
        ..Default::default()
    };
    let (agent, reported) = agent_with(3, limits);

    for msg in turns(12, 2_000) {
        agent.run(msg).await.expect("run");
    }

    let reports = reported.lock().unwrap().clone();
    assert!(
        reports.len() >= 12,
        "one report per turn: {}",
        reports.len()
    );
    let truth = *reports.last().unwrap() as f64;
    let estimate = agent.estimated_context_tokens() as f64;
    let drift = (estimate - truth).abs() / truth;
    assert!(
        drift <= 0.10,
        "estimate {estimate} vs provider {truth} — {:.1}% off",
        drift * 100.0
    );
}

/// A provider that reports nothing must not poison the estimate: the heuristic takes over and
/// compaction still happens on its own terms.
#[tokio::test]
async fn a_provider_that_reports_no_usage_falls_back_to_the_heuristic() {
    struct Silent;
    #[async_trait]
    impl StreamFn for Silent {
        async fn stream(&self, model: &ModelRef, _ctx: LlmContext) -> BoxEventStream {
            let message = AssistantMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: 0,
                provider: "silent".into(),
                model_id: model.id.clone(),
                content: vec![epsilon_contracts::content::ContentBlock::text(
                    "y".repeat(4_000),
                )],
                stop_reason: StopReason::EndTurn,
                error_message: None,
                usage: Usage::default(),
            };
            Box::new(Fixed {
                events: vec![StreamEvent::Done { message }].into(),
            })
        }
    }

    let limits = LoopLimits {
        compaction_threshold_tokens: 2_000,
        compaction_tail_tokens: 300,
        max_turns: 20,
        ..Default::default()
    };
    let agent = epsilon_core::Agent::new(
        limits,
        Arc::new(Silent),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    let mut rx = agent.subscribe();

    for msg in turns(4, 2_000) {
        agent.run(msg).await.expect("run");
    }

    assert!(
        compactions(&mut rx) >= 1,
        "with no reported usage the char heuristic is the only signal left, and it is over budget"
    );
}
