//! Queue-delivery conformance against pi 0.84.2 agent.ts constructor defaults.
//! Assert assembled provider contexts rather than only inspecting queue settings.

use std::sync::Arc;

use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_core::{Agent, AutoAllow, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep};

fn user(text: &str) -> Message {
    Message::User(UserMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: 0,
        content: vec![UserContent::Text { text: text.into() }],
    })
}

#[tokio::test]
async fn default_followups_wait_for_separate_stop_boundaries() {
    let provider = FauxProvider::new(vec![
        FauxStep::say("initial complete"),
        FauxStep::say("first complete"),
        FauxStep::say("second complete"),
    ]);
    let requests = provider.recorded_requests_handle();
    let agent = Agent::new(
        LoopLimits::default(),
        Arc::new(provider),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    let initial = user("initial");
    let first = user("first follow-up");
    let second = user("second follow-up");
    agent.follow_up(first.clone());
    agent.follow_up(second.clone());
    agent.run(initial.clone()).await.unwrap();

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3, "one request for each queued follow-up");
    assert_eq!(requests[0].messages, vec![initial.clone()]);
    assert_eq!(requests[1].messages.len(), 3);
    assert_eq!(requests[1].messages[0], initial);
    assert_eq!(requests[1].messages[2], first);
    assert!(!requests[1].messages.contains(&second));
    assert_eq!(requests[2].messages.len(), 5);
    assert_eq!(requests[2].messages[2], first);
    assert_eq!(requests[2].messages[4], second);
    let Message::Assistant(answer) = &requests[2].messages[3] else {
        panic!("the first follow-up must finish before the second is delivered");
    };
    assert_eq!(answer.text(), "first complete");
}

#[tokio::test]
async fn explicit_all_followups_share_one_stop_boundary() {
    let provider = FauxProvider::new(vec![
        FauxStep::say("initial complete"),
        FauxStep::say("both complete"),
    ]);
    let requests = provider.recorded_requests_handle();
    let agent = Agent::new(
        LoopLimits {
            followup_mode: epsilon_core::QueueMode::All,
            ..LoopLimits::default()
        },
        Arc::new(provider),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    let initial = user("initial");
    let first = user("first follow-up");
    let second = user("second follow-up");
    agent.follow_up(first.clone());
    agent.follow_up(second.clone());
    agent.run(initial.clone()).await.unwrap();

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].messages, vec![initial.clone()]);
    assert_eq!(requests[1].messages.len(), 4);
    assert_eq!(requests[1].messages[0], initial);
    assert_eq!(requests[1].messages[2], first);
    assert_eq!(requests[1].messages[3], second);
    let Message::Assistant(answer) = &requests[1].messages[1] else {
        panic!("initial response must precede the batched follow-ups");
    };
    assert_eq!(answer.text(), "initial complete");
}
