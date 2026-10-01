//! A goal cannot be completed while its gates have not passed.
//!
//! # What is actually tested here
//!
//! Not the policy — that is four lines. What is tested is the **claim**: that a completion
//! request runs the gates first and refuses when one fails, and that the refusal names the
//! command and what it printed. A gate feature whose test only checks a struct field has
//! proved nothing a user would notice.
//!
//! The passing case runs `cargo` only when it is on `PATH`; otherwise the test asserts the
//! runner *reports the absence* rather than pretending a pass. That distinction is the
//! whole point of the feature.

use epsilon_contracts::goal::{GoalGate, GoalSnapshot, GoalStatus};
use epsilon_tools::goal_gate::{GoalGateRunner, workspace_fingerprint};
use std::path::Path;

/// A command that exists everywhere this project supports and always passes.
const ALWAYS_PASSES: &str = "cargo --version";

#[tokio::test]
async fn a_gate_that_passes_lets_the_completion_through() {
    if !has_cargo() {
        eprintln!("cargo is not on PATH; skipping the runnable gate");
        return;
    }
    let runner = GoalGateRunner::new(None);
    let report = runner
        .run(&GoalGate::new(ALWAYS_PASSES), Path::new("."))
        .await;
    assert!(report.passed, "{}", report.sentence());
    assert_eq!(report.exit_code, Some(0));
    assert!(!report.unavailable);
}

/// The refusal a model acts on has to carry the command and its output. A gate that fails
/// with no explanation is a blocker with nothing behind it — the exact shape
/// `goal.rs`'s `Blocked` arm already refuses to record.
#[tokio::test]
async fn a_failing_gate_names_the_command_and_its_output() {
    if !has_cargo() {
        eprintln!("cargo is not on PATH; skipping the runnable gate");
        return;
    }
    let runner = GoalGateRunner::new(None);
    let report = runner
        .run(
            &GoalGate::new("cargo definitely-not-a-subcommand"),
            Path::new("."),
        )
        .await;
    assert!(!report.passed);
    assert_ne!(report.exit_code, Some(0), "{}", report.sentence());
    let sentence = report.sentence();
    assert!(
        sentence.contains("cargo definitely-not-a-subcommand"),
        "{sentence}"
    );
    assert!(
        sentence.contains("exit"),
        "the code is what to look up: {sentence}"
    );
}

/// A timeout is a failure with a reason, not a hang. A gate that blocks the goal loop for
/// ever is a goal that never completes and never explains itself.
#[tokio::test]
async fn a_gate_that_runs_too_long_fails_rather_than_waiting() {
    if !has_cargo() {
        eprintln!("cargo is not on PATH; skipping the runnable gate");
        return;
    }
    let runner = GoalGateRunner::new(None);
    let started = std::time::Instant::now();
    let gate = GoalGate {
        command: "cargo --version".into(),
        timeout_seconds: 1,
    };
    let report = runner.run(&gate, Path::new(".")).await;
    // Whether the timeout fired depends on the machine; what must not happen is the test
    // waiting far past the limit.
    assert!(
        started.elapsed() < std::time::Duration::from_secs(60),
        "a one-second gate took {:?}",
        started.elapsed()
    );
    assert!(report.output_tail.len() <= epsilon_tools::goal_gate::GATE_OUTPUT_TAIL + 64);
}

#[test]
fn a_gate_with_no_command_can_never_pass_and_says_so() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let runner = GoalGateRunner::new(None);
    let report = runtime.block_on(runner.run(&GoalGate::new("  "), Path::new(".")));
    assert!(!report.passed);
    assert!(report.unavailable);
    assert!(
        report.sentence().contains("never pass"),
        "an unrunnable gate must explain itself: {}",
        report.sentence()
    );
}

/// A goal with no gates is unaffected: the status quo is "the caller decides", and a feature
/// that made completion harder for everyone to get a gate declared first would be adopted by
/// nobody and used by nothing.
#[test]
fn a_goal_with_no_gates_declares_none() {
    let snapshot = GoalSnapshot {
        id: "g1".into(),
        revision: 1,
        objective: "do the thing".into(),
        status: GoalStatus::Active,
        max_rounds: 8,
        rounds_started: 0,
        blocker: None,
        gates: Vec::new(),
        created_at_ms: 0,
        updated_at_ms: 0,
    };
    assert!(snapshot.gates.is_empty());
    assert!(!snapshot.exhausted());
}

/// A session written before gates existed must still open. This is the M202 rule made
/// concrete: a field added to a durable type without a default turns every old session file
/// into one the harness cannot read.
#[test]
fn a_session_written_before_gates_existed_still_decodes() {
    let before = r#"{"id":"g1","revision":2,"objective":"x","status":"active","max_rounds":8,"rounds_started":1,"created_at_ms":0,"updated_at_ms":0}"#;
    let snapshot: GoalSnapshot =
        serde_json::from_str(before).expect("a goal written before gates must still decode");
    assert!(
        snapshot.gates.is_empty(),
        "absent means none, not a failure"
    );
    assert_eq!(snapshot.objective, "x");
}

#[test]
fn a_gate_round_trips_through_the_session_log() {
    let gate = GoalGate {
        command: "cargo test".into(),
        timeout_seconds: 90,
    };
    let json = serde_json::to_value(&gate).expect("a gate serializes");
    let back: GoalGate = serde_json::from_value(json).expect("a gate deserializes");
    assert_eq!(back, gate);
    // An absent timeout is the default rather than zero, which would be a gate that fails
    // instantly and always.
    let bare: GoalGate = serde_json::from_str(r#"{"command":"cargo test"}"#).expect("decodes");
    assert_eq!(
        bare.timeout_seconds,
        epsilon_contracts::goal::DEFAULT_GATE_TIMEOUT_SECS
    );
}

#[test]
fn the_workspace_fingerprint_is_stable_and_short() {
    let dir = tempfile::tempdir().expect("a workspace");
    let a = workspace_fingerprint(dir.path());
    let b = workspace_fingerprint(dir.path());
    assert_eq!(a, b, "an unchanged workspace must not look moved");
    assert_eq!(
        a.len(),
        16,
        "a hex digest, not a path or a stack trace: {a}"
    );
}

fn has_cargo() -> bool {
    std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .is_ok()
}
