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
use epsilon_safety::{ConfinedCommand, ConfinementPolicy, Sandbox};
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
    #[serde(default)]
    pub timeout_secs: Option<u64>,
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
}

impl Default for BashTool {
    fn default() -> Self {
        Self::new()
    }
}

fn shell_argv(command: &str) -> (&'static str, Vec<String>) {
    #[cfg(windows)]
    {
        (
            "powershell",
            vec![
                "-NoLogo".into(),
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
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let args: BashArgs = serde_json::from_value(call.args)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'bash': {e}")))?;
        let timeout = args.timeout_secs.unwrap_or(120).clamp(1, 600);
        let started = std::time::Instant::now();

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

            let outcome = sandbox
                .run(
                    ConfinedCommand {
                        program: program.into(),
                        args: argv,
                        cwd: ctx.cwd.clone(),
                    },
                    &policy,
                )
                .await;

            return match outcome {
                Ok(o) => {
                    let combined = combine_output(&o.stdout, &o.stderr, o.exit_code);
                    let duration_ms = started.elapsed().as_millis();
                    let t = util::truncate_output(&combined, &ctx.spill_dir, "bash");
                    let mut details = serde_json::json!({
                        "exit_code": o.exit_code,
                        "duration_ms": duration_ms,
                        "truncated": t.truncated,
                        "confinement": o.backend,
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
        let (program, argv) = shell_argv(&args.command);
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(&argv).current_dir(&ctx.cwd);

        let future = cmd.output();
        let output = tokio::time::timeout(std::time::Duration::from_secs(timeout), future)
            .await
            .map_err(|_| {
                ToolFailure::msg(format!(
                    "command timed out after {timeout}s and was killed: {}",
                    args.command
                ))
            })?
            .map_err(|e| ToolFailure::msg(format!("spawn failed: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = combine_output(&stdout, &stderr, output.status.code());
        let duration_ms = started.elapsed().as_millis();

        let t = util::truncate_output(&combined, &ctx.spill_dir, "bash");
        let mut details = serde_json::json!({
            "exit_code": output.status.code(),
            "duration_ms": duration_ms,
            "truncated": t.truncated,
            "confinement": "off",
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
