//! Delegated-task scenarios (M53): background children, resumption, status
//! reports, and the completion notice that reaches the parent model.
//!
//! Offline and deterministic: the faux provider scripts the child turns, and
//! every assertion is on the registry (`task_snapshot`) or on the parent's
//! produced messages — never on wall-clock timing.

use std::sync::Arc;

use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_contracts::tool::{TaskHost, TaskOutcome, TaskRequest, TaskState};
use epsilon_core::{AutoAllow, LoopLimits, ToolRegistry};
use epsilon_llm::{FauxProvider, FauxStep, PanicGuard};

fn user_msg(text: &str) -> Message {
    Message::User(UserMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: 0,
        content: vec![UserContent::Text {
            text: text.to_string(),
        }],
    })
}

fn agent_with(steps: Vec<FauxStep>) -> Arc<epsilon_core::Agent> {
    epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(PanicGuard(FauxProvider::new(steps))),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    )
}

/// An agent whose transcript is durable (M58): children of this agent get their
/// own session files next to this one.
fn agent_with_store(
    steps: Vec<FauxStep>,
    session_file: &std::path::Path,
) -> Arc<epsilon_core::Agent> {
    let agent = agent_with(steps);
    let store = epsilon_session::SessionStore::open_in(session_file, &std::env::temp_dir())
        .expect("parent store");
    agent.attach_session_store(Arc::new(std::sync::Mutex::new(store)));
    agent
}

fn req(prompt: &str, description: &str, background: bool, resume: Option<&str>) -> TaskRequest {
    req_with(prompt, description, background, resume, None)
}

/// [`req`] plus an explicit model for the child (M56).
fn req_with(
    prompt: &str,
    description: &str,
    background: bool,
    resume: Option<&str>,
    model: Option<&str>,
) -> TaskRequest {
    TaskRequest {
        prompt: prompt.to_string(),
        description: description.to_string(),
        subagent_type: "default".to_string(),
        background,
        resume: resume.map(str::to_string),
        model: model.map(str::to_string),
    }
}

/// Wait for a background child to leave `Running`, or fail the test.
async fn await_settled(
    agent: &Arc<epsilon_core::Agent>,
    id: &str,
) -> epsilon_contracts::tool::TaskSnapshot {
    for _ in 0..300 {
        let snap = agent.task_snapshot(id).expect("tracked");
        if snap.state != TaskState::Running {
            return snap;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("task {id} never settled");
}

#[tokio::test]
async fn background_task_runs_detached_and_its_notice_reaches_the_parent() {
    // Script order is the call order: the child consumes the first step, the
    // parent's later run consumes the second.
    let agent = agent_with(vec![
        FauxStep::say("child work done"),
        FauxStep::say("parent ack"),
    ]);
    let host = agent.clone() as Arc<dyn TaskHost>;

    let outcome = host
        .run_task(req("do the thing", "bg probe", true, None))
        .await
        .expect("background start must succeed");
    let snapshot = match outcome {
        TaskOutcome::Running(s) => s,
        other => panic!("background start must return Running, got {other:?}"),
    };
    assert_eq!(snapshot.state, TaskState::Running);
    assert_eq!(snapshot.description, "bg probe");
    let id = snapshot.id.clone();
    assert!(id.starts_with("task"), "ids are task-prefixed: {id}");

    // The child finishes on its own and the registry learns the outcome.
    let settled = await_settled(&agent, &id).await;
    assert_eq!(settled.state, TaskState::Completed);
    assert!(
        settled.output.contains("child work done"),
        "{}",
        settled.output
    );
    assert_eq!(settled.turns, 1);

    // The completion notice is queued and arrives as the parent's first
    // pending message on its next run (drained with the steering queue).
    let mut events = agent.subscribe();
    let run = agent.run(user_msg("carry on")).await.unwrap();
    // ...and it must also reach the transcript: a message the model acts on
    // but the user never sees would be a lie by omission.
    let mut notice_in_transcript = false;
    while let Ok(ev) = events.try_recv() {
        if let epsilon_contracts::agent_event::AgentEvent::MessageEnd { message } = &*ev
            && let Message::User(u) = &**message
        {
            notice_in_transcript |= u.content.iter().any(|c| match c {
                UserContent::Text { text } => text.contains("state=\"completed\""),
                _ => false,
            });
        }
    }
    assert!(
        notice_in_transcript,
        "the notice must be emitted as a message event"
    );
    let first = run
        .produced
        .first()
        .expect("the notice must be produced first");
    let text = match first {
        Message::User(u) => u
            .content
            .iter()
            .filter_map(|c| match c {
                UserContent::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
        other => panic!("notice must be a user message, got {other:?}"),
    };
    assert!(text.contains("<task id="), "{text}");
    assert!(text.contains("state=\"completed\""), "{text}");
    assert!(text.contains("child work done"), "{text}");
    assert!(text.contains(&id), "the notice names the task: {text}");
}

#[tokio::test]
async fn resume_continues_the_same_child_and_accumulates_turns() {
    let agent = agent_with(vec![
        FauxStep::say("first answer"),
        FauxStep::say("second answer"),
    ]);
    let host = agent.clone() as Arc<dyn TaskHost>;

    let first = host
        .run_task(req("first question", "dig once", false, None))
        .await
        .expect("foreground run");
    let first_id = match &first {
        TaskOutcome::Completed(res) => res
            .details
            .get("task_id")
            .and_then(|v| v.as_str())
            .expect("id exposed in details")
            .to_string(),
        other => panic!("foreground must complete inline, got {other:?}"),
    };
    assert!(matches!(&first, TaskOutcome::Completed(res) if res.text.contains("first answer")));

    let second = host
        .run_task(req(
            "follow-up question",
            "dig once",
            false,
            Some(&first_id),
        ))
        .await
        .expect("resume must run");
    match second {
        TaskOutcome::Completed(res) => {
            assert!(res.text.contains("second answer"), "{}", res.text);
            assert_eq!(
                res.details["task_id"], first_id,
                "a resume keeps the id — it is the same child"
            );
        }
        other => panic!("resume must complete inline, got {other:?}"),
    }

    // One tracked task, two turns: the registry stayed on the same child.
    let snapshot = agent.task_snapshot(&first_id).expect("still tracked");
    assert_eq!(snapshot.turns, 2, "turns accumulate across resumes");
    assert_eq!(agent.task_snapshots().len(), 1, "no fresh child was built");
}

#[tokio::test]
async fn status_probe_reports_without_running_anything() {
    let agent = agent_with(vec![FauxStep::say("only turn")]);
    let host = agent.clone() as Arc<dyn TaskHost>;
    let outcome = host
        .run_task(req("one question", "probe me", false, None))
        .await
        .unwrap();
    let TaskOutcome::Completed(res) = outcome else {
        panic!("expected completion");
    };
    let id = res.details["task_id"].as_str().unwrap().to_string();

    // A probe with no prompt must not start a run (the script has no steps
    // left — a run would surface a provider error instead of a snapshot).
    let probe = host
        .run_task(req("", "", false, Some(&id)))
        .await
        .expect("status probe must succeed");
    match probe {
        TaskOutcome::Running(snapshot) => {
            assert_eq!(snapshot.state, TaskState::Completed);
            assert!(snapshot.output.contains("only turn"), "{}", snapshot.output);
        }
        other => panic!("probe must return a snapshot, got {other:?}"),
    }
}

#[tokio::test]
async fn unknown_task_id_is_refused_with_the_tracked_ids() {
    let agent = agent_with(vec![FauxStep::say("hello")]);
    let host = agent.clone() as Arc<dyn TaskHost>;
    host.run_task(req("do it", "first", false, None))
        .await
        .expect("first task runs");
    let tracked = agent.task_ids();
    assert_eq!(tracked.len(), 1);

    let failure = host
        .run_task(req("continue", "first", false, Some("task-nope")))
        .await
        .expect_err("unknown id must fail");
    assert!(failure.contains("unknown task id 'task-nope'"), "{failure}");
    assert!(
        failure.contains(&tracked[0]),
        "names what is tracked: {failure}"
    );
}

#[tokio::test]
async fn resuming_a_running_background_task_is_refused_until_it_settles() {
    use epsilon_core::testing::SlowProvider;

    // SlowProvider stalls every stream open: long enough to observe the
    // running state, short enough to keep the test quick.
    let provider = SlowProvider::new(FauxProvider::new(vec![FauxStep::say("slow child")]), 400);
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(provider),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    let host = agent.clone() as Arc<dyn TaskHost>;
    let started = host
        .run_task(req("slow work", "slow one", true, None))
        .await
        .unwrap();
    let TaskOutcome::Running(snapshot) = started else {
        panic!("expected Running");
    };
    let id = snapshot.id.clone();
    assert_eq!(snapshot.state, TaskState::Running);

    let refusal = host
        .run_task(req("more work", "slow one", false, Some(&id)))
        .await
        .expect_err("a running task cannot be resumed");
    assert!(refusal.contains("still running"), "{refusal}");

    let settled = await_settled(&agent, &id).await;
    assert_eq!(settled.state, TaskState::Completed);
    assert!(settled.output.contains("slow child"), "{}", settled.output);
}

#[tokio::test]
async fn aborting_the_parent_stops_running_children() {
    use epsilon_core::testing::SlowProvider;

    // The child stalls in stream creation for 5s; the abort must win quickly
    // (M34's abort-during-stream path) instead of waiting the stall out.
    let provider = SlowProvider::new(FauxProvider::new(vec![FauxStep::say("never seen")]), 5_000);
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(provider),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    let host = agent.clone() as Arc<dyn TaskHost>;
    let started = host
        .run_task(req("slow work", "orphan guard", true, None))
        .await
        .unwrap();
    let TaskOutcome::Running(snapshot) = started else {
        panic!("expected Running");
    };
    assert_eq!(snapshot.state, TaskState::Running);

    // One child is running and the parent has no run in flight, so
    // `abort_current_run` reports false for the parent — while still cascading
    // into the child (the cascade must not depend on a live parent token).
    assert_eq!(
        agent
            .task_snapshots()
            .iter()
            .filter(|s| s.state == TaskState::Running)
            .count(),
        1,
        "exactly the child is running"
    );
    assert!(
        !agent.abort_current_run(),
        "no parent run: the return value describes the parent, not the cascade"
    );

    let settled = await_settled(&agent, &snapshot.id).await;
    assert_eq!(
        settled.state,
        TaskState::Failed,
        "an aborted child is failed"
    );
    assert!(
        settled.output.contains("aborted"),
        "the reason says aborted: {}",
        settled.output
    );
    // Nothing is running any more, so a second sweep asks nobody.
    assert_eq!(agent.abort_delegated_tasks(), 0);
}

#[tokio::test]
async fn child_failures_land_in_the_registry_as_failed() {
    // A provider error makes the child run fail; the notice must say so.
    let agent = agent_with(vec![FauxStep::Error {
        message: "provider exploded".into(),
    }]);
    let host = agent.clone() as Arc<dyn TaskHost>;
    let failure = host
        .run_task(req("will fail", "failing task", false, None))
        .await
        .expect_err("child failure must surface");
    assert!(failure.contains("subagent run failed"), "{failure}");
    assert!(failure.contains("provider exploded"), "{failure}");
    let snapshots = agent.task_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].state, TaskState::Failed);
    assert!(snapshots[0].output.contains("provider exploded"));
}

// ---------------------------------------------------------------------------
// M56: per-delegation model override
// ---------------------------------------------------------------------------

#[tokio::test]
async fn request_model_is_honoured_and_reported() {
    let agent = agent_with(vec![FauxStep::say("scouted")]);
    let host = agent.clone() as Arc<dyn TaskHost>;
    let outcome = host
        .run_task(req_with(
            "look around",
            "scout",
            false,
            None,
            Some("deepseek-v4-pro"),
        ))
        .await
        .expect("delegation");
    match outcome {
        TaskOutcome::Completed(res) => {
            assert_eq!(
                res.details.get("model").and_then(|v| v.as_str()),
                Some("deepseek-v4-pro"),
                "the tool result must name the model that ran"
            );
        }
        other => panic!("expected Completed, got {other:?}"),
    }
    let snap = agent.task_snapshots().pop().expect("tracked");
    assert_eq!(snap.model.as_deref(), Some("deepseek-v4-pro"));
}

#[tokio::test]
async fn unknown_and_partial_models_are_refused_before_any_run() {
    let agent = agent_with(vec![FauxStep::say("should never run")]);
    let host = agent.clone() as Arc<dyn TaskHost>;

    let err = host
        .run_task(req_with(
            "x",
            "bad model",
            false,
            None,
            Some("no-such-model"),
        ))
        .await
        .expect_err("unknown id must fail");
    assert!(err.contains("unknown model"), "{err}");

    // A partial match is offered, never used: a delegated run must not swap
    // models on a guess.
    let err = host
        .run_task(req_with("x", "partial", false, None, Some("deepseek")))
        .await
        .expect_err("partial id must fail");
    assert!(err.contains("only partially matches"), "{err}");

    assert!(
        agent.task_snapshots().is_empty(),
        "a refused delegation must not register a task"
    );
}

#[tokio::test]
async fn agent_def_model_outranks_the_request() {
    // Operator configuration wins: the def's model is the type's contract.
    let agent = agent_with(vec![FauxStep::say("ran")]);
    agent.set_agents(vec![epsilon_core::agents::AgentDef {
        name: "scout".to_string(),
        description: "read-only scout".to_string(),
        tools: None,
        model: Some("deepseek-flash".to_string()),
        system_prompt: String::new(),
        source: epsilon_core::agents::AgentSource::User,
    }]);
    let host = agent.clone() as Arc<dyn TaskHost>;
    let mut request = req_with("look", "scout", false, None, Some("claude-opus-5"));
    request.subagent_type = "scout".to_string();
    let outcome = host.run_task(request).await.expect("delegation");
    match outcome {
        TaskOutcome::Completed(res) => assert_eq!(
            res.details.get("model").and_then(|v| v.as_str()),
            Some("deepseek-flash"),
            "the def's model wins over the call's request"
        ),
        other => panic!("expected Completed, got {other:?}"),
    }
}

#[tokio::test]
async fn the_background_notice_names_the_model() {
    let agent = agent_with(vec![FauxStep::say("child work done")]);
    let host = agent.clone() as Arc<dyn TaskHost>;
    let started = host
        .run_task(req_with(
            "work",
            "cheap scout",
            true,
            None,
            Some("deepseek-flash"),
        ))
        .await
        .expect("started");
    let id = match started {
        TaskOutcome::Running(snap) => snap.id,
        other => panic!("expected Running, got {other:?}"),
    };
    let settled = await_settled(&agent, &id).await;
    assert_eq!(settled.state, TaskState::Completed);
    assert_eq!(settled.model.as_deref(), Some("deepseek-flash"));
    // The notice travels as a user-shaped message and arrives first on the
    // parent's next run (steering drain), carrying the model attribute.
    let run = agent.run(user_msg("carry on")).await.expect("parent run");
    let first = run.produced.first().expect("notice produced first");
    let text = match first {
        Message::User(u) => u
            .content
            .iter()
            .filter_map(|c| match c {
                UserContent::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
        other => panic!("expected the notice as a user message, got {other:?}"),
    };
    assert!(
        text.contains("model=\"deepseek-flash\""),
        "notice must name the child's model: {text}"
    );
}

// ---------------------------------------------------------------------------
// M58: durable child sessions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_child_persists_its_own_session_file_beside_the_parent() {
    let tmp = tempfile::tempdir().unwrap();
    let parent_file = tmp.path().join("tui.jsonl");
    let agent = agent_with_store(vec![FauxStep::say("child leg one")], &parent_file);
    let host = agent.clone() as Arc<dyn TaskHost>;
    let outcome = host
        .run_task(req("look around", "durable scout", false, None))
        .await
        .expect("delegation");
    let id = match outcome {
        TaskOutcome::Completed(res) => res
            .details
            .get("task_id")
            .and_then(|v| v.as_str())
            .expect("task id")
            .to_string(),
        other => panic!("expected Completed, got {other:?}"),
    };

    // The child's file exists, named after the parent + the task id, and its
    // header points back at the parent session.
    let child_file = tmp.path().join(format!("tui-{id}.jsonl"));
    assert!(child_file.exists(), "child session file: {child_file:?}");
    // M61: the live child agent still holds its file's write lease, so read the
    // bytes instead of opening a second store — which is also the honest check
    // ("what a restarted process would parse off disk").
    let child_text = std::fs::read_to_string(&child_file).expect("child file");
    let header: serde_json::Value =
        serde_json::from_str(child_text.lines().next().expect("header line")).expect("header json");
    let parent_text = std::fs::read_to_string(&parent_file).expect("parent file");
    let parent_header: serde_json::Value =
        serde_json::from_str(parent_text.lines().next().expect("parent header")).expect("json");
    assert_eq!(
        header.get("parent_session").and_then(|v| v.as_str()),
        parent_header.get("id").and_then(|v| v.as_str()),
        "the child records the session it was spawned from"
    );
    // The transcript + its meta are on disk: a restarted process can rebuild it.
    let entries: Vec<epsilon_contracts::session_entry::SessionEntry> = child_text
        .lines()
        .skip(1)
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    assert!(
        entries.iter().any(|e| matches!(
            &e.kind,
            epsilon_contracts::session_entry::EntryKind::Custom { custom_type, .. }
                if custom_type == "task_meta"
        )),
        "the child file carries its task metadata"
    );
    assert!(
        entries.iter().any(|e| matches!(
            &e.kind,
            epsilon_contracts::session_entry::EntryKind::Message { message, .. }
                if matches!(message, Message::Assistant(a) if a.text().contains("child leg one"))
        )),
        "the child's own turn is persisted"
    );
}

#[tokio::test]
async fn a_task_id_resumes_after_the_process_that_created_it_is_gone() {
    let tmp = tempfile::tempdir().unwrap();
    let parent_file = tmp.path().join("tui.jsonl");
    let id = {
        let agent = agent_with_store(vec![FauxStep::say("first leg done")], &parent_file);
        let host = agent.clone() as Arc<dyn TaskHost>;
        match host
            .run_task(req("start the work", "two-leg task", false, None))
            .await
            .expect("delegation")
        {
            TaskOutcome::Completed(res) => res
                .details
                .get("task_id")
                .and_then(|v| v.as_str())
                .expect("task id")
                .to_string(),
            other => panic!("expected Completed, got {other:?}"),
        }
    }; // the first agent is dropped: nothing is tracked in memory any more

    // A fresh process: same session directory, empty registry, same task id.
    let agent = agent_with_store(vec![FauxStep::say("second leg done")], &parent_file);
    assert!(
        agent.task_snapshots().is_empty(),
        "a restarted agent tracks nothing"
    );
    let host = agent.clone() as Arc<dyn TaskHost>;

    // Status probe first: the rehydrated snapshot reports what was on disk.
    let probe = host
        .run_task(req_with("", "two-leg task", false, Some(&id), None))
        .await
        .expect("probe");
    match probe {
        TaskOutcome::Running(snap) => {
            assert_eq!(snap.state, TaskState::Completed);
            assert_eq!(snap.turns, 1, "one assistant turn was restored");
            assert!(snap.output.contains("first leg done"), "{}", snap.output);
        }
        other => panic!("expected a status report, got {other:?}"),
    }

    // Then continue the same child: its history is intact, turns accumulate.
    let outcome = host
        .run_task(req_with(
            "finish the work",
            "two-leg task",
            false,
            Some(&id),
            None,
        ))
        .await
        .expect("resume");
    match outcome {
        TaskOutcome::Completed(res) => {
            assert!(res.text.contains("second leg done"), "{}", res.text);
            assert_eq!(
                res.details.get("turns").and_then(|v| v.as_u64()),
                Some(1),
                "this leg's turns"
            );
        }
        other => panic!("expected Completed, got {other:?}"),
    }
    let snap = agent.task_snapshot(&id).expect("tracked after rehydrate");
    assert_eq!(snap.turns, 2, "turns accumulate across the restart");
    assert_eq!(snap.state, TaskState::Completed);
}

#[tokio::test]
async fn unknown_ids_still_fail_honestly_when_nothing_is_on_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let agent = agent_with_store(vec![], &tmp.path().join("tui.jsonl"));
    let host = agent.clone() as Arc<dyn TaskHost>;
    let err = host
        .run_task(req_with(
            "",
            "probe",
            false,
            Some("task_nothing_here"),
            None,
        ))
        .await
        .expect_err("no such task");
    assert!(err.contains("unknown task id"), "{err}");
    assert!(err.contains("on disk"), "{err}");
}

// ---------------------------------------------------------------------------
// M93 / F18: a resumed child is running before the dispatch returns — and every
// way out of a leg (completion, in-band failure, panic, no child at all) leaves
// the registry in a state that is true.
// ---------------------------------------------------------------------------

/// An agent whose provider stalls `delay_ms` before every stream open (M93's
/// timing tests are deterministic: the stall is the assertion window, never a
/// sleep-and-hope) and, when a path is given, whose transcript is durable — the
/// M58 rehydrate paths need a session file to read children from.
fn slow_agent(
    steps: Vec<FauxStep>,
    delay_ms: u64,
    session_file: Option<&std::path::Path>,
) -> Arc<epsilon_core::Agent> {
    use epsilon_core::testing::SlowProvider;

    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(SlowProvider::new(FauxProvider::new(steps), delay_ms)),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    if let Some(path) = session_file {
        let store = epsilon_session::SessionStore::open_in(path, &std::env::temp_dir())
            .expect("parent store");
        agent.attach_session_store(Arc::new(std::sync::Mutex::new(store)));
    }
    agent
}

/// The task id a Completed delegation reported in its result details.
fn task_id_of(outcome: &TaskOutcome) -> String {
    match outcome {
        TaskOutcome::Completed(res) => res
            .details
            .get("task_id")
            .and_then(|v| v.as_str())
            .expect("id exposed in details")
            .to_string(),
        other => panic!("expected Completed, got {other:?}"),
    }
}

/// Seed a child session file that a status probe rehydrates into a settled
/// entry — no provider call, no stall to race. The registry-cap filler.
fn seed_child_file(dir: &std::path::Path, id: &str, subagent_type: &str) {
    let file = dir.join(format!("tui-{id}.jsonl"));
    let mut store = epsilon_session::SessionStore::open_child_in(&file, dir, "parent-filler")
        .expect("seed child file");
    store
        .append(epsilon_contracts::session_entry::EntryKind::Custom {
            custom_type: "task_meta".to_string(),
            data: Some(serde_json::json!({
                "task_id": id,
                "description": "settled filler",
                "subagent_type": subagent_type,
                "model": null,
            })),
        })
        .expect("meta entry");
    // The store drops here: its M61 write lease must be gone before a probe
    // reopens the file.
}

#[tokio::test]
async fn a_resumed_background_child_is_running_before_the_dispatch_returns() {
    // Leg one completes inline, leaving the entry in a terminal state...
    let agent = slow_agent(
        vec![FauxStep::say("leg one"), FauxStep::say("leg two")],
        200,
        None,
    );
    let host = agent.clone() as Arc<dyn TaskHost>;
    let first = host
        .run_task(req("first leg", "two-leg task", false, None))
        .await
        .expect("foreground leg");
    let id = task_id_of(&first);

    // ...and the background resume must announce `Running` before the leg has
    // run a step. The registry is all a later call can see, and reporting the
    // previous leg's `Completed` while a new leg is in flight is the defect.
    let resumed = host
        .run_task(req("second leg", "two-leg task", true, Some(&id)))
        .await
        .expect("resume");
    match resumed {
        TaskOutcome::Running(snapshot) => {
            assert_eq!(snapshot.id, id);
            assert_eq!(
                snapshot.state,
                TaskState::Running,
                "the dispatch must report the claim, not the old terminal state"
            );
        }
        other => panic!("expected Running, got {other:?}"),
    }
    let running = agent
        .task_snapshots()
        .into_iter()
        .filter(|s| s.state == TaskState::Running)
        .count();
    assert_eq!(running, 1, "exactly the resumed child is running");

    // The detached leg still lands its own outcome.
    let settled = await_settled(&agent, &id).await;
    assert_eq!(settled.state, TaskState::Completed);
    assert!(settled.output.contains("leg two"), "{}", settled.output);
}

#[tokio::test]
async fn a_second_resume_of_a_running_resumed_child_is_refused_without_forging_failure() {
    let agent = slow_agent(
        vec![
            FauxStep::say("leg one"),
            FauxStep::say("leg two"),
            FauxStep::say("parent ack"),
        ],
        300,
        None,
    );
    let host = agent.clone() as Arc<dyn TaskHost>;
    let first = host
        .run_task(req("first leg", "shared child", false, None))
        .await
        .expect("foreground leg");
    let id = task_id_of(&first);

    let resumed = host
        .run_task(req("second leg", "shared child", true, Some(&id)))
        .await
        .expect("resume");
    assert!(
        matches!(&resumed, TaskOutcome::Running(s) if s.state == TaskState::Running),
        "the resumed leg claims the entry: {resumed:?}"
    );

    // A second resume lands while that leg is in flight. It must be refused by
    // the claim itself — never accepted and then written `Failed` by the
    // duplicate leg's own `AlreadyRunning` error, which told the parent model
    // that a task still in flight had failed.
    let refusal = host
        .run_task(req("third leg", "shared child", true, Some(&id)))
        .await
        .expect_err("a second resume while the leg is in flight must be refused");
    assert!(refusal.contains("still running"), "{refusal}");
    let held = agent.task_snapshot(&id).expect("still tracked");
    assert_eq!(
        held.state,
        TaskState::Running,
        "the refusal must leave the claim untouched"
    );

    // The in-flight leg finishes normally; the notice the parent model receives
    // is its completion, and nothing queued a failure for it.
    let settled = await_settled(&agent, &id).await;
    assert_eq!(settled.state, TaskState::Completed);
    assert!(settled.output.contains("leg two"), "{}", settled.output);

    let run = agent.run(user_msg("carry on")).await.expect("parent run");
    let notices: Vec<String> = run
        .produced
        .iter()
        .filter_map(|m| match m {
            Message::User(u) => Some(
                u.content
                    .iter()
                    .filter_map(|c| match c {
                        UserContent::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            ),
            _ => None,
        })
        .collect();
    assert!(
        notices.iter().any(|n| n.contains("state=\"completed\"")),
        "leg two's completion notice must reach the parent: {notices:?}"
    );
    assert!(
        !notices.iter().any(|n| n.contains("state=\"failed\"")),
        "no failure may be forged for a task that never failed: {notices:?}"
    );
}

#[tokio::test]
async fn aborting_the_parent_reaches_a_resumed_running_child() {
    // One stall per leg; the resumed leg's stall is aborted long before it
    // elapses (M34's abort-during-stream path, not a waited-out timeout).
    let agent = slow_agent(
        vec![FauxStep::say("leg one"), FauxStep::say("never seen")],
        1_000,
        None,
    );
    let host = agent.clone() as Arc<dyn TaskHost>;
    let first = host
        .run_task(req("first leg", "resumable", false, None))
        .await
        .expect("foreground leg");
    let id = task_id_of(&first);

    let resumed = host
        .run_task(req("second leg", "resumable", true, Some(&id)))
        .await
        .expect("resume");
    assert!(matches!(&resumed, TaskOutcome::Running(s) if s.state == TaskState::Running));

    // The cascade's selector must see the resumed child...
    assert_eq!(
        agent.abort_delegated_tasks(),
        1,
        "a resumed running child must be reachable by the abort cascade"
    );
    // ...and a parent-level abort (no parent run is active, so it reports false
    // for the parent) leaves the child failed-aborted, never `Running`.
    assert!(
        !agent.abort_current_run(),
        "the return value describes the parent, which has no run in flight"
    );
    let settled = await_settled(&agent, &id).await;
    assert_eq!(settled.state, TaskState::Failed);
    assert!(
        settled.output.contains("aborted"),
        "the reason says aborted: {}",
        settled.output
    );
}

#[tokio::test]
async fn a_resumed_running_child_survives_registry_eviction() {
    let tmp = tempfile::tempdir().unwrap();
    let parent_file = tmp.path().join("tui.jsonl");
    let agent = slow_agent(
        vec![FauxStep::say("leg one"), FauxStep::say("leg two")],
        200,
        Some(&parent_file),
    );
    let host = agent.clone() as Arc<dyn TaskHost>;

    // Entry #1: a settled leg, then a background resume that is in flight when
    // the registry reaches its cap.
    let first = host
        .run_task(req("first leg", "cap child", false, None))
        .await
        .expect("foreground leg");
    let id = task_id_of(&first);

    // Seed the fillers first so only the probes — file reads, no provider
    // calls — have to fit inside the resumed leg's stall. Sizes match
    // `TASK_REGISTRY_CAP` (16, private to epsilon-core): entry #1 plus fifteen
    // probes fill the registry; the sixteenth probe is the eviction trigger.
    let fillers: Vec<String> = (0..16).map(|i| format!("task-fill-{i:02}")).collect();
    for fill in &fillers {
        seed_child_file(tmp.path(), fill, "default");
    }

    let resumed = host
        .run_task(req("second leg", "cap child", true, Some(&id)))
        .await
        .expect("resume");
    assert!(matches!(&resumed, TaskOutcome::Running(s) if s.state == TaskState::Running));

    for fill in &fillers[..15] {
        let probe = host
            .run_task(req_with("", "settled filler", false, Some(fill), None))
            .await
            .expect("probe");
        assert!(
            matches!(&probe, TaskOutcome::Running(s) if s.state == TaskState::Completed),
            "a seeded filler rehydrates settled: {probe:?}"
        );
    }
    assert_eq!(agent.task_ids().len(), 16, "the registry is at its cap");

    // The 17th registration evicts the oldest *settled* entry — not the child
    // first in dispatch order, whose eviction would leave the running child
    // untracked: a later resume or probe of that id fails on the still-held M61
    // store lease instead of reporting it running.
    let probe = host
        .run_task(req_with(
            "",
            "settled filler",
            false,
            Some(&fillers[15]),
            None,
        ))
        .await
        .expect("probe");
    assert!(matches!(&probe, TaskOutcome::Running(_)));
    let child = agent
        .task_snapshot(&id)
        .expect("a running child must not be evicted for capacity");
    assert_eq!(
        child.state,
        TaskState::Running,
        "and still running, not settled"
    );
    assert!(
        agent.task_snapshot(&fillers[0]).is_none(),
        "the eviction took the oldest settled entry instead"
    );
    assert_eq!(agent.task_ids().len(), 16, "the registry stays at its cap");
}

#[tokio::test]
async fn a_resumed_run_that_fails_lands_failed_not_running() {
    let tmp = tempfile::tempdir().unwrap();
    let parent_file = tmp.path().join("tui.jsonl");
    let agent = slow_agent(
        vec![
            FauxStep::say("leg one"),
            FauxStep::Error {
                message: "provider exploded".into(),
            },
        ],
        150,
        Some(&parent_file),
    );
    let host = agent.clone() as Arc<dyn TaskHost>;
    let first = host
        .run_task(req("first leg", "doomed resume", false, None))
        .await
        .expect("foreground leg");
    let id = task_id_of(&first);

    // The failing leg's error must be accompanied by a terminal write: the claim
    // this milestone adds is only honest while every way out of a leg records
    // its outcome, so the entry lands `Failed` with the reason — never left
    // `Running`, which is what an unpaired claim would strand it in.
    let failure = host
        .run_task(req("second leg", "doomed resume", false, Some(&id)))
        .await
        .expect_err("the failing leg must surface as an error");
    assert!(failure.contains("provider exploded"), "{failure}");
    let settled = agent.task_snapshot(&id).expect("still tracked");
    assert_eq!(
        settled.state,
        TaskState::Failed,
        "a failed resumed run lands Failed, not Running"
    );
    assert!(
        settled.output.contains("provider exploded"),
        "{}",
        settled.output
    );
    assert!(
        agent
            .task_snapshots()
            .iter()
            .all(|s| s.state != TaskState::Running),
        "no entry may be left Running after a failed leg"
    );

    // A rehydrate that cannot rebuild its child fails *before* any claim: nothing
    // may be registered — an entry left `Running` by a failed lookup would be the
    // other lie the claim's placement (after the last fallible step) prevents.
    let ghost = "task-ghost-type";
    seed_child_file(tmp.path(), ghost, "no-such-agent-type");
    let err = host
        .run_task(req_with("", "ghost", false, Some(ghost), None))
        .await
        .expect_err("a child whose type cannot be rebuilt must fail");
    assert!(err.contains("unknown agent type"), "{err}");
    assert!(
        !agent.task_ids().iter().any(|tracked| tracked == ghost),
        "a failed rehydrate registers no entry"
    );
    assert!(
        agent
            .task_snapshots()
            .iter()
            .all(|s| s.state != TaskState::Running),
        "and leaves nothing Running behind"
    );
}

/// Provider that answers the first call and panics where the seam would be on
/// the second. Used *without* `PanicGuard` so the panic reaches the agent loop
/// the way a core bug — rather than a provider fault — would (the F18 wrap must
/// contain it either way).
struct PanicOnSecondCall {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl epsilon_llm::StreamFn for PanicOnSecondCall {
    async fn stream(
        &self,
        model: &epsilon_llm::ModelRef,
        ctx: epsilon_llm::LlmContext,
    ) -> epsilon_llm::BoxEventStream {
        if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
            panic!("provider exploded");
        }
        FauxProvider::new(vec![FauxStep::say("provider answer")])
            .stream(model, ctx)
            .await
    }
}

#[tokio::test]
async fn a_panicking_resumed_run_lands_failed_not_running() {
    // The extra test the F18 wrap owes: a panic inside the detached body used to
    // end the task silently, leaving the entry at its previous state and never
    // queueing a notice — the registry reported a settled child for a leg that
    // died mid-run.
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(PanicOnSecondCall {
            calls: std::sync::atomic::AtomicUsize::new(0),
        }),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    let host = agent.clone() as Arc<dyn TaskHost>;
    let first = host
        .run_task(req("first leg", "panic probe", false, None))
        .await
        .expect("foreground leg");
    let id = task_id_of(&first);

    let resumed = host
        .run_task(req("second leg", "panic probe", true, Some(&id)))
        .await
        .expect("resume");
    assert!(matches!(&resumed, TaskOutcome::Running(s) if s.state == TaskState::Running));

    let settled = await_settled(&agent, &id).await;
    assert_eq!(
        settled.state,
        TaskState::Failed,
        "a panicking run must still be recorded as failed"
    );
    assert!(
        settled.output.contains("panicked"),
        "the record says the run panicked: {}",
        settled.output
    );
    assert!(
        settled.output.contains("provider exploded"),
        "and keeps the panic's own message: {}",
        settled.output
    );

    // The parent model is told the truth too: the notice says failed.
    let run = agent.run(user_msg("carry on")).await.expect("parent run");
    let notices: Vec<String> = run
        .produced
        .iter()
        .filter_map(|m| match m {
            Message::User(u) => Some(
                u.content
                    .iter()
                    .filter_map(|c| match c {
                        UserContent::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            ),
            _ => None,
        })
        .collect();
    assert!(
        notices.iter().any(|n| n.contains("state=\"failed\"")),
        "the panic must reach the parent as a failed notice: {notices:?}"
    );
}

#[tokio::test]
async fn a_panicking_foreground_resume_lands_failed_not_running() {
    // The foreground leg had no panic boundary of its own: the claim (or the fresh
    // register) writes `Running` before the background/foreground split, so an unwind
    // straight through `await_task_run` used to leave the just-claimed entry `Running`
    // forever. The containment must land the same terminal record the detached body
    // lands.
    let agent = epsilon_core::Agent::new(
        LoopLimits::default(),
        Arc::new(PanicOnSecondCall {
            calls: std::sync::atomic::AtomicUsize::new(0),
        }),
        Arc::new(ToolRegistry::new()),
        Arc::new(AutoAllow),
        std::env::temp_dir(),
    );
    let host = agent.clone() as Arc<dyn TaskHost>;
    let first = host
        .run_task(req("first leg", "foreground panic probe", false, None))
        .await
        .expect("foreground leg");
    let id = task_id_of(&first);

    // The panic surfaces as the call's error — not as an unwind through the caller...
    let failure = host
        .run_task(req(
            "second leg",
            "foreground panic probe",
            false,
            Some(&id),
        ))
        .await
        .expect_err("a panicking foreground leg must surface as an error");
    assert!(failure.contains("panicked"), "{failure}");
    assert!(failure.contains("provider exploded"), "{failure}");

    // ...and the entry the claim marked `Running` ends `Failed`, naming the panic.
    let settled = agent.task_snapshot(&id).expect("still tracked");
    assert_eq!(
        settled.state,
        TaskState::Failed,
        "a panicking foreground run must still be recorded as failed"
    );
    assert!(
        settled.output.contains("panicked"),
        "the record says the run panicked: {}",
        settled.output
    );
    assert!(
        settled.output.contains("provider exploded"),
        "and keeps the panic's own message: {}",
        settled.output
    );
}
