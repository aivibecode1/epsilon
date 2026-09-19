//! The bash tool: shell execution with OS confinement, timeout, output caps, and spill
//! locators.
//!
//! Layer 2 integration (ADR-005): when a [`Sandbox`] backend is attached, every command
//! runs inside it (Windows: Job Object with memory/process/CPU caps; Unix: bubblewrap).
//! Confinement failures are reported as `[confinement]`-marked errors so the model can
//! distinguish "sandbox blocked it" from "the command failed" (DSH denial-signature
//! rule). With confinement explicitly off, execution is direct — trusted automation
//! only.

use async_trait::async_trait;
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};
use epsilon_safety::{ConfinedCommand, ConfinementPolicy, Sandbox, SandboxMode};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;

use crate::util;

/// Arguments for bash.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BashArgs {
    /// Shell command to run in the workspace root.
    pub command: String,
    /// Kill the process after this many seconds (default 120, max 600).
    /// Accepts pi's `timeout` wire name as an alias; zero is rejected (pi
    /// `resolveTimeoutMs` parity) instead of silently clamping.
    #[serde(default, alias = "timeout")]
    pub timeout_secs: Option<u64>,
    /// Ask for a wider sandbox mode than the session's (port plan item 15). Only
    /// `danger-full-access` or a widening needs approval, and it must travel with
    /// `justification`.
    #[serde(default)]
    pub sandbox_permissions: Option<String>,
    /// Why the wider mode is needed — one sentence. Travels with `sandbox_permissions`.
    #[serde(default)]
    pub justification: Option<String>,
}

pub const BASH_DESC: &str = include_str!("../descriptions/bash.txt");

/// Runs commands through the platform shell.
///
/// Platform choice (documented): POSIX uses `bash -c`; Windows uses PowerShell
/// (`-NoLogo -NonInteractive -Command`) because it is always present and matches the
/// development environment.
pub struct BashTool {
    descriptor: ToolDescriptor,
    sandbox: Option<Arc<dyn Sandbox>>,
    policy: ConfinementPolicy,
    /// The mode the session runs at — the ceiling a per-call request may not cross
    /// without approval (port plan item 15). Default `WorkspaceWrite`, which is what
    /// every existing caller means.
    session_mode: SandboxMode,
}

impl BashTool {
    /// Build the tool without a sandbox (confinement off).
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "bash".into(),
                label: "Shell".into(),
                description: BASH_DESC.into(),
                parameters_schema: serde_json::to_value(schemars::schema_for!(BashArgs))
                    .unwrap_or(serde_json::json!({"type":"object"})),
                execution_mode: ExecutionMode::Sequential,
            },
            sandbox: None,
            policy: ConfinementPolicy::default(),
            session_mode: SandboxMode::WorkspaceWrite,
        }
    }

    /// Attach an OS-confinement backend; every call runs confined from now on.
    pub fn with_sandbox(mut self, sandbox: Arc<dyn Sandbox>) -> Self {
        self.sandbox = Some(sandbox);
        self
    }

    /// Override the confinement policy (resource caps / wall limit defaults).
    pub fn with_policy(mut self, policy: ConfinementPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Set the mode the session runs at. Every call may *narrow* below this freely; a call
    /// that asks to widen is refused here (see [`Self::resolve_mode`]).
    pub fn with_session_mode(mut self, mode: SandboxMode) -> Self {
        self.session_mode = mode;
        self
    }

    /// Resolve one call's effective mode from its paired arguments.
    ///
    /// The ladder rule, applied at the last moment before anything spawns: a call may ask
    /// for a narrower or equal mode and gets it; a call that asks for a **wider** one is
    /// refused, because widening is a consent decision and this harness has no per-call
    /// consent channel yet (recorded in the M78 log — borrowing the model's own arguments
    /// for it would be forgeable). A malformed pair is an error, never a default.
    fn resolve_mode(&self, args: &BashArgs) -> Result<SandboxMode, ToolFailure> {
        let requested = epsilon_safety::validate_escalation_args(
            args.sandbox_permissions.as_deref(),
            args.justification.as_deref(),
        )
        .map_err(|e| ToolFailure::msg(format!("[sandbox] {e}")))?;

        let Some(req) = requested else {
            return Ok(self.session_mode);
        };
        if !req.mode.is_strictly_wider_than(self.session_mode) {
            return Ok(req.mode);
        }
        let target = req.mode;
        let refusal = if target == SandboxMode::DangerFullAccess {
            format!(
                "[sandbox] {target} is not available per call: it is the \"no confinement at \
                 all\" tier, which is a decision about the whole session, not one command. \
                 Run it at the current mode, or ask the user to restart with `--sandbox off` \
                 if they want unconfined execution."
            )
        } else {
            format!(
                "[sandbox] {target} is wider than this session's mode ({}); widening needs \
                 approval, which is a user decision.",
                self.session_mode
            )
        };
        Err(ToolFailure::msg(format!(
            "{refusal}\n{}",
            epsilon_safety::escalation_hint(&args.command)
        )))
    }
}

impl Default for BashTool {
    fn default() -> Self {
        Self::new()
    }
}

/// How to run a shell command on this platform.
///
/// # Windows: `-NoProfile` is not an optimisation, it is the difference between
/// usable and broken
///
/// Measured on the development machine:
///
/// | invocation | `echo` round trip |
/// |---|---|
/// | `powershell -Command` (loads the user profile) | **~6,200 ms** |
/// | `powershell -NoProfile -Command` | ~1,060 ms |
/// | `cmd /C` | ~70 ms |
///
/// A user profile that imports a module or runs a package manager turns *every
/// tool call* into a six-second stall. From the TUI that is indistinguishable
/// from a hang: the spinner turns, the model waits, and a one-line `echo` looks
/// like a deadlock. `-NoLogo` silences the banner; `-NoProfile` skips the code
/// the user wrote for their interactive shell, which is not ours to run.
///
/// `-NonInteractive` stays: a command that prompts for input would otherwise
/// wait forever on a pipe nobody is feeding.
///
/// We do not switch to `cmd /C` despite it being ~15x faster, because the tool is
/// named `bash` and documented as a POSIX-style shell; silently giving the model
/// `cmd` syntax would be a different, more confusing surprise than a slow start.
pub fn shell_argv(command: &str) -> (&'static str, Vec<String>) {
    #[cfg(windows)]
    {
        (
            "powershell",
            vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                command.to_string(),
            ],
        )
    }
    #[cfg(not(windows))]
    {
        ("bash", vec!["-c".into(), command.to_string()])
    }
}

/// What a finished command means — three questions, kept apart (M77; DSH postmortem 0004).
///
/// The failure this exists to prevent: deciding "the command failed" from the exit code alone,
/// mixing it with "the harness refused to run it", and letting a tool that exits non-zero on a
/// legitimate empty result (ripgrep exits 1 on *no matches*) read as a confinement refusal. DSH's
/// postmortem records exactly that mislabel; Epsilon reports correctly today, by construction
/// rather than by accident, and this type is that construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandVerdict {
    /// The command ran and exited with status 0.
    Success,
    /// The command ran and exited non-zero. A *command* outcome: the model decides what to make
    /// of it, and nothing here calls it a confinement failure.
    Exited(i32),
    /// The command ran and was killed by a signal.
    Signalled,
}

impl CommandVerdict {
    /// Stable string for `details` and for anything that wants to branch on it.
    pub fn as_str(&self) -> String {
        match self {
            CommandVerdict::Success => "success".to_string(),
            CommandVerdict::Exited(code) => format!("exited:{code}"),
            CommandVerdict::Signalled => "signalled".to_string(),
        }
    }

    /// Whether this is a clean run.
    pub fn is_success(&self) -> bool {
        matches!(self, CommandVerdict::Success)
    }
}

/// Classify a *completed* run.
///
/// Only completed runs reach here: a runner failure (setup, spawn, timeout, refusal) is a
/// [`epsilon_safety::SandboxError`] and never produces an outcome — which is what keeps the third
/// question ("did the sandbox refuse?") answerable by *where the answer came from*, not by
/// pattern-matching output text.
fn command_verdict(outcome: &epsilon_safety::ConfinedOutcome) -> CommandVerdict {
    match outcome.exit_code {
        Some(0) => CommandVerdict::Success,
        Some(code) => CommandVerdict::Exited(code),
        None => CommandVerdict::Signalled,
    }
}

fn combine_output(stdout: &str, stderr: &str, exit_code: Option<i32>) -> String {
    let mut combined = String::new();
    if !stdout.is_empty() {
        combined.push_str(stdout);
    }
    if !stderr.is_empty() {
        if !combined.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str("[stderr]\n");
        combined.push_str(stderr);
    }
    match exit_code {
        Some(0) => {}
        Some(c) => combined.push_str(&format!("\n[exit code: {c}]")),
        None => combined.push_str("\n[terminated by signal]"),
    }
    combined
}

#[async_trait]
impl Tool for BashTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let args: BashArgs = serde_json::from_value(call.args)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'bash': {e}")))?;
        // Pi parity (`resolveTimeoutMs`): a zero timeout is a validation error, not a
        // silent clamp to 1s. The 600s upper cap and 120s default are intentional
        // Epsilon hardenings (pi caps at i32::MAX ms — no cap in practice — and has
        // no default); see the design-decisions amendment.
        let timeout = match args.timeout_secs {
            Some(0) => {
                return Err(ToolFailure::msg(
                    "invalid timeout: must be a positive number of seconds",
                ));
            }
            Some(s) => s.min(600),
            None => 120,
        };
        let started = std::time::Instant::now();
        // Live view of the running command (throttled, bounded tail). Both paths
        // below stream into it so long builds are visible while they run.
        let live = util::LiveOutput::new(on_update);
        // One mode per call, resolved before anything spawns (port plan item 15).
        let mode = self.resolve_mode(&args)?;

        // ---- confined path ---------------------------------------------------------
        if let Some(sandbox) = self.sandbox.as_ref() {
            if !sandbox.available() {
                // Fail closed (DSH rule): never fall back to unconfined execution.
                return Err(ToolFailure::msg(format!(
                    "[confinement] backend '{}' is unavailable on this platform; \
                     refusing to run unconfined. Ask the user how to proceed.",
                    sandbox.name()
                )));
            }
            let (program, argv) = shell_argv(&args.command);
            let mut policy = self.policy.clone();
            policy.wall_timeout_secs = timeout;

            let outcome = {
                let run_fut = sandbox.run(
                    ConfinedCommand {
                        program: program.into(),
                        args: argv,
                        cwd: ctx.cwd.clone(),
                        mode,
                    },
                    &policy,
                    Some(live.sink()),
                );
                // Cancellation (M12): dropping the in-flight run future drops the
                // sandbox's job guard, which terminates the whole tree.
                tokio::select! {
                    r = run_fut => r,
                    _ = ctx.token.cancelled() => Err(epsilon_safety::SandboxError::Setup(
                        "cancelled by user".into(),
                    )),
                }
            };

            return match outcome {
                Ok(o) => {
                    let combined = combine_output(&o.stdout, &o.stderr, o.exit_code);
                    let duration_ms = started.elapsed().as_millis();
                    let t = util::truncate_output(&combined, &ctx.spill_dir, "bash");
                    let verdict = command_verdict(&o);
                    let mut details = serde_json::json!({
                        "exit_code": o.exit_code,
                        "duration_ms": duration_ms,
                        "truncated": t.truncated,
                        "confinement": o.backend,
                        "mode": mode.as_str(),
                        // M77: the outcome says how completely the boundary held, so a caller
                        // that needs an absolute boundary can reject a partial one instead of
                        // reading a report paragraph.
                        "enforcement": o.enforcement.describe(),
                        // M77: the three questions, answered separately.
                        "verdict": verdict.as_str(),
                    });
                    if let Some(spill) = t.spilled_to {
                        details["full_output_path"] =
                            serde_json::Value::String(spill.display().to_string());
                    }
                    Ok(ToolOutput {
                        content: vec![epsilon_contracts::content::ContentBlock::text(t.text)],
                        details,
                        usage: None,
                        // Non-zero exits are information for the model, not run-stoppers.
                        terminate: false,
                    })
                }
                Err(epsilon_safety::SandboxError::Timeout { seconds }) => {
                    Err(ToolFailure::msg(format!(
                        "command timed out after {seconds}s and was killed (job terminated): {}",
                        args.command
                    )))
                }
                Err(e) => Err(ToolFailure::msg(format!(
                    "[confinement] {e}. The command was NOT executed."
                ))),
            };
        }

        // ---- unconfined path (explicit opt-out; tests / trusted automation) --------
        // A request for a narrower mode cannot be honored here: with confinement off, saying
        // "read-only" would describe a boundary that is not there. Refuse rather than mislead.
        if mode != SandboxMode::DangerFullAccess && args.sandbox_permissions.is_some() {
            return Err(ToolFailure::msg(format!(
                "[confinement] this session runs with confinement OFF, so a {mode} request \
                 cannot be honored — the command would run unconfined whatever it said. \
                 Restart with `--sandbox auto` (or `--sandbox read-only`) to get a boundary."
            )));
        }
        let (program, argv) = shell_argv(&args.command);
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(&argv).current_dir(&ctx.cwd);
        // DSH subprocess rule: never hand a child the harness's credentials. One
        // `env` in a tool result would put every API key into the transcript, the
        // session JSONL and any export of it.
        epsilon_safety::scrub_secret_env(&mut cmd);
        // Piped + drained incrementally rather than `output()`: a build that runs
        // for ten minutes should show its progress, and a child that fills a pipe
        // buffer must never block on a parent that is waiting for exit.
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| ToolFailure::msg(format!("spawn failed: {e}")))?;
        let stdout_pipe = child
            .stdout
            .take()
            .ok_or_else(|| ToolFailure::msg("stdout pipe unavailable".to_string()))?;
        let stderr_pipe = child
            .stderr
            .take()
            .ok_or_else(|| ToolFailure::msg("stderr pipe unavailable".to_string()))?;
        let sink = live.sink();
        let (out_sink, err_sink) = (Some(sink.clone()), Some(sink));
        let out_task =
            tokio::spawn(async move { epsilon_safety::drain_pipe(stdout_pipe, out_sink).await });
        let err_task =
            tokio::spawn(async move { epsilon_safety::drain_pipe(stderr_pipe, err_sink).await });

        let wait = tokio::time::timeout(std::time::Duration::from_secs(timeout), child.wait());
        let status = tokio::select! {
            r = wait => match r {
                Ok(Ok(status)) => status,
                Ok(Err(e)) => return Err(ToolFailure::msg(format!("wait failed: {e}"))),
                Err(_) => {
                    let _ = child.kill().await;
                    return Err(ToolFailure::msg(format!(
                        "command timed out after {timeout}s and was killed: {}",
                        args.command
                    )));
                }
            },
            _ = ctx.token.cancelled() => {
                let _ = child.kill().await;
                return Err(ToolFailure::msg(format!(
                    "[aborted] command cancelled by user before completion: {}",
                    args.command
                )));
            }
        };
        let stdout_bytes = out_task.await.unwrap_or_default();
        let stderr_bytes = err_task.await.unwrap_or_default();
        let stdout = String::from_utf8_lossy(&stdout_bytes);
        let stderr = String::from_utf8_lossy(&stderr_bytes);
        let combined = combine_output(&stdout, &stderr, status.code());
        let duration_ms = started.elapsed().as_millis();

        let t = util::truncate_output(&combined, &ctx.spill_dir, "bash");
        // Same vocabulary on the opt-out path: a caller branching on `verdict` should not have
        // to know which path produced the result.
        let verdict = match status.code() {
            Some(0) => CommandVerdict::Success,
            Some(code) => CommandVerdict::Exited(code),
            None => CommandVerdict::Signalled,
        };
        let mut details = serde_json::json!({
            "exit_code": status.code(),
            "duration_ms": duration_ms,
            "truncated": t.truncated,
            "confinement": "off",
            "enforcement": "none — confinement is switched off for this call",
            "mode": mode.as_str(),
            "verdict": verdict.as_str(),
        });
        if let Some(spill) = t.spilled_to {
            details["full_output_path"] = serde_json::Value::String(spill.display().to_string());
        }

        Ok(ToolOutput {
            content: vec![epsilon_contracts::content::ContentBlock::text(t.text)],
            details,
            usage: None,
            terminate: false,
        })
    }
}

#[cfg(test)]
mod shell_argv_tests {
    use super::shell_argv;

    #[cfg(windows)]
    #[test]
    fn the_windows_shell_skips_the_user_profile() {
        // Without `-NoProfile`, every tool call runs the user's interactive shell
        // setup. Measured on the development machine: ~6,200 ms for `echo` with
        // the profile, ~1,060 ms without, ~70 ms for `cmd /C`. From the TUI a
        // six-second stall per call is indistinguishable from a hang, and the
        // spinner turning the whole time makes it look like one. This assertion
        // is the only thing standing between that and a silent regression.
        let (program, args) = shell_argv("echo hi");
        assert_eq!(program, "powershell");
        assert!(
            args.iter().any(|a| a == "-NoProfile"),
            "the user's profile is not ours to run, and running it stalls every call: {args:?}"
        );
        assert!(
            args.iter().any(|a| a == "-NonInteractive"),
            "an interactive prompt would wait forever on a pipe: {args:?}"
        );
        assert_eq!(args.last().expect("the command is last"), "echo hi");
    }

    #[cfg(not(windows))]
    #[test]
    fn the_posix_shell_is_bash_dash_c() {
        let (program, args) = shell_argv("echo hi");
        assert_eq!(program, "bash");
        assert_eq!(args, vec!["-c".to_string(), "echo hi".to_string()]);
    }
}

#[cfg(test)]
mod verdict_tests {
    use super::*;
    use epsilon_safety::{ConfinedOutcome, Enforcement};

    fn outcome(exit_code: Option<i32>) -> ConfinedOutcome {
        ConfinedOutcome {
            exit_code,
            stdout: String::new(),
            stderr: String::new(),
            backend: "test",
            enforcement: Enforcement::from_missing(vec!["filesystem"]),
        }
    }

    #[test]
    fn a_non_zero_exit_is_a_command_outcome_never_a_confinement_one() {
        // The DSH postmortem case: `rg` exits 1 when it finds nothing. That is a *command*
        // result the model should read, not a refused call, and the only way to tell them apart
        // is that a refusal never produces an outcome at all.
        let v = command_verdict(&outcome(Some(1)));
        assert_eq!(v, CommandVerdict::Exited(1));
        assert!(!v.is_success(), "a non-zero exit is not a clean run");
        assert_eq!(v.as_str(), "exited:1");
        assert!(!v.as_str().contains("confinement"), "{v:?}");
    }

    #[test]
    fn zero_and_signals_are_their_own_answers() {
        assert!(command_verdict(&outcome(Some(0))).is_success());
        assert_eq!(command_verdict(&outcome(Some(0))).as_str(), "success");
        assert_eq!(command_verdict(&outcome(None)), CommandVerdict::Signalled);
        assert_eq!(command_verdict(&outcome(None)).as_str(), "signalled");
    }

    #[test]
    fn the_enforcement_vocabulary_cannot_be_empty_partial() {
        // A backend that could not confine nothing must be `Full`, not a `Partial` with no
        // names — an unanswerable state a report would have to invent a sentence for.
        assert_eq!(Enforcement::from_missing(vec![]), Enforcement::Full);
        let partial = Enforcement::from_missing(vec!["filesystem", "network"]);
        assert!(!partial.is_full());
        assert_eq!(partial.missing(), ["filesystem", "network"]);
        assert!(partial.describe().contains("filesystem, network"));
        assert_eq!(Enforcement::Full.describe(), "full");
    }
}
