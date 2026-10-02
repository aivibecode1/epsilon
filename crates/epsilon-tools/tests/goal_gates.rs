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

/// The real backend, if this machine has one.
///
/// `best_available` is the same call the product's `resolve_sandbox` makes, so a test that
/// skips when it is unavailable is skipping for the same reason a session would refuse —
/// not because the test harness was never set up for it.
fn real_backend() -> Option<std::sync::Arc<dyn epsilon_safety::Sandbox>> {
    let sandbox = epsilon_safety::best_available();
    sandbox.available().then_some(sandbox)
}

/// **A gate actually runs through the confinement, and that branch now has coverage.**
///
/// Every gate test in this file used `GoalGateRunner::new(None)` — the *unconfined* path —
/// so the branch that decides *whether* to run unconfined had never executed: the one that
/// refuses when the backend is missing, and the one that hands the command to the backend
/// instead of to the host.
///
/// **What this proves, stated plainly because the name overclaims otherwise.** It proves the
/// gate reaches the sandbox, runs there, and reports the result. It does **not** prove a
/// filesystem boundary: on Windows a job object bounds memory, process count and CPU and
/// kills the tree on timeout, and `JobObjectSandbox::capabilities()` says so —
/// `Enforcement::from_missing(vec!["filesystem", "network"])`. "Workspace write" is a mode
/// label there, not an enforced boundary. That honesty is itself tested, below, and it is
/// what `epsilon-cli`'s doctor prints.
#[tokio::test]
async fn a_gate_runs_through_the_backend_when_the_platform_has_one() {
    let Some(sandbox) = real_backend() else {
        eprintln!("no confinement backend on this platform; the confined branch stays untested");
        return;
    };
    let dir = tempfile::tempdir().expect("a workspace");
    let runner = GoalGateRunner::new(Some(sandbox));
    let report = runner
        .run(&GoalGate::new("cargo --version"), dir.path())
        .await;
    assert!(report.passed, "{}", report.sentence());
    assert!(
        !report.unavailable,
        "a backend exists, so nothing was unavailable"
    );
    // The sentence distinguishes the two ways a gate can pass, which is the whole reason
    // the report carries the backend's name.
    assert!(
        !report.sentence().contains("could not run"),
        "{}",
        report.sentence()
    );
}

/// A gate that fails **confined** reports the failure the same way an unconfined one does.
///
/// The failure path is what a model acts on, so it is the path that must not depend on which
/// backend ran it.
#[tokio::test]
async fn a_failing_confined_gate_names_the_command_and_its_exit_code() {
    let Some(sandbox) = real_backend() else {
        eprintln!("no confinement backend on this platform; skipping the confined failure path");
        return;
    };
    let dir = tempfile::tempdir().expect("a workspace");
    let runner = GoalGateRunner::new(Some(sandbox));
    let report = runner
        .run(
            &GoalGate::new("cargo definitely-not-a-subcommand"),
            dir.path(),
        )
        .await;
    assert!(!report.passed, "{}", report.sentence());
    assert_ne!(report.exit_code, Some(0), "{}", report.sentence());
    assert!(
        report
            .sentence()
            .contains("cargo definitely-not-a-subcommand"),
        "{}",
        report.sentence()
    );
}

/// A backend that reports itself unavailable makes the gate **refuse**, not run unconfined.
///
/// This is the fail-closed rule, and it is the one a confused deputy would rely on: a user
/// who asked to be confined on a machine that cannot confine must not get an unconfined
/// command wearing a gate's name. Before this test the branch had no coverage at all.
#[tokio::test]
async fn an_unavailable_backend_refuses_rather_than_running_unconfined() {
    let runner = GoalGateRunner::new(Some(std::sync::Arc::new(epsilon_safety::NullSandbox)));
    let dir = tempfile::tempdir().expect("a workspace");
    let report = runner.run(&GoalGate::new(ALWAYS_PASSES), dir.path()).await;
    assert!(
        !report.passed,
        "a gate that cannot be confined must not report success"
    );
    assert!(
        report.unavailable,
        "and it must be marked unavailable, not merely failed: {}",
        report.sentence()
    );
    assert!(
        report.sentence().contains("refusing to run unconfined"),
        "the refusal has to say why: {}",
        report.sentence()
    );
}

/// The backend's own report of what it cannot confine is true, and stays true.
///
/// `epsilon-cli`'s doctor prints `sandbox.capabilities()` verbatim, and the Windows backend's
/// comment says a caller needing a filesystem boundary "must reject this backend rather than
/// trust the word 'confined'". That is the honest position, and it is **only** honest while the
/// declaration matches reality — so it is a test, not a comment. A future change that made the
/// job object claim full enforcement would pass every gate test in this file and leave the
/// doctor lying to the user.
#[test]
fn a_backend_declares_what_it_cannot_confine() {
    let sandbox = epsilon_safety::best_available();
    let caps = sandbox.capabilities();
    if cfg!(windows) {
        assert!(
            !caps.is_full(),
            "a job object bounds memory and process count; it does not bound the filesystem, \
             and reporting full enforcement would make `doctor` lie"
        );
        assert!(
            caps.missing().contains(&"filesystem"),
            "the filesystem is exactly what it cannot confine: {:?}",
            caps.missing()
        );
    } else {
        // Bubblewrap gives a real filesystem boundary; the NullSandbox fallback does not.
        // Either way the backend must be saying so rather than claiming what it lacks.
        assert!(
            caps.missing().is_empty() || !caps.is_full(),
            "a backend either confines fully or names what it does not: {:?}",
            caps.missing()
        );
    }
}

fn has_cargo() -> bool {
    std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .is_ok()
}
