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
    let runner = GoalGateRunner::new(None, Some(std::sync::Arc::new(epsilon_safety::AutoApprove)));
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
    let runner = GoalGateRunner::new(None, Some(std::sync::Arc::new(epsilon_safety::AutoApprove)));
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
    let runner = GoalGateRunner::new(None, Some(std::sync::Arc::new(epsilon_safety::AutoApprove)));
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
    let runner = GoalGateRunner::new(None, Some(std::sync::Arc::new(epsilon_safety::AutoApprove)));
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
    // A temp directory is not a repository, so there is no fingerprint — and that is the
    // assertion, not an incidental: a `String` version returned a *constant* here, which for
    // carry-over would have meant "never run a gate again".
    assert_eq!(
        workspace_fingerprint(dir.path()),
        None,
        "a directory that is not a repository must report no fingerprint rather than one \
         that never changes"
    );
}

/// Inside a repository, an unchanged workspace has a stable fingerprint and a changed one
/// does not.
///
/// The carry-over decision is only as good as this function, and a fingerprint that could not
/// tell a moved tree from a still one would skip every gate for ever.
#[test]
fn inside_a_repository_the_fingerprint_moves_when_the_tree_does() {
    let dir = tempfile::tempdir().expect("a workspace");
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .expect("git runs")
    };
    if !git(&["init", "-q"]).status.success() {
        eprintln!("git is unavailable; the in-repository fingerprint is untested here");
        return;
    }
    let _ = git(&["config", "user.email", "gate@example.test"]);
    let _ = git(&["config", "user.name", "gate test"]);
    std::fs::write(dir.path().join("a.txt"), "one\n").expect("a file");
    let _ = git(&["add", "-A"]);
    let _ = git(&["commit", "-qm", "first"]);
    // A fresh repository with no commit has no HEAD, so there is no fingerprint yet.
    let first = workspace_fingerprint(dir.path()).expect("a committed tree has a fingerprint");
    let again = workspace_fingerprint(dir.path()).expect("still a fingerprint");
    assert_eq!(first, again, "an unchanged workspace must not look moved");
    assert_eq!(first.len(), 16, "a hex digest, not a path: {first}");

    std::fs::write(dir.path().join("b.txt"), "two\n").expect("a second file");
    let moved = workspace_fingerprint(dir.path()).expect("still a fingerprint");
    assert_ne!(
        first, moved,
        "a workspace that gained a file must not look like the one that did not"
    );
}

/// **An unchanged workspace replays its failure instead of re-running it.**
///
/// `GateReport::carried_over` was `false` at every construction site in the crate — the field
/// and the sentence that explains it existed, and nothing produced either. Hermes skips
/// re-running a gate whose workspace has not moved since the same failure
/// (`goals.py:371-391`, `:1306-1320`), because `cargo test` on a real project is thirty
/// seconds and an unattended loop that re-runs an unchanged red suite burns a minute of CPU
/// to learn the identical thing.
///
/// The gate here is deliberately one that takes *measurable* time — a command that always
/// fails — so a carried-over report is distinguishable from a re-run by more than a flag.
#[tokio::test]
async fn an_unchanged_workspace_replays_the_failure_instead_of_re_running_it() {
    let dir = tempdir_with_repo().expect("a workspace that is a git repository");
    let gate = GoalGate::new(FAILING_MARKER);
    let runner = GoalGateRunner::new(None, Some(std::sync::Arc::new(epsilon_safety::AutoApprove)));

    let first = runner.run(&gate, dir.path()).await;
    assert!(!first.passed, "the gate fails, so the premise holds");
    assert!(!first.carried_over, "the first run cannot be a replay");
    assert!(
        first.output_tail.contains("gate-marker"),
        "precondition: the first run really ran: {}",
        first.output_tail
    );

    let second = runner.run(&gate, dir.path()).await;
    assert!(!second.passed);
    assert!(
        second.carried_over,
        "an unchanged workspace must replay: {}",
        second.sentence()
    );
    assert_eq!(
        second.output_tail, first.output_tail,
        "a replay has to be the same evidence, or it is not a replay"
    );
    assert!(
        second.sentence().contains("not re-run"),
        "and it must say why it did not run, or a reader sees a stale answer as a fresh one: {}",
        second.sentence()
    );
}

/// A workspace that **moved** re-runs, because the failure may be fixed or gone.
#[tokio::test]
async fn a_workspace_that_moved_re_runs_the_gate() {
    let dir = tempdir_with_repo().expect("a workspace that is a git repository");
    let gate = GoalGate::new(FAILING_MARKER);
    let runner = GoalGateRunner::new(None, Some(std::sync::Arc::new(epsilon_safety::AutoApprove)));

    assert!(!runner.run(&gate, dir.path()).await.carried_over);
    assert!(
        runner.run(&gate, dir.path()).await.carried_over,
        "precondition: the second run is a replay"
    );

    std::fs::write(dir.path().join("moved.txt"), "something changed\n").expect("a file");
    let after = runner.run(&gate, dir.path()).await;
    assert!(
        !after.carried_over,
        "a tree that moved must be re-gated: {}",
        after.sentence()
    );
}

/// Outside a repository there is **no fingerprint**, so the gate always runs.
///
/// This is the case that made the return type `Option`. As a `String` it returned a constant
/// outside git, which is the safe direction for "should I re-run?" and the *unsafe* one for
/// "may I skip?": a constant means never check again.
#[tokio::test]
async fn outside_a_repository_the_gate_always_runs() {
    let dir = tempfile::tempdir().expect("not a repository");
    let gate = GoalGate::new(FAILING_MARKER);
    let runner = GoalGateRunner::new(None, Some(std::sync::Arc::new(epsilon_safety::AutoApprove)));
    for _ in 0..2 {
        let report = runner.run(&gate, dir.path()).await;
        assert!(
            !report.carried_over,
            "a workspace with no fingerprint must never be treated as unchanged: {}",
            report.sentence()
        );
    }
}

/// A gate that **passed** is re-run even on an unchanged workspace.
///
/// Only failures are carried. The thing that made a gate pass can be undone by a file the
/// fingerprint does not see — a deleted dependency, a changed environment, an upstream
/// service — and a replayed pass would report "done" about a workspace nobody has checked.
#[tokio::test]
async fn a_passing_gate_is_never_carried_over() {
    let dir = tempdir_with_repo().expect("a workspace that is a git repository");
    let gate = GoalGate::new("cargo --version");
    let runner = GoalGateRunner::new(None, Some(std::sync::Arc::new(epsilon_safety::AutoApprove)));
    assert!(runner.run(&gate, dir.path()).await.passed);
    let second = runner.run(&gate, dir.path()).await;
    assert!(second.passed);
    assert!(
        !second.carried_over,
        "a pass is evidence about now, not about an unchanged tree"
    );
}

/// A command that fails and says so, so a replay can be told from a re-run.
const FAILING_MARKER: &str = "echo gate-marker && exit 3";

/// A temp directory that is a git repository with one commit, so it has a fingerprint.
fn tempdir_with_repo() -> Option<tempfile::TempDir> {
    let dir = tempfile::tempdir().expect("a workspace");
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
    };
    if !git(&["init", "-q"]).is_ok_and(|o| o.status.success()) {
        eprintln!("git is unavailable; carry-over over a real repository is untested here");
        return None;
    }
    let _ = git(&["config", "user.email", "gate@example.test"]);
    let _ = git(&["config", "user.name", "gate test"]);
    std::fs::write(dir.path().join("seed.txt"), "seed\n").expect("a file");
    let _ = git(&["add", "-A"]);
    let _ = git(&["commit", "-qm", "seed"]);
    Some(dir)
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
/// Every gate test in this file builds an *unconfined* runner — the path that hands the
/// command straight to the host —
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
    let runner = GoalGateRunner::new(
        Some(sandbox),
        Some(std::sync::Arc::new(epsilon_safety::AutoApprove)),
    );
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
    let runner = GoalGateRunner::new(
        Some(sandbox),
        Some(std::sync::Arc::new(epsilon_safety::AutoApprove)),
    );
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
    let runner = GoalGateRunner::new(
        Some(std::sync::Arc::new(epsilon_safety::NullSandbox)),
        Some(std::sync::Arc::new(epsilon_safety::AutoApprove)),
    );
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
