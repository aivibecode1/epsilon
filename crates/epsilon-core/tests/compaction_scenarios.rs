//! M7 integration scenarios: pressure-triggered compaction with the session mirror
//! attached, and context-overflow recovery (ADR-007 triggers #1 and #2).

use std::sync::{Arc, Mutex};

use epsilon_contracts::agent_event::AgentEvent;
use epsilon_contracts::content::{ContentBlock, UserContent};
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_contracts::session_entry::EntryKind;
use epsilon_core::testing::{EchoTool, SlowProvider};
use epsilon_core::{AutoAllow, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep, PanicGuard};
use epsilon_session::{ModelViewEntry, SessionStore};

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
    setup_with_summarizer(
        steps,
        limits,
        Arc::new(PanicGuard(SlowProvider::new(
            FauxProvider::new(vec![FauxStep::say("## Objective\nsummarized state")]),
            20,
        ))),
    )
}

fn setup_with_summarizer(
    steps: Vec<FauxStep>,
    limits: LoopLimits,
    summarizer: Arc<dyn epsilon_llm::StreamFn>,
) -> (
    Arc<epsilon_core::Agent>,
    Arc<Mutex<SessionStore>>,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::open(dir.path().join("session.jsonl")).unwrap();

    let provider = SlowProvider::new(FauxProvider::new(steps), 20);
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool::new()));

    let agent = epsilon_core::Agent::new(
        limits,
        Arc::new(PanicGuard(provider)),
        Arc::new(registry),
        Arc::new(AutoAllow),
        dir.path().to_path_buf(),
    );
    // Dedicated summarizer script: aux calls are isolated from the main loop's provider
    // (production semantics AND deterministic faux scripts).
    agent.set_summarizer(summarizer);
    agent.attach_session_store(Arc::new(Mutex::new(store)));
    let handle = agent.store_handle_for_tests();
    (agent, handle, dir)
}

fn drain_compactions(rx: &mut tokio::sync::broadcast::Receiver<Arc<AgentEvent>>) -> (usize, usize) {
    let (started, finished, _) = drain_all(rx);
    (started, finished)
}

/// One pass over the event log: compaction bookends plus notices, so a test can
/// assert both what the loop did and what it told the user.
fn drain_all(
    rx: &mut tokio::sync::broadcast::Receiver<Arc<AgentEvent>>,
) -> (usize, usize, Vec<String>) {
    let mut started = 0;
    let mut finished = 0;
    let mut notices = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(ev) => match &*ev {
                AgentEvent::CompactionStarted => started += 1,
                AgentEvent::CompactionFinished => finished += 1,
                AgentEvent::Notice { text } => notices.push(text.clone()),
                _ => {}
            },
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
    (started, finished, notices)
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

/// CORE-04: a failed provider turn is not a summary (pi rejects
/// `stopReason === "error"`). The attempt is reported, nothing is committed, and
/// the run continues on the uncompacted transcript instead of replacing it with
/// failure prose or a placeholder.
#[tokio::test]
async fn failed_summarization_does_not_commit_and_is_reported_once() {
    let steps = vec![
        big_echo_call("tc_1", 4000),
        big_echo_call("tc_2", 4000),
        FauxStep::say("final answer"),
    ];
    let limits = LoopLimits {
        compaction_threshold_tokens: 1500,
        compaction_tail_tokens: 300,
        max_turns: 8,
        ..Default::default()
    };
    let (agent, store, _dir) = setup_with_summarizer(
        steps,
        limits,
        Arc::new(PanicGuard(SlowProvider::new(
            FauxProvider::new(vec![FauxStep::Error {
                message: "429 rate limited".into(),
            }]),
            5,
        ))),
    );
    let mut rx = agent.subscribe();

    let outcome = agent.run(user("long task")).await.unwrap();

    // The run continues: a summarizer failure is a compaction problem, not a run
    // failure, and the transcript it could not summarize stays intact.
    assert_eq!(
        outcome.reason,
        epsilon_contracts::agent_event::TurnEndReason::Completed
    );
    assert!(
        !has_summary_head(&agent),
        "no summary may replace the transcript"
    );
    assert!(
        agent
            .history()
            .iter()
            .any(|m| matches!(m, Message::ToolResult(_))),
        "the real tool results stay in history"
    );

    let (started, finished, notices) = drain_all(&mut rx);
    assert_eq!(
        (started, finished),
        (1, 0),
        "one attempt, nothing committed"
    );
    assert!(
        notices.iter().any(|n| n.contains("compaction skipped")),
        "the failure is reported, not silent: {notices:?}"
    );

    let st = store.lock().unwrap();
    assert!(
        !st.read_all().unwrap().iter().any(|e| matches!(
            e.kind,
            epsilon_contracts::session_entry::EntryKind::Compaction { .. }
        )),
        "no compaction entry may be durable"
    );
}

/// CORE-04, second acceptance path: an empty answer is refused rather than
/// replaced with the old template placeholder (an Epsilon reliability rule; pi's
/// inspected helper has no nonempty check).
#[tokio::test]
async fn empty_summary_text_is_refused_rather_than_padded() {
    let steps = vec![big_echo_call("tc_1", 4000), FauxStep::say("final answer")];
    let limits = LoopLimits {
        compaction_threshold_tokens: 1500,
        compaction_tail_tokens: 300,
        max_turns: 8,
        ..Default::default()
    };
    let (agent, store, _dir) = setup_with_summarizer(
        steps,
        limits,
        Arc::new(PanicGuard(SlowProvider::new(
            FauxProvider::new(vec![FauxStep::say("   \n  ")]),
            5,
        ))),
    );
    let mut rx = agent.subscribe();

    let outcome = agent.run(user("long task")).await.unwrap();
    assert_eq!(
        outcome.reason,
        epsilon_contracts::agent_event::TurnEndReason::Completed
    );

    let (started, finished, notices) = drain_all(&mut rx);
    assert_eq!((started, finished), (1, 0));
    assert!(
        notices.iter().any(|n| n.contains("no text")),
        "the empty answer is named: {notices:?}"
    );
    assert!(!has_summary_head(&agent));
    let st = store.lock().unwrap();
    assert!(!st.read_all().unwrap().iter().any(|e| matches!(
        e.kind,
        epsilon_contracts::session_entry::EntryKind::Compaction { .. }
    )));
}

/// CORE-04 (cancellation): Esc during the summarization call reaches that call,
/// and a cancelled pass neither rewrites the transcript nor appends an entry.
#[tokio::test]
async fn abort_during_summarization_ends_the_run_aborted_and_commits_nothing() {
    let steps = vec![big_echo_call("tc_1", 4000), FauxStep::say("never reached")];
    let limits = LoopLimits {
        compaction_threshold_tokens: 1500,
        compaction_tail_tokens: 300,
        max_turns: 8,
        ..Default::default()
    };
    // Summon the summarizer's stalled connect: the pass is inside its select when
    // the abort arrives.
    let (agent, store, _dir) = setup_with_summarizer(
        steps,
        limits,
        Arc::new(PanicGuard(SlowProvider::new(
            FauxProvider::new(vec![FauxStep::say("## Objective\ntoo late")]),
            5_000,
        ))),
    );
    let mut rx = agent.subscribe();

    let handle = tokio::spawn({
        let agent = Arc::clone(&agent);
        async move { agent.run(user("long task")).await }
    });
    // Long enough for turn one to finish and the summarization call to start.
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    assert!(agent.abort_current_run());

    let outcome = handle.await.unwrap().unwrap();
    assert_eq!(
        outcome.reason,
        epsilon_contracts::agent_event::TurnEndReason::Aborted
    );
    assert!(
        !has_summary_head(&agent),
        "a cancelled pass must not compact"
    );
    let (started, finished, _) = drain_all(&mut rx);
    assert_eq!(
        (started, finished),
        (1, 0),
        "the attempt started, nothing was committed"
    );
    let st = store.lock().unwrap();
    assert!(!st.read_all().unwrap().iter().any(|e| matches!(
        e.kind,
        epsilon_contracts::session_entry::EntryKind::Compaction { .. }
    )));
}

// ---- SESSION-01a / F04 alternate-branch: the provider view follows the ACTIVE branch ----------

/// Provider-bound rendering of a transcript: one string per message, as a request carries it.
fn rendered(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .map(|m| match m {
            Message::User(u) => u
                .content
                .iter()
                .filter_map(|c| match c {
                    UserContent::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Message::Assistant(a) => a.text(),
            Message::ToolResult(t) => t
                .content
                .iter()
                .filter_map(|c| match c {
                    ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Message::Custom(_) => String::new(),
        })
        .collect()
}

/// What a request built from this view would carry.
fn view_texts(view: &[ModelViewEntry]) -> Vec<String> {
    rendered(
        &view
            .iter()
            .cloned()
            .map(|v| v.into_message())
            .collect::<Vec<_>>(),
    )
}

/// An agent over an EXISTING session file with a request log attached: the reopened leg
/// needs the provider-bound view, which only a real request can show.
fn agent_over_session(
    path: &std::path::Path,
    workspace: &std::path::Path,
) -> (
    Arc<epsilon_core::Agent>,
    Arc<Mutex<Vec<epsilon_llm::LlmContext>>>,
) {
    let store = SessionStore::open_in(path, workspace).unwrap();
    let provider = FauxProvider::new(vec![FauxStep::say("branch B answer")]);
    let requests = provider.recorded_requests_handle();
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool::new()));
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(PanicGuard(SlowProvider::new(provider, 20))),
        Arc::new(registry),
        Arc::new(AutoAllow),
        workspace.to_path_buf(),
    );
    agent.attach_session_store(Arc::new(Mutex::new(store)));
    (agent, requests)
}

/// The ledger's missing F04 alternate-branch variant (`audit/02-defect-ledger.md:97-98`):
/// a compaction on one branch must not leak into its sibling's provider view, a branch
/// summary attached at the navigation target appears exactly once on the NEW branch, and
/// reopening the file reproduces that same view — on a real request, built by the loop
/// from the reopened store.
#[tokio::test]
async fn a_sibling_branch_sees_its_own_summary_and_not_the_other_branchs_compaction() {
    // Branch A runs and compacts on its own lane (same pressure config as above).
    let steps = vec![
        big_echo_call("tc_1", 4000),
        FauxStep::say("branch A answer"),
    ];
    let limits = LoopLimits {
        compaction_threshold_tokens: 1500,
        compaction_tail_tokens: 300,
        max_turns: 8,
        ..Default::default()
    };
    let (agent, store, dir) = setup(steps, limits);

    let outcome = agent.run(user("shared start")).await.unwrap();
    assert_eq!(
        outcome.reason,
        epsilon_contracts::agent_event::TurnEndReason::Completed
    );

    let path = store.lock().unwrap().path().to_path_buf();
    let root_id = {
        let st = store.lock().unwrap();
        let all = st.read_all().unwrap();
        assert!(
            all.iter()
                .any(|e| matches!(e.kind, EntryKind::Compaction { .. })),
            "branch A must have compacted for this scenario"
        );
        assert!(
            view_texts(&st.model_view().unwrap())
                .iter()
                .any(|t| t.contains("<compaction_summary>")),
            "branch A's view carries the compaction"
        );
        all.into_iter()
            .find(|e| {
                matches!(
                    &e.kind,
                    EntryKind::Message {
                        message: Message::User(_),
                        ..
                    }
                )
            })
            .expect("the prompt entry")
            .id
    };

    // Branch B: summarize the abandoned lane, then continue on the new one.
    let tip_id = {
        let mut st = store.lock().unwrap();
        let summary = st
            .branch_with_summary(Some(&root_id), "explored branch A", None, None)
            .unwrap();
        assert_eq!(
            st.leaf_id().as_deref(),
            Some(summary.id.as_str()),
            "the summary is the new leaf"
        );
        st.append(EntryKind::Message {
            message: user("branch B work"),
            api_request_sha256: None,
        })
        .unwrap()
        .id
    };
    let intended = vec![
        "shared start".to_string(),
        "<branch_summary>\nexplored branch A\n</branch_summary>".to_string(),
        "branch B work".to_string(),
    ];
    {
        let st = store.lock().unwrap();
        assert_eq!(
            view_texts(&st.model_view().unwrap()),
            intended,
            "the live view follows the ACTIVE branch only"
        );
    }

    // Reopen the file (one writer per session: both handles go first).
    drop(store);
    drop(agent);
    let mut reopened = SessionStore::open(&path).unwrap();
    assert_eq!(
        view_texts(&reopened.model_view().unwrap()),
        intended,
        "reopening must reproduce the branch view"
    );
    assert_eq!(
        reopened.leaf_id().as_deref(),
        Some(tip_id.as_str()),
        "the branch tip is the file's last line"
    );
    // Either sibling can be selected: navigating to the abandoned branch shows ITS
    // view — its compaction, and no branch summary — and back again restores B.
    let compaction_id = reopened
        .read_all()
        .unwrap()
        .into_iter()
        .find(|e| matches!(e.kind, EntryKind::Compaction { .. }))
        .expect("branch A's compaction entry")
        .id;
    reopened.navigate_to(&compaction_id).unwrap();
    let a_view = view_texts(&reopened.model_view().unwrap());
    assert!(
        a_view.iter().any(|t| t.contains("<compaction_summary>")),
        "the abandoned branch's own view still carries its compaction: {a_view:?}"
    );
    assert!(
        !a_view.iter().any(|t| t.starts_with("<branch_summary>")),
        "the sibling's branch summary is not on this path: {a_view:?}"
    );
    reopened.navigate_to(&tip_id).unwrap();
    assert_eq!(
        view_texts(&reopened.model_view().unwrap()),
        intended,
        "navigating back restores the sibling's view"
    );
    drop(reopened);

    // The provider view: a real request, built by the loop from the reopened store.
    let (agent2, requests) = agent_over_session(&path, dir.path());
    let outcome = agent2.run(user("continue on B")).await.unwrap();
    assert_eq!(
        outcome.reason,
        epsilon_contracts::agent_event::TurnEndReason::Completed
    );

    let first = requests
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("the first request");
    let mut expected = intended.clone();
    expected.push("continue on B".to_string());
    let seen = rendered(&first.messages);
    assert_eq!(
        seen, expected,
        "the provider view equals the intended projection"
    );
    assert_eq!(
        seen.iter()
            .filter(|t| t.starts_with("<branch_summary>"))
            .count(),
        1,
        "the branch summary appears exactly once"
    );
    assert!(
        !seen.iter().any(|t| t.contains("<compaction_summary>")),
        "branch A's compaction must not leak into the sibling: {seen:?}"
    );
}

// -------------------------------------------------------------------------------------
// M145 / F22 / F23 — a compaction is a real provider call, so it is a real bill.
// -------------------------------------------------------------------------------------

#[tokio::test]
async fn a_compaction_records_its_bill_on_its_own_entry() {
    // This is the test that found F23. It drives a real compaction through a real agent
    // with a store attached, then asks the store what it recorded. M144 appended a SEPARATE
    // accounting entry, which made the bill the file's last entry — and
    // `Agent::pop_last_message` pops the last entry to discard a failed turn, so the pop ate
    // the bill and left an unbilled compaction behind:
    //   kinds=["message", "message", "message", "compaction", "message", "message",
    //          "compaction", "message"]
    // The bill now lives on the compaction entry, so there is nothing to separate.
    //
    // Same shape as `pressure_triggers_compaction_and_persists_entry`, which is known to
    // fire: 4000-char echo turns (~2000 estimated tokens each) against a 1500 threshold,
    // plus a trailing text step so the loop ends cleanly. Smaller steps made the trigger
    // miss, and a test that fails before its subject is not evidence.
    let (agent, handle, _dir) = setup(
        vec![
            big_echo_call("tc_1", 4000),
            big_echo_call("tc_2", 4000),
            epsilon_llm::FauxStep::say("final answer"),
        ],
        LoopLimits {
            compaction_threshold_tokens: 1500,
            compaction_tail_tokens: 300,
            max_turns: 8,
            ..Default::default()
        },
    );

    let mut rx = agent.subscribe();
    let _ = agent.run(user("work hard enough to compact")).await;
    // The same drain the neighbouring scenarios use, so a compaction bookend that has not
    // been consumed cannot be mistaken for one that has not happened.
    let (started, _) = drain_compactions(&mut rx);
    assert!(
        started > 0,
        "the pressure trigger never fired, so nothing was billed"
    );

    // The faux provider stamps usage in `blank()` (faux.rs:167) with no seam to change it.
    // It is NOT a constant: a summarizer's prompt carries the whole shadowed history, so
    // the real stamp here is 200 prompt / 100 completion, not `blank()`'s 100/50. The first
    // draft of this test asserted `blank()`'s constants and failed twice — `left: 200` then
    // `left: 100` — with the bill recorded correctly and my expectations the thing that was
    // wrong both times. Asserting numbers the provider cannot produce would have been a test
    // of nothing; a *fixture wish* is not a measurement.
    let accounts = handle.lock().unwrap().usage_accounts().unwrap();
    assert!(
        !accounts.is_empty(),
        "a billed compaction recorded nothing, so its cost is invisible"
    );
    let (model_id, by_reason) = accounts.iter().next().expect("at least one row");
    let compaction = by_reason
        .get("compaction")
        .expect("the reader says what kind of call this was");
    assert_eq!(
        compaction.input, 200,
        "the summarizer's own prompt tokens, not a guess at them"
    );
    assert_eq!(compaction.output, 100);
    assert!(
        !model_id.is_empty(),
        "a bill with no charging model cannot be priced, and must be skipped rather than \
         stored unattributable"
    );

    // And the bill is on the compaction entry itself — one entry, so a positional pop takes
    // the summary and its cost together (F23).
    let st = handle.lock().unwrap();
    let kinds: Vec<String> = st
        .read_all()
        .unwrap()
        .into_iter()
        .map(|e| e.kind.wire_tag().unwrap_or_else(|| "?".into()))
        .collect();
    assert!(
        !kinds.iter().any(|k| k == "usage_account"),
        "a separate accounting entry is exactly the shape F23 rejects: {kinds:?}"
    );
    // The reader reports a SUM and an entry holds ONE bill, so the invariant is not
    // equality between them — it is that the reader's total equals the sum of the entries.
    // The pressure threshold can fire twice here (100 + 100 = 200), and asserting one entry's
    // 100 against the reader's 200 is how this test failed twice while the code was right.
    let entries: Vec<(Option<epsilon_contracts::message::Usage>, Option<String>)> = st
        .read_all()
        .unwrap()
        .into_iter()
        .filter_map(|e| match e.kind {
            EntryKind::Compaction {
                usage, model_id, ..
            } => Some((usage, model_id)),
            _ => None,
        })
        .collect();
    let summed_input: u64 = entries
        .iter()
        .filter_map(|(u, _)| u.as_ref())
        .map(|u| u.input)
        .sum();
    assert_eq!(
        summed_input, compaction.input,
        "the reader's total must equal the sum of the bills on the entries, not one of them"
    );
    assert!(
        entries.iter().any(|(u, m)| u.is_some() && m.is_some()),
        "some compaction entry must carry both a bill and the model that was charged"
    );
}
