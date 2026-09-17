//! M69: cache-miss attribution through the real loop.
//!
//! `epsilon-core/src/cache_stats.rs` decides *why* a prompt was re-billed; these scenarios prove
//! the loop feeds it the three facts it needs — the provider's usage, the model id, and the
//! stable-tier fingerprint — and that the resulting report reaches a user through
//! `Agent::cache_report()`.
//!
//! The provider here behaves like a wire in good health: the first call writes the cache, later
//! calls read it. Then it stops reading it, which is what a cache break looks like from inside
//! the harness.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage, UserMessage};
use epsilon_contracts::stream::StreamEvent;
use epsilon_core::{AutoAllow, LoopLimits, ToolRegistry};
use epsilon_llm::{BoxEventStream, EventStream, LlmContext, ModelRef, StreamFn};

/// What the fake wire reports for the next call: `(input, cache_read, cache_write)`.
type Script = Arc<Mutex<Vec<(u64, u64, u64)>>>;

struct Wire {
    script: Script,
    /// Model ids seen, in call order — proves the loop hands over the *current* model.
    seen: Arc<Mutex<Vec<String>>>,
}

struct One(Option<StreamEvent>);

#[async_trait]
impl EventStream for One {
    async fn next(&mut self) -> Option<StreamEvent> {
        self.0.take()
    }
}

#[async_trait]
impl StreamFn for Wire {
    async fn stream(&self, model: &ModelRef, _ctx: LlmContext) -> BoxEventStream {
        if let Ok(mut seen) = self.seen.lock() {
            seen.push(model.id.clone());
        }
        let (input, read, write) = {
            let mut script = self.script.lock().unwrap();
            if script.is_empty() {
                script.push((100, 0, 0));
            }
            script.remove(0)
        };
        let usage = Usage {
            input,
            output: 5,
            cache_read: read,
            cache_write: write,
            total_tokens: input + 5 + read + write,
        };
        let message = AssistantMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            provider: "wire".into(),
            model_id: model.id.clone(),
            content: vec![epsilon_contracts::content::ContentBlock::text("ok")],
            stop_reason: StopReason::EndTurn,
            error_message: None,
            usage,
        };
        Box::new(One(Some(StreamEvent::Done { message })))
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

fn agent_with(script: Script, seen: Arc<Mutex<Vec<String>>>) -> Arc<epsilon_core::Agent> {
    let stream = Arc::new(Wire { script, seen }) as Arc<dyn StreamFn>;
    epsilon_core::Agent::new(
        LoopLimits::default(),
        stream,
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    )
}

#[tokio::test]
async fn a_warm_session_reports_hits_and_no_waste() {
    let script: Script = Arc::new(Mutex::new(vec![
        (1_000, 0, 1_000), // cold: the prefix is written to the cache
        (100, 900, 0),     // served
        (100, 900, 0),     // served
    ]));
    let agent = agent_with(script, Arc::new(Mutex::new(Vec::new())));
    for i in 0..3 {
        agent.run(user(&format!("turn {i}"))).await.expect("run");
    }

    let report = agent.cache_report().join("\n");
    assert!(
        report.contains("3 call(s)") && report.contains("2 served from cache"),
        "{report}"
    );
    assert!(
        report.contains("read 1,800 tok"),
        "cache reads are summed: {report}"
    );
    assert!(
        !report.contains("re-billed"),
        "a warm session wastes nothing: {report}"
    );
}

#[tokio::test]
async fn a_cache_that_stops_being_used_is_attributed_and_priced() {
    let script: Script = Arc::new(Mutex::new(vec![
        (1_000, 0, 1_000), // cold
        (100, 900, 0),     // served
        (4_000, 0, 0),     // the cache stopped answering: 4k tokens at full price
    ]));
    let agent = agent_with(script, Arc::new(Mutex::new(Vec::new())));
    // A catalog model, so the report can price the waste (the id has to be one the catalog
    // actually carries — a bare `deepseek-chat` is not a row, which M63 learned the hard way).
    agent.set_model_id("claude-haiku-4-5");
    for i in 0..3 {
        agent.run(user(&format!("turn {i}"))).await.expect("run");
    }

    let report = agent.cache_report().join("\n");
    assert!(report.contains("re-billed 4,000 tok"), "{report}");
    assert!(
        report.contains("re-read by the provider"),
        "same model, same prefix, no gap — the only honest thing to say: {report}"
    );
    assert!(
        report.contains('$'),
        "a catalog model gets a price: {report}"
    );
}

#[tokio::test]
async fn a_model_switch_is_named_as_the_cause() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let script: Script = Arc::new(Mutex::new(vec![
        (1_000, 0, 1_000), // cold on the first model
        (2_500, 0, 0),     // different model: a cache is namespaced per model
    ]));
    let agent = agent_with(script, Arc::clone(&seen));
    agent.set_model_id("faux-1");
    agent.run(user("first")).await.expect("run");
    agent.set_model_id("faux-2");
    agent.run(user("second")).await.expect("run");

    let ids = seen.lock().unwrap().clone();
    assert_eq!(
        ids,
        vec!["faux-1", "faux-2"],
        "the loop passed the live model"
    );

    let report = agent.cache_report().join("\n");
    assert!(
        report.contains("model switch (faux-1 → faux-2)"),
        "{report}"
    );
    assert!(report.contains("2,500 tok"), "{report}");
}

#[tokio::test]
async fn a_provider_that_never_mentions_caching_is_not_reported_as_wasteful() {
    let script: Script = Arc::new(Mutex::new(vec![
        (1_000, 0, 0),
        (1_000, 0, 0),
        (1_000, 0, 0),
    ]));
    let agent = agent_with(script, Arc::new(Mutex::new(Vec::new())));
    for i in 0..3 {
        agent.run(user(&format!("turn {i}"))).await.expect("run");
    }
    let report = agent.cache_report().join("\n");
    assert!(
        report.contains("reports no cache traffic"),
        "silence is not a miss: {report}"
    );
    assert!(!report.contains("re-billed"), "{report}");
}
