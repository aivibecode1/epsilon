//! The bash tool: shell execution with timeout, output caps, and spill locators.

use async_trait::async_trait;
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};
use schemars::JsonSchema;
use serde::Deserialize;

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
/// development environment. OS confinement is Layer 2 (M10); until then every call is
/// gated by the permission engine (`bash` defaults to ask).
pub struct BashTool {
    descriptor: ToolDescriptor,
}

impl BashTool {
    /// Build the tool.
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
        }
    }
}

impl Default for BashTool {
    fn default() -> Self {
        Self::new()
    }
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

        let mut cmd = shell_command(&args.command);
        cmd.current_dir(&ctx.cwd);

        let started = std::time::Instant::now();
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
        let mut combined = String::new();
        if !stdout.is_empty() {
            combined.push_str(&stdout);
        }
        if !stderr.is_empty() {
            if !combined.is_empty() && !combined.ends_with('\n') {
                combined.push('\n');
            }
            combined.push_str("[stderr]\n");
            combined.push_str(&stderr);
        }
        let exit_note = match output.status.code() {
            Some(0) => String::new(),
            Some(c) => format!("\n[exit code: {c}]"),
            None => "\n[terminated by signal]".to_string(),
        };
        combined.push_str(&exit_note);
        let duration_ms = started.elapsed().as_millis();

        let t = util::truncate_output(&combined, &ctx.spill_dir, "bash");
        let mut details = serde_json::json!({
            "exit_code": output.status.code(),
            "duration_ms": duration_ms,
            "truncated": t.truncated,
        });
        if let Some(spill) = t.spilled_to {
            details["full_output_path"] = serde_json::Value::String(spill.display().to_string());
        }

        Ok(ToolOutput {
            content: vec![epsilon_contracts::content::ContentBlock::text(t.text)],
            details,
            usage: None,
            // Non-zero exits are information for the model, not run-stoppers.
            terminate: false,
        })
    }
}

fn shell_command(command: &str) -> tokio::process::Command {
    #[cfg(windows)]
    {
        let mut c = tokio::process::Command::new("powershell");
        c.args(["-NoLogo", "-NonInteractive", "-Command", command]);
        c
    }
    #[cfg(not(windows))]
    {
        let mut c = tokio::process::Command::new("bash");
        c.arg("-c").arg(command);
        c
    }
}
