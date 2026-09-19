//! Live provider smoke test — the one check that cannot run offline.
//!
//! Every other test in this workspace runs against the faux provider, which is
//! why the suite is free and deterministic — and also why a broken *wire* can
//! survive a green build: the adapters are exercised against recorded payloads,
//! never against a real endpoint. This file closes that gap for the common path
//! (build a request, send it, fold the stream, read the answer) on whichever
//! provider the machine actually holds a credential for.
//!
//! Run it deliberately:
//!
//! ```sh
//! cargo test -p epsilon-llm --features real --test live_smoke -- --ignored --nocapture
//! ```
//!
//! It is `#[ignore]`d so no ordinary `cargo test` can spend money or need
//! network. With no credential it prints which providers it looked for and
//! passes, so CI can run it without secrets.
//!
//! Cost: one request, a handful of output tokens.

#![cfg(feature = "real")]

use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::Message;
use epsilon_contracts::stream::StreamEvent;
use epsilon_llm::catalog::{ModelInfo, curated, lookup_model, resolve_api_key};
use epsilon_llm::{DirectProvider, LlmContext, ModelRef, StreamFn};

/// Candidate models in preference order: cheap and widely available. The first
/// one whose provider has a credential wins.
const CANDIDATES: &[&str] = &[
    "deepseek-flash",
    "deepseek-v4-flash",
    "claude-haiku-4-5",
    "gpt-4.1-mini",
    "gemini-2.5-flash",
    "kimi-k2.6",
    "glm-4.5",
];

/// The model this machine can actually call, or `None` when nothing is configured.
fn resolve_candidate() -> Option<&'static ModelInfo> {
    for id in CANDIDATES {
        match lookup_model(id) {
            Some(info) if resolve_api_key(info.provider).is_some() => return Some(info),
            _ => continue,
        }
    }
    curated().iter().find(|m| {
        m.supports_tools && !m.env_key.is_empty() && resolve_api_key(m.provider).is_some()
    })
}

fn llm_context(prompt: &str) -> LlmContext {
    LlmContext {
        system_prompt: "You are a smoke test. Answer with exactly one word.".into(),
        messages: vec![Message::User(epsilon_contracts::message::UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            content: vec![UserContent::Text {
                text: prompt.to_string(),
            }],
        })],
        tools: vec![],
        thinking_level: epsilon_contracts::ThinkingLevel::Off,
    }
}

#[tokio::test]
#[ignore = "hits a real provider and spends tokens; run with --ignored when a key is configured"]
async fn a_real_provider_answers_end_to_end() {
    let Some(info) = resolve_candidate() else {
        // Not a failure: an unauthenticated machine is a legitimate state.
        eprintln!(
            "live smoke: skipped — no credential found for any catalog provider. \
             Set one (e.g. DEEPSEEK_API_KEY) or run `epsilon auth login <provider>`."
        );
        return;
    };
    eprintln!(
        "live smoke: provider={} model={} key={}",
        info.provider, info.id, info.env_key
    );

    let provider = DirectProvider::new();
    let model = ModelRef::new(info.id);
    let mut stream = provider
        .stream(&model, llm_context("Reply with the single word: pong"))
        .await;

    let mut streamed = String::new();
    let done = loop {
        match stream.next().await {
            Some(StreamEvent::TextDelta { delta, .. }) => streamed.push_str(&delta),
            Some(StreamEvent::Done { message }) => break message,
            Some(_) => continue,
            None => panic!("stream ended without a terminal Done event"),
        }
    };

    // The never-fail contract: a failure arrives as a stop reason, never a panic.
    assert!(
        !done.stop_reason.is_failure(),
        "provider {} returned {:?}: {}",
        info.provider,
        done.stop_reason,
        done.error_message.unwrap_or_default()
    );
    let text = done.text();
    eprintln!("live smoke: reply={text:?} usage={:?}", done.usage);
    assert!(
        !text.trim().is_empty() || !streamed.trim().is_empty(),
        "a successful call must deliver some text"
    );
}
