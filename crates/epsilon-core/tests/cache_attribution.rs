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
    /// Whether this provider reports cache fields at all.
    ///
    /// **A field, not a global.** M196 first tried a `static` flag and the test
    /// that sets it poisoned the one that reads it, because the tests in this
    /// binary run concurrently on separate tasks in one process — a static is
    /// shared state between tests that are supposed to be independent. A field
    /// on the provider makes the property per-test by construction, which also
    /// means a test cannot be made to pass or fail by running alone.
    reports_cache: bool,
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
            // This file exists to exercise cache attribution, so its provider is
            // one that reports cache traffic (K1) -- unless a test is
            // specifically about a provider that does NOT. M196: this was
            // hardcoded `true`, so `a_provider_that_never_mentions_caching...`
            // was never actually testing a silent provider; it was testing a
            // reporting one with no hits, and passed for the wrong reason.
            cache_reported: self.reports_cache,
            // No reasoning split in this fixture's scripted replies.
            reasoning: None,
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

/// Set while a test is exercising a provider that reports no cache fields.
///
/// A static rather than a parameter on `Wire` because `agent_with` is called by
/// every test in the file and threading a flag through all of them would make
/// the common case harder to read for one test's sake. It is read only at
/// response-construction time, and the tests that use it are `#[tokio::test]`s
/// running on their own task, so the window is not shared.
fn agent_with(script: Script, seen: Arc<Mutex<Vec<String>>>) -> Arc<epsilon_core::Agent> {
    agent_with_cache_reporting(script, seen, true)
}

/// As [`agent_with`], with the provider's cache-reporting behaviour chosen.
fn agent_with_cache_reporting(
    script: Script,
    seen: Arc<Mutex<Vec<String>>>,
    reports_cache: bool,
) -> Arc<epsilon_core::Agent> {
    let stream = Arc::new(Wire {
        script,
        seen,
        reports_cache,
    }) as Arc<dyn StreamFn>;
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
    // M196: this is what makes the test name true. Without it the `Wire`
    // provider reports cache fields and the test is asserting about a
    // *reporting* provider with no hits -- a different thing entirely.
    let agent = agent_with_cache_reporting(script, Arc::new(Mutex::new(Vec::new())), false);
    for i in 0..3 {
        agent.run(user(&format!("turn {i}"))).await.expect("run");
    }
    let report = agent.cache_report().join("\n");
    // M196: the wording changed because "reports no cache traffic" claimed the
    // PROVIDER had said nothing, which is a different claim from "nothing has
    // hit yet" -- and this test could not tell the two apart. The intent is
    // unchanged: a provider that says nothing about caching must not be told it
    // is missing.
    assert!(
        report.contains("does not report cache fields"),
        "silence is not a miss: {report}"
    );
    assert!(!report.contains("re-billed"), "{report}");
}

/// A provider that DOES report caching, and has simply not hit it, must not be
/// accused of being silent.
///
/// This is the case the `moved_cache_traffic` / `provider_reports_cache` split
/// exists for, and it is the common one: a healthy cache on a session too short
/// to have hit it. The sibling test above covers the genuinely silent provider;
/// without this one, the fix could pass by never accusing anyone of anything.
#[tokio::test]
async fn a_reporting_provider_with_no_hits_yet_is_not_called_silent() {
    let script: Script = Arc::new(Mutex::new(vec![
        (1_000, 0, 0),
        (1_000, 0, 0),
        (1_000, 0, 0),
    ]));
    // The flag stays FALSE: this provider answers, it just has not been hit.
    let agent = agent_with(script, Arc::new(Mutex::new(Vec::new())));
    for i in 0..3 {
        agent.run(user(&format!("turn {i}"))).await.expect("run");
    }
    let report = agent.cache_report().join("\n");
    assert!(
        !report.contains("does not report cache fields"),
        "a reporting provider is not silent: {report}"
    );
    assert!(
        report.contains("no call has hit the cache yet"),
        "and the report names the true reason: {report}"
    );
}
