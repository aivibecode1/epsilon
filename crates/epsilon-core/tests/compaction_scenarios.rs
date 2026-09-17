//! M7 integration scenarios: pressure-triggered compaction with the session mirror
//! attached, and context-overflow recovery (ADR-007 triggers #1 and #2).

use std::sync::{Arc, Mutex};

use epsilon_contracts::agent_event::AgentEvent;
use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_core::testing::{EchoTool, SlowProvider};
use epsilon_core::{AutoAllow, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep, PanicGuard};
use epsilon_session::SessionStore;

fn user(text: &str) -> Message {
    Message::User(UserMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: 0,
        content: vec![UserContent::Text {
            text: text.to_string(),
        }],
    })
}

fn big_echo_call(id: &str, chars: usize) -> FauxStep {
    // Echoed text becomes a ~chars/4-token tool result; the tool-call ARGUMENTS carry the
    // same payload, so each such turn costs roughly 2x chars/4 estimated tokens.
    FauxStep::call(id, "echo", serde_json::json!({ "text": "x".repeat(chars) }))
}

fn setup(
    steps: Vec<FauxStep>,
    limits: LoopLimits,
) -> (
    Arc<epsilon_core::Agent>,
    Arc<Mutex<SessionStore>>,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::open(dir.path().join("session.jsonl")).unwrap();

    let provider = SlowProvider::new(FauxProvider::new(steps), 20);
    // Dedicated summarizer script: aux calls are isolated from the main loop's provider
    // (production semantics AND deterministic faux scripts).
    let summarizer = SlowProvider::new(
        FauxProvider::new(vec![FauxStep::say("## Objective\nsummarized state")]),
        20,
    );

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool::new()));

    let agent = epsilon_core::Agent::new(
        limits,
        Arc::new(PanicGuard(provider)),
        Arc::new(registry),
        Arc::new(AutoAllow),
        dir.path().to_path_buf(),
    );
    agent.set_summarizer(Arc::new(PanicGuard(summarizer)));
    agent.attach_session_store(Arc::new(Mutex::new(store)));
    let handle = agent.store_handle_for_tests();
    (agent, handle, dir)
}

fn drain_compactions(rx: &mut tokio::sync::broadcast::Receiver<Arc<AgentEvent>>) -> (usize, usize) {
    let mut started = 0;
    let mut finished = 0;
    loop {
        match rx.try_recv() {
            Ok(ev) => match *ev {
                AgentEvent::CompactionStarted => started += 1,
                AgentEvent::CompactionFinished => finished += 1,
                _ => {}
            },
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
    (started, finished)
}

fn has_summary_head(agent: &epsilon_core::Agent) -> bool {
    matches!(agent.history().first(),
        Some(Message::User(u)) if u.content.iter().any(|c| matches!(
            c, UserContent::Text { text } if text.contains("<compaction_summary>"))))
}

#[tokio::test]
async fn pressure_triggers_compaction_and_persists_entry() {
    // Each big-echo turn costs ~2000 estimated tokens (args + result), so the 1500
    // threshold fires after the FIRST turn and may legitimately re-fire after refill.
    let steps = vec![
        big_echo_call("tc_1", 4000),
        big_echo_call("tc_2", 4000),
        epsilon_llm::FauxStep::say("final answer"),
    ];
    let limits = LoopLimits {
        compaction_threshold_tokens: 1500,
        compaction_tail_tokens: 300,
        max_turns: 8,
        ..Default::default()
    };
    let (agent, store, _dir) = setup(steps, limits);
    let mut rx = agent.subscribe();

    let outcome = agent.run(user("long task")).await.unwrap();
    assert_eq!(
        outcome.reason,
        epsilon_contracts::agent_event::TurnEndReason::Completed
    );

    let (started, finished) = drain_compactions(&mut rx);
    assert!(
        started >= 1 && finished >= 1 && started == finished,
        "s={started} f={finished}"
    );

    assert!(
        has_summary_head(&agent),
        "history must begin with the summary"
    );
    assert!(
        agent.history().len() <= 4,
        "history len: {}",
        agent.history().len()
    );

    // Durable mirror holds the persisted entry; model view shrank vs the raw log.
    let st = store.lock().unwrap();
    let entries = st.read_all().unwrap();
    assert!(entries.iter().any(|e| matches!(
        e.kind,
        epsilon_contracts::session_entry::EntryKind::Compaction { ref summary, .. }
            if summary.contains("Objective")
    )));
    let view_len = st.model_view().unwrap().len();
    let log_len = entries.len();
    assert!(
        view_len < log_len,
        "shadowing must shrink the view: {view_len} vs {log_len}"
    );
}

#[tokio::test]
async fn context_overflow_compacts_once_and_recovers() {
    let steps = vec![
        big_echo_call("tc_1", 500),
        epsilon_llm::FauxStep::Error {
            message: "prompt is too long: maximum context window exceeded".into(),
        },
        epsilon_llm::FauxStep::say("recovered"),
    ];
    let (agent, store, _dir) = setup(
        steps,
        LoopLimits {
            compaction_threshold_tokens: 999_999,
            ..Default::default()
        },
    );
    let mut rx = agent.subscribe();

    let outcome = agent.run(user("go")).await.unwrap();

    // The run RECOVERED instead of failing with Error.
    assert_eq!(
        outcome.reason,
        epsilon_contracts::agent_event::TurnEndReason::Completed
    );

    let (started, finished) = drain_compactions(&mut rx);
    assert_eq!(
        (started, finished),
        (1, 1),
        "bounded retry: exactly one recovery pass"
    );

    assert!(
        store
            .lock()
            .unwrap()
            .read_all()
            .unwrap()
            .iter()
            .any(|e| matches!(
                e.kind,
                epsilon_contracts::session_entry::EntryKind::Compaction { .. }
            ))
    );
}

#[test]
fn for_model_resolves_catalog_windows() {
    // Known ids seed a window; unknown ids (custom providers, ad-hoc test
    // models) keep `None` so the fixed threshold keeps working.
    let known = LoopLimits::for_model("claude-fable-5");
    assert_eq!(known.context_window_tokens, Some(1_000_000));
    assert_eq!(known.compaction_reserve_tokens, 16_384);
    assert!(known.compaction_enabled);

    let custom = LoopLimits::for_model("lmstudio/something-made-up");
    assert_eq!(custom.context_window_tokens, None);
}

#[tokio::test]
async fn disabled_compaction_never_fires() {
    // Pi's `enabled` gate: a threshold that WOULD fire stays quiet.
    let steps = vec![
        big_echo_call("tc_1", 4000),
        epsilon_llm::FauxStep::say("final answer"),
    ];
    let limits = LoopLimits {
        compaction_threshold_tokens: 1, // fires on anything when enabled
        compaction_enabled: false,
        max_turns: 8,
        ..Default::default()
    };
    let (agent, _store, _dir) = setup(steps, limits);
    let mut rx = agent.subscribe();

    let outcome = agent.run(user("long task")).await.unwrap();
    assert_eq!(
        outcome.reason,
        epsilon_contracts::agent_event::TurnEndReason::Completed
    );
    let (started, finished) = drain_compactions(&mut rx);
    assert_eq!((started, finished), (0, 0));
    assert!(!has_summary_head(&agent));
}

#[tokio::test]
async fn known_window_overrides_fixed_threshold() {
    // Pi window-relative trigger: a huge fixed threshold must NOT suppress
    // compaction once the live budget (window - reserve) is exceeded.
    let steps = vec![
        big_echo_call("tc_1", 4000),
        big_echo_call("tc_2", 4000),
        epsilon_llm::FauxStep::say("final answer"),
    ];
    let limits = LoopLimits {
        compaction_threshold_tokens: 999_999, // would never fire on its own
        compaction_tail_tokens: 300,
        context_window_tokens: Some(2_000), // budget 2000 - 0 reserve
        compaction_reserve_tokens: 0,
        max_turns: 8,
        ..Default::default()
    };
    let (agent, _store, _dir) = setup(steps, limits);
    let mut rx = agent.subscribe();

    let outcome = agent.run(user("long task")).await.unwrap();
    assert_eq!(
        outcome.reason,
        epsilon_contracts::agent_event::TurnEndReason::Completed
    );
    let (started, finished) = drain_compactions(&mut rx);
    assert!(
        started >= 1 && started == finished,
        "window budget must fire: s={started} f={finished}"
    );
}

#[tokio::test]
async fn model_hot_swap_refreshes_live_window() {
    // `set_model_id` tracks hot-swaps: moving to a known id installs its
    // window (threshold alone would have fired), moving to an unknown id
    // clears back to the fixed threshold (no stale window survives).
    let steps = vec![
        big_echo_call("tc_1", 4000),
        epsilon_llm::FauxStep::say("final answer"),
    ];
    let limits = LoopLimits {
        compaction_threshold_tokens: 1, // fires on anything without a window
        compaction_tail_tokens: 300,
        max_turns: 8,
        ..Default::default()
    };
    let (agent, _store, _dir) = setup(steps, limits);

    // Known 1M-token model: live budget dwarfs the script, no compaction.
    agent.set_model_id("claude-fable-5");
    let mut rx = agent.subscribe();
    let outcome = agent.run(user("long task")).await.unwrap();
    assert_eq!(
        outcome.reason,
        epsilon_contracts::agent_event::TurnEndReason::Completed
    );
    assert_eq!(drain_compactions(&mut rx), (0, 0));

    // Unknown id (custom provider): override clears, fixed threshold fires.
    // A stale 1M window surviving the swap would suppress this compaction.
    let (agent2, _store2, _dir2) = setup(
        vec![
            big_echo_call("tc_1", 4000),
            epsilon_llm::FauxStep::say("final answer"),
        ],
        LoopLimits {
            compaction_threshold_tokens: 1,
            compaction_tail_tokens: 300,
            context_window_tokens: Some(1_000_000), // stale if not cleared
            max_turns: 8,
            ..Default::default()
        },
    );
    agent2.set_model_id("lmstudio/something-made-up");
    let mut rx2 = agent2.subscribe();
    let outcome2 = agent2.run(user("long task")).await.unwrap();
    assert_eq!(
        outcome2.reason,
        epsilon_contracts::agent_event::TurnEndReason::Completed
    );
    let (s2, f2) = drain_compactions(&mut rx2);
    assert!(
        s2 >= 1 && s2 == f2,
        "cleared window must fire: s={s2} f={f2}"
    );
}
