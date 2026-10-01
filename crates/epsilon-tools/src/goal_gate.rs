//! Running a goal's gates: the evidence that decides whether a goal is done.
//!
//! # Why the gate runs through the sandbox and not around it
//!
//! A gate is a shell command, and it is run by the **same** [`Sandbox`] the `bash` tool
//! uses, resolved from the same [`Confinement`] in the same
//! [`register_builtins_with`](crate::register_builtins_with) call. That is the whole design,
//! and it is a security decision rather than a tidiness one:
//!
//! * A gate declared outside confinement would be a **confused deputy**. Everything the
//!   agent runs is confined; one command the agent cannot run but the goal can would turn the
//!   permission gate into decoration.
//! * Sharing the resolution means there is **one** answer to "which backend is in force",
//!   so a transcript cannot say `confinement: job-object` for a tool call and
//!   `confinement: off` for the gate that decided whether it succeeded.
//! * `Confinement::Off` still means off, and is still recorded. A gate is not a way to smuggle
//!   an unconfined command past a read-only session.
//!
//! # Why the fingerprint exists
//!
//! Re-running `cargo test` for a workspace that has not moved answers the same thing every
//! time, and on a slow suite that is minutes burned per round. Hermes skips a re-run when the
//! workspace fingerprint matches the last *failure*
//! (`goals.py:371-391`), and so does this: an unchanged tree with the same failing gate is
//! reported from the last result rather than recomputed.
//!
//! Outside a git repository there is no fingerprint, and the rule is to **always run** —
//! which is the safe direction, not the convenient one.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use epsilon_contracts::goal::GoalGate;
use epsilon_safety::{ConfinedCommand, ConfinementPolicy, Sandbox};

/// How much of a failing gate's output is kept.
///
/// A test suite can print megabytes; the continuation prompt needs the reason, not the
/// scrollback. Bounded so a gate cannot blow the context of the very round it was meant to
/// inform.
pub const GATE_OUTPUT_TAIL: usize = 3_000;

/// What one gate did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateReport {
    /// The gate this is about.
    pub gate: GoalGate,
    /// Whether it passed.
    pub passed: bool,
    /// The process exit code, when one ran.
    pub exit_code: Option<i32>,
    /// The tail of its output: why it failed, or a trace of what it checked.
    pub output_tail: String,
    /// `true` when this answer was carried over rather than recomputed.
    pub carried_over: bool,
    /// `true` when the gate could not run at all — no backend, or an empty command.
    pub unavailable: bool,
}

impl GateReport {
    /// One sentence naming the command, the result, and what to do about it.
    ///
    /// A refusal a reader cannot act on is the defect class this repository keeps paying for,
    /// so the exit code and the command are both in it.
    pub fn sentence(&self) -> String {
        if self.unavailable {
            return format!(
                "goal gate `{}` could not run: {}",
                self.gate.command, self.output_tail
            );
        }
        if self.carried_over {
            return format!(
                "goal gate `{}` still failing (not re-run: the workspace has not changed since \
                 the last attempt)",
                self.gate.command
            );
        }
        if self.passed {
            return format!("goal gate `{}` passed", self.gate.command);
        }
        format!(
            "goal gate `{}` failed{}",
            self.gate.command,
            self.exit_code
                .map(|c| format!(" (exit {c})"))
                .unwrap_or_default()
        )
    }
}

/// Runs a goal's gates under one sandbox.
#[derive(Clone)]
pub struct GoalGateRunner {
    /// The backend, or `None` when the session runs unconfined by explicit choice.
    sandbox: Option<Arc<dyn Sandbox>>,
    policy: ConfinementPolicy,
}

impl std::fmt::Debug for GoalGateRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoalGateRunner")
            .field(
                "sandbox",
                &self.sandbox.as_ref().map(|s| s.name().to_string()),
            )
            .finish()
    }
}

impl GoalGateRunner {
    /// A runner over `sandbox`.
    ///
    /// `None` means the session chose [`Confinement::Off`](crate::Confinement::Off)
    /// explicitly. **That is a choice, not a missing backend**, and the gate honours it by
    /// running unconfined — the first version of this refused, which inverted the shell
    /// tool's own rule and made the feature inert for exactly the people who opted in.
    ///
    /// The case that *does* refuse is `Some(sandbox)` where the backend is unavailable:
    /// there the user asked to be confined and the machine cannot do it, and failing
    /// closed is the whole point.
    pub fn new(sandbox: Option<Arc<dyn Sandbox>>) -> Self {
        Self {
            sandbox,
            policy: ConfinementPolicy::default(),
        }
    }

    /// Run one gate in `cwd` and report what happened.
    pub async fn run(&self, gate: &GoalGate, cwd: &Path) -> GateReport {
        if !gate.is_runnable() {
            return GateReport {
                gate: gate.clone(),
                passed: false,
                exit_code: None,
                output_tail: "the gate has no command, so it can never pass".into(),
                carried_over: false,
                unavailable: true,
            };
        }

        let Some(sandbox) = self.sandbox.as_ref() else {
            // Explicitly unconfined by the user's own choice; the report says so, because a
            // transcript that cannot distinguish "ran unconfined" from "ran in a job
            // object" is not the audit record this project keeps asking for.
            return run_unconfined(gate, cwd).await;
        };
        if !sandbox.available() {
            return GateReport {
                gate: gate.clone(),
                passed: false,
                exit_code: None,
                output_tail: format!(
                    "no confinement backend is available on this platform (`{}`), refusing to \
                     run unconfined",
                    sandbox.name()
                ),
                carried_over: false,
                unavailable: true,
            };
        }

        let (program, argv) = crate::shell::shell_argv(&gate.command);
        let mut policy = self.policy.clone();
        policy.wall_timeout_secs = u64::from(gate.timeout_seconds.max(1));

        match sandbox
            .run(
                ConfinedCommand {
                    program: program.into(),
                    args: argv,
                    cwd: cwd.to_path_buf(),
                    mode: epsilon_safety::SandboxMode::WorkspaceWrite,
                },
                &policy,
                None,
            )
            .await
        {
            Ok(outcome) => {
                let combined = [outcome.stdout.as_str(), outcome.stderr.as_str()]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n");
                GateReport {
                    gate: gate.clone(),
                    passed: outcome.exit_code == Some(0),
                    exit_code: outcome.exit_code,
                    output_tail: tail(&combined, GATE_OUTPUT_TAIL),
                    carried_over: false,
                    unavailable: false,
                }
            }
            Err(e) => GateReport {
                gate: gate.clone(),
                passed: false,
                exit_code: None,
                output_tail: format!("{e}"),
                carried_over: false,
                unavailable: true,
            },
        }
    }
}

/// Run a gate without a sandbox, because the session asked for exactly that.
///
/// The timeout is enforced here too, and enforced the same way: a gate that blocks the goal
/// loop for ever is a goal that never completes and never explains itself.
async fn run_unconfined(gate: &GoalGate, cwd: &Path) -> GateReport {
    let (program, argv) = crate::shell::shell_argv(&gate.command);
    let limit = Duration::from_secs(u64::from(gate.timeout_seconds.max(1)));
    let program = program.to_string();
    let name = program.clone();
    let cwd = cwd.to_path_buf();
    let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();

    let run = async move {
        let mut cmd = tokio::process::Command::new(&program);
        cmd.args(&argv).current_dir(&cwd);
        // The same preparation the shell tool applies: no stdin to block on, piped output,
        // and the secret-scrubbed environment. A gate that inherited secrets the session
        // deliberately withheld from every other command would be the one command in the
        // process that leaked them.
        crate::shell::ChildStdio::configure(&mut cmd);
        epsilon_safety::scrub_secret_env(&mut cmd);
        cmd.output().await
    };

    let unfinished = format!("`{name}` did not finish within {limit:?}");
    let joined = tokio::time::timeout(limit, run).await;
    match joined {
        Ok(Ok(out)) => {
            let combined = [
                String::from_utf8_lossy(&out.stdout).into_owned(),
                String::from_utf8_lossy(&out.stderr).into_owned(),
            ]
            .iter()
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
            GateReport {
                gate: gate.clone(),
                passed: out.status.success(),
                exit_code: out.status.code(),
                output_tail: tail(&combined, GATE_OUTPUT_TAIL),
                carried_over: false,
                unavailable: false,
            }
        }
        Ok(Err(e)) => GateReport {
            gate: gate.clone(),
            passed: false,
            exit_code: None,
            output_tail: format!("could not run `{name}`: {e}"),
            carried_over: false,
            unavailable: true,
        },
        Err(_) => GateReport {
            gate: gate.clone(),
            passed: false,
            exit_code: None,
            output_tail: unfinished,
            carried_over: false,
            unavailable: true,
        },
    }
}

/// The last `limit` characters of `text`, prefixed when it was cut.
fn tail(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.trim_end().to_string();
    }
    // Character-safe: cutting mid-codepoint would put a replacement character in a
    // continuation prompt, which is exactly the sort of corruption nobody looks for.
    let cut: String = text
        .chars()
        .skip(text.chars().count() - limit)
        .collect::<String>();
    format!("… (earlier output dropped)\n{cut}")
}

/// A cheap fingerprint of the workspace: has anything moved?
///
/// **Not a security primitive and not meant to be one** — it detects change, it does not
/// authenticate. A `DefaultHasher` over `git rev-parse HEAD` and `git status --porcelain`
/// answers "did the tree move" without adding a dependency for a value nobody ever stores.
pub fn workspace_fingerprint(workspace: &Path) -> String {
    let head = run_capture(workspace, &["git", "rev-parse", "HEAD"]);
    let status = run_capture(workspace, &["git", "status", "--porcelain"]);
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    head.hash(&mut h);
    status.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Run `program` and return its trimmed stdout, or an empty string.
///
/// The fingerprint must work with **no** git, in a directory that is not a repository, and
/// in a workspace with a detached head. Every one of those returns empty rather than
/// failing, because a fingerprint that cannot be computed must not stop a goal.
fn run_capture(workspace: &Path, program: &[&str]) -> String {
    let Ok(out) = std::process::Command::new(program[0])
        .args(&program[1..])
        .current_dir(workspace)
        .output()
    else {
        return String::new();
    };
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_names_the_command_and_the_exit_code() {
        let report = GateReport {
            gate: GoalGate::new("cargo test"),
            passed: false,
            exit_code: Some(101),
            output_tail: "test result: FAILED".into(),
            carried_over: false,
            unavailable: false,
        };
        let sentence = report.sentence();
        assert!(sentence.contains("cargo test"), "{sentence}");
        assert!(sentence.contains("101"), "{sentence}");
    }

    /// The failure a reader acts on is the *tail*: the failing assertion is at the end of a
    /// suite's output, and the head is build noise.
    #[test]
    fn the_tail_keeps_the_end_and_says_it_cut() {
        let long = "x".repeat(5_000);
        let cut = tail(&format!("HEAD-{long}"), 100);
        assert!(
            cut.starts_with("… (earlier output dropped)"),
            "{}",
            &cut[..40]
        );
        assert!(cut.ends_with('x'));
    }

    #[test]
    fn a_short_output_is_left_alone() {
        assert_eq!(tail("short\n", 100), "short");
    }

    /// A cut that split a codepoint would put a replacement character in a continuation
    /// prompt, which is exactly the corruption nobody looks for.
    #[test]
    fn the_tail_is_cut_on_a_character_boundary() {
        let text = "é".repeat(200);
        let cut = tail(&text, 51);
        assert!(!cut.contains('\u{FFFD}'), "the cut split a character");
        assert!(cut.ends_with('é'));
    }

    /// No git, no repository, a detached head: every one must still produce a fingerprint,
    /// because a fingerprint that cannot be computed must not stop a goal.
    #[test]
    fn the_fingerprint_works_without_git() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let fingerprint = workspace_fingerprint(dir.path());
        assert_eq!(fingerprint.len(), 16, "a hex digest: {fingerprint:?}");
    }

    #[test]
    fn an_empty_gate_cannot_ever_pass() {
        let runner = GoalGateRunner::new(None);
        let gate = GoalGate::new("   ");
        let report = block_on(runner.run(&gate, std::path::Path::new(".")));
        assert!(!report.passed);
        assert!(report.unavailable);
        assert!(
            report.sentence().contains("never pass"),
            "{}",
            report.sentence()
        );
    }

    /// `Confinement::Off` is a **choice**, and the gate honours it.
    ///
    /// This is the opposite of the first version, which refused. A refusal there was
    /// backwards: it inverted the shell tool's own rule (refuse when no backend exists,
    /// run when the user asked for none), and it made the feature inert for exactly the
    /// people who opted into running unconfined.
    #[test]
    fn an_unconfined_session_runs_the_gate_rather_than_refusing_to_evidence_it() {
        if std::process::Command::new("cargo")
            .arg("--version")
            .output()
            .is_err()
        {
            return; // cargo is not on PATH here; there is nothing to say about the result
        }
        let runner = GoalGateRunner::new(None);
        let report = block_on(runner.run(&GoalGate::new("cargo --version"), Path::new(".")));
        assert!(report.passed, "{}", report.sentence());
        assert!(!report.unavailable);
    }

    /// Run a future to completion in a test.
    ///
    /// tokio's runtime rather than `futures::executor::block_on`, because a hand-rolled
    /// waker needs `unsafe` and this workspace forbids it outright. Building a runtime per
    /// test is cheap, and these tests do nothing else.
    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a test runtime")
            .block_on(fut)
    }
}
