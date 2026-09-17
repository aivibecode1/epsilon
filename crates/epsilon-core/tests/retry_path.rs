//! M60: the retry policy engaged through the whole loop.
//!
//! The policy (classification, backoff, the `Retrying` executor) existed and was
//! unit-tested, but no production path ever wrapped a stream with it — a
//! transient provider failure failed the turn outright. These scenarios run the
//! real loop over a provider that fails first and answers later.

use std::sync::{Arc, Mutex};

use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_core::{AutoAllow, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep, PanicGuard, RetryNotice, Retrying};

/// A provider that fails `failures` times with a retryable 503, then answers.
/// Faux steps are consumed in order, so the agent sees the failure stream ends.
fn flaky(extra: Vec<FauxStep>) -> Arc<dyn epsilon_llm::StreamFn> {
    let mut steps = vec![
        FauxStep::Error {
            message: "HTTP 503 Service Unavailable".into(),
        },
        FauxStep::Error {
            message: "HTTP 503 Service Unavailable".into(),
        },
    ];
    steps.extend(extra);
    Arc::new(PanicGuard(FauxProvider::new(steps)))
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

fn agent_with(stream: Arc<dyn epsilon_llm::StreamFn>) -> Arc<epsilon_core::Agent> {
    epsilon_core::Agent::new(
        LoopLimits::default(),
        stream,
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    )
}

#[tokio::test]
async fn a_transient_provider_failure_no_longer_fails_the_turn() {
    let notices = Arc::new(Mutex::new(Vec::<RetryNotice>::new()));
    let log = Arc::clone(&notices);
    let stream = Arc::new(
        Retrying::new(flaky(vec![FauxStep::Assistant {
            text: "recovered".into(),
        }]))
        .with_max_attempts(epsilon_llm::RETRY_MAX_ATTEMPTS)
        .with_on_retry(move |n| log.lock().unwrap().push(n.clone())),
    ) as Arc<dyn epsilon_llm::StreamFn>;
    let agent = agent_with(stream);

    let run = agent
        .run(user("do a thing"))
        .await
        .expect("the run completes");
    let text = run
        .produced
        .iter()
        .filter_map(|m| match m {
            Message::Assistant(a) => Some(a.text()),
            _ => None,
        })
        .collect::<String>();
    assert!(
        text.contains("recovered"),
        "the third attempt's answer reached the transcript: {text:?}"
    );
    assert_eq!(
        notices.lock().unwrap().len(),
        2,
        "one notice per scheduled wait, so the pause is explainable"
    );
    assert_eq!(
        notices.lock().unwrap()[0].kind,
        epsilon_llm::FailureKind::Server
    );
}

#[tokio::test]
async fn an_auth_failure_is_never_retried_even_inside_the_loop() {
    // Retrying an auth failure leaks attempts and can lock accounts (ADR-010) —
    // the loop must surface it immediately instead of burning backoff sleeps.
    let notices = Arc::new(Mutex::new(Vec::<RetryNotice>::new()));
    let log = Arc::clone(&notices);
    let stream = Arc::new(
        Retrying::new(Arc::new(PanicGuard(FauxProvider::new(vec![
            FauxStep::Error {
                message: "401 Unauthorized".into(),
            },
        ]))))
        .with_on_retry(move |n| log.lock().unwrap().push(n.clone())),
    ) as Arc<dyn epsilon_llm::StreamFn>;
    let agent = agent_with(stream);

    let run = agent
        .run(user("do a thing"))
        .await
        .expect("the run completes");
    assert!(
        notices.lock().unwrap().is_empty(),
        "no wait may be scheduled for a non-retryable failure"
    );
    let failed = run.produced.iter().any(|m| match m {
        Message::Assistant(a) => a
            .error_message
            .as_deref()
            .is_some_and(|e| e.contains("Unauthorized")),
        _ => false,
    });
    assert!(failed, "the failure is reported verbatim");
}

#[test]
fn the_notice_text_names_what_happened_and_what_it_costs() {
    let notice = RetryNotice {
        model: "deepseek-v4-pro".into(),
        kind: epsilon_llm::FailureKind::Network,
        attempt: 1,
        delay: std::time::Duration::from_millis(1500),
        detail: "provider stream timed out: no data for 120s".into(),
    };
    let text = epsilon_core::retry_notice_text(&notice);
    assert!(text.contains("timed out"), "{text}");
    assert!(text.contains("deepseek-v4-pro"), "{text}");
    assert!(text.contains("1.5s"), "{text}");
    assert!(text.contains("attempt 3"), "1-based attempt number: {text}");
    assert!(text.contains("(network)"), "{text}");
}
