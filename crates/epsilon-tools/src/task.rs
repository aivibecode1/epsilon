//! The task tool: model-initiated subagent delegation (pi/opencode `task` parity).
//!
//! G4 (2026-08-28): the harness advertises `todo` as a checklist but had no
//! delegation primitive — the model could plan work but not fan it out. pi,
//! opencode, and DSH all expose a `task` tool so the model can delegate a
//! focused prompt to a fresh subagent context. This crate now exposes the
//! same surface so the provider schema advertises `task` and the loop can
//! gate on capability `task`.
//!
//! v1 executes as a *structured capture*: the tool validates the delegation
//! request, enforces the same caps the prompt advertises (description 3-5
//! words, prompt is the full context), and returns a `<task>` envelope the
//! model can treat as the subagent's result. A real isolated Agent run
//! (child session, filtered toolset, permission derivation) is the next
//! ladder rung — wiring it requires an `Agent` handle inside `ToolContext`,
//! which would widen the contract crate. Shipping the surface first keeps
//! the diff ponytail-small and honest: the tool is present, gated, and
//! observable, and the model stops seeing "Tool 'task' not found".
//!
//! Rung 2 (contracts inversion): when the loop injects a `TaskHost` via
//! `ToolContext::task_host`, this tool delegates to an isolated child `Agent`
//! and wraps its transcript in the same `<task>` envelope; otherwise it
//! falls back to the capture envelope so direct callers/tests stay honest.

use async_trait::async_trait;
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};
use schemars::JsonSchema;
use serde::Deserialize;

/// Hard caps so one pathological call cannot flood the context.
const MAX_DESCRIPTION_CHARS: usize = 80;
const MAX_PROMPT_CHARS: usize = 24_000;
const MAX_SUBAGENT_TYPE_CHARS: usize = 64;

#[derive(Debug, Deserialize, JsonSchema)]
/// Arguments for the `task` subagent delegation tool.
pub struct TaskArgs {
    /// Short 3-5 word description of the task. Required to start or resume
    /// work; omit it together with the prompt for a status-only probe.
    #[serde(default)]
    pub description: String,
    /// Complete prompt for the subagent — must contain all context because the
    /// subagent starts fresh. Omit it together with `task_id` to ask for a
    /// status report instead of running anything.
    #[serde(default)]
    pub prompt: String,
    /// Optional subagent flavor (e.g. `explore`, `plan`). Defaults to `default`.
    #[serde(default)]
    pub subagent_type: Option<String>,
    /// Address an existing task: with a prompt it resumes that child (its
    /// transcript is intact), alone it reports the task's state.
    #[serde(default)]
    pub task_id: Option<String>,
    /// Run the child detached: the call returns as soon as it starts, the
    /// result arrives later as its own message. Default `false` (await it).
    #[serde(default)]
    pub background: Option<bool>,
    /// Model the subagent runs on (catalog id, e.g. `deepseek-flash`). Omit to
    /// inherit this session's model; an agent type that defines its own model
    /// overrides this. The result names the model that actually ran.
    #[serde(default)]
    pub model: Option<String>,
    /// The command that triggered this task (opencode parity, optional).
    #[serde(default)]
    pub command: Option<String>,
}

pub const TASK_DESC: &str = include_str!("../descriptions/task.txt");

/// Model-delegatable subagent tool (pi/opencode `task` parity, capture v1).
pub struct TaskTool {
    descriptor: ToolDescriptor,
}

impl TaskTool {
    /// Build a `task` tool descriptor.
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "task".into(),
                label: "Task".into(),
                description: TASK_DESC.into(),
                parameters_schema: serde_json::to_value(schemars::schema_for!(TaskArgs))
                    .unwrap_or(serde_json::json!({"type":"object"})),
                // A task fans out work — barrier semantics match pi's subagent
                // queueing and prevent interleaving of parallel-safe calls
                // around a delegation point.
                execution_mode: ExecutionMode::Sequential,
            },
        }
    }
}

impl Default for TaskTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TaskTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        if ctx.token.is_cancelled() {
            return Err(ToolFailure::msg("[aborted] task cancelled before start"));
        }

        let args: TaskArgs = serde_json::from_value(call.args)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'task': {e}")))?;

        let task_id = args
            .task_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let prompt = args.prompt.trim().to_string();
        let description = args.description.trim().to_string();
        // M53: `task_id` with no prompt is a status check — the work-request
        // caps and required fields only apply when the call asks for work.
        let status_only = task_id.is_some() && prompt.is_empty();
        if !status_only {
            if description.is_empty() {
                return Err(ToolFailure::msg(
                    "task: description is required (3-5 words)",
                ));
            }
            if description.chars().count() > MAX_DESCRIPTION_CHARS {
                return Err(ToolFailure::msg(format!(
                    "task: description must stay under {MAX_DESCRIPTION_CHARS} characters"
                )));
            }
            if prompt.is_empty() {
                return Err(ToolFailure::msg(
                    "task: prompt is required — provide the full subagent prompt with context (pass task_id alone for a status check)",
                ));
            }
            if prompt.chars().count() > MAX_PROMPT_CHARS {
                return Err(ToolFailure::msg(format!(
                    "task: prompt must stay under {MAX_PROMPT_CHARS} characters"
                )));
            }
        }
        let subagent_type = args
            .subagent_type
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("default");
        if subagent_type.chars().count() > MAX_SUBAGENT_TYPE_CHARS {
            return Err(ToolFailure::msg(
                "task: subagent_type must stay under 64 characters",
            ));
        }

        let model = args
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let request = epsilon_contracts::tool::TaskRequest {
            prompt: prompt.clone(),
            description: if description.is_empty() {
                "status".to_string()
            } else {
                description.clone()
            },
            subagent_type: subagent_type.to_string(),
            background: args.background.unwrap_or(false),
            resume: task_id.clone(),
            model,
        };

        // M53: the loop injects an isolated host whose registry keeps children
        // alive between calls, so this branch handles foreground runs,
        // background starts, resumptions, and status reports alike.
        if let Some(host) = ctx.task_host.clone() {
            if ctx.token.is_cancelled() {
                return Err(ToolFailure::msg(
                    "[aborted] task cancelled before delegation",
                ));
            }
            use epsilon_contracts::tool::{TaskOutcome, TaskState};
            return match host.run_task(request).await {
                Ok(TaskOutcome::Completed(res)) => {
                    // M56: report the model the child actually ran on (the
                    // child_details carry the resolved id) so the caller can
                    // tell a delegated scout from its own model.
                    let ran_on = res
                        .details
                        .get("model")
                        .and_then(|v| v.as_str())
                        .filter(|m| !m.is_empty());
                    let model_line = ran_on
                        .map(|m| format!("\n<model>{m}</model>"))
                        .unwrap_or_default();
                    let envelope = format!(
                        "<task id=\"{}\" state=\"completed\">\n<description>{}</description>\n<subagent_type>{}</subagent_type>{}\n<task_result>\n{}\n</task_result>\n</task>",
                        res.details
                            .get("task_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&call.tool_call_id),
                        description,
                        subagent_type,
                        model_line,
                        res.text
                    );
                    Ok(ToolOutput {
                        content: vec![epsilon_contracts::content::ContentBlock::text(envelope)],
                        details: serde_json::json!({
                            "description": description,
                            "subagent_type": subagent_type,
                            "model": ran_on,
                            "prompt_chars": prompt.chars().count(),
                            "task_id": res.details.get("task_id").cloned(),
                            "host": "isolated",
                            "child_details": res.details,
                        }),
                        usage: res.usage,
                        terminate: false,
                    })
                }
                Ok(TaskOutcome::Running(snapshot)) => {
                    let state = match snapshot.state {
                        TaskState::Running => "running",
                        TaskState::Completed => "completed",
                        TaskState::Failed => "failed",
                    };
                    let model_line = snapshot
                        .model
                        .as_deref()
                        .map(|m| format!("\n<model>{m}</model>"))
                        .unwrap_or_default();
                    let envelope = if snapshot.state == TaskState::Running {
                        format!(
                            "<task id=\"{}\" state=\"running\">\n<description>{}</description>\n<subagent_type>{}</subagent_type>{}\n<note>Started in the background. Its result arrives as its own message when it finishes; check progress with task_id \"{}\" (no prompt), or resume it with a new prompt once it stops.</note>\n</task>",
                            snapshot.id,
                            snapshot.description,
                            snapshot.subagent_type,
                            model_line,
                            snapshot.id
                        )
                    } else {
                        format!(
                            "<task id=\"{}\" state=\"{state}\">\n<description>{}</description>\n<subagent_type>{}</subagent_type>{}\n<task_result>\n{}\n</task_result>{}\n</task>",
                            snapshot.id,
                            snapshot.description,
                            snapshot.subagent_type,
                            model_line,
                            snapshot.output,
                            if snapshot.turns > 0 {
                                format!("\n<turns>{}</turns>", snapshot.turns)
                            } else {
                                String::new()
                            }
                        )
                    };
                    Ok(ToolOutput {
                        content: vec![epsilon_contracts::content::ContentBlock::text(envelope)],
                        details: serde_json::json!({
                            "description": snapshot.description,
                            "subagent_type": snapshot.subagent_type,
                            "model": snapshot.model,
                            "task_id": snapshot.id,
                            "state": state,
                            "turns": snapshot.turns,
                            "host": "isolated",
                        }),
                        usage: snapshot.usage,
                        terminate: false,
                    })
                }
                Err(e) => Err(ToolFailure::msg(format!("task: {e}"))),
            };
        }

        let envelope = format!(
            "<task id=\"{}\" state=\"capture\">\n<description>{}</description>\n<subagent_type>{}</subagent_type>\n<task_result>\n{prompt}\n</task_result>\n<note>Subagent host not wired in this context — prompt captured for inline execution. Continue the work directly; when run via the loop the task fans out to an isolated child Agent.</note>\n</task>",
            call.tool_call_id, description, subagent_type
        );

        Ok(ToolOutput {
            content: vec![epsilon_contracts::content::ContentBlock::text(envelope)],
            details: serde_json::json!({
                "description": description,
                "subagent_type": subagent_type,
                "prompt_chars": prompt.chars().count(),
                "task_id": task_id,
                "host": "capture-v1",
                "state": "capture",
            }),
            usage: None,
            terminate: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epsilon_contracts::content::ContentBlock;
    use std::sync::Arc;

    async fn run(args: serde_json::Value) -> Result<ToolOutput, ToolFailure> {
        let ctx = ToolContext {
            session_id: "t".into(),
            observation_scope: std::sync::Arc::new(()),
            cwd: std::env::temp_dir(),
            spill_dir: std::env::temp_dir(),
            token: Default::default(),
            task_host: None,
            goal_host: None,
        };
        TaskTool::new()
            .execute(
                ToolInvocation {
                    tool_call_id: "tc-task-1".into(),
                    args,
                },
                ctx,
                Arc::new(|_| {}),
            )
            .await
    }

    #[tokio::test]
    async fn capture_fallback_does_not_claim_success() {
        let out = run(serde_json::json!({
            "description": "explore auth flow",
            "prompt": "Read crates/epsilon-tui/src/app.rs and summarize."
        }))
        .await
        .unwrap();
        let text = text_of(&out);
        assert!(text.contains("state=\"capture\""), "{text}");
        assert!(!text.contains("state=\"completed\""), "{text}");
    }

    #[tokio::test]
    async fn renders_envelope_with_description_and_prompt() {
        let out = run(serde_json::json!({
            "description": "explore auth flow",
            "prompt": "Read crates/epsilon-tui/src/app.rs and summarize the login picker flow. Return the file paths you inspected and the answer in 5 lines."
        }))
        .await
        .unwrap();
        let text = match &out.content[0] {
            ContentBlock::Text { text } => text.clone(),
            _ => panic!("text expected"),
        };
        assert!(text.contains("<task"));
        assert!(text.contains("explore auth flow"));
        assert!(text.contains("login picker"));
        assert!(text.contains("state=\"capture\""), "{text}");
        assert!(!text.contains("state=\"completed\""), "{text}");
        assert_eq!(out.details["description"], "explore auth flow");
        assert_eq!(out.details["host"], "capture-v1");
        assert_eq!(out.details["state"], "capture");
    }

    #[tokio::test]
    async fn rejects_empty_description_and_prompt() {
        let err = run(serde_json::json!({"description": "  ", "prompt": "hello"}))
            .await
            .unwrap_err();
        assert!(err.message.contains("description is required"));

        let err = run(serde_json::json!({"description": "fix bug", "prompt": "  "}))
            .await
            .unwrap_err();
        assert!(err.message.contains("prompt is required"));
    }

    #[tokio::test]
    async fn background_true_starts_the_child_and_reports_running() {
        // M53: the call returns as soon as the child starts; the envelope
        // carries the task id so the model can poll or resume it later.
        let host = std::sync::Arc::new(RecordingHost::default());
        let out = run_with_host(
            serde_json::json!({
                "description": "background job",
                "prompt": "do work",
                "background": true
            }),
            std::sync::Arc::clone(&host),
        )
        .await
        .unwrap();
        let text = text_of(&out);
        assert!(text.contains("state=\"running\""), "{text}");
        assert!(text.contains("task-7"));
        assert_eq!(out.details["state"], "running");
        assert_eq!(out.details["task_id"], "task-7");
        assert!(host.seen.lock().unwrap()[0].background, "flag forwarded");
    }

    #[tokio::test]
    async fn task_id_alone_is_a_status_probe_not_a_new_run() {
        let host = std::sync::Arc::new(RecordingHost {
            snapshot: Some(epsilon_contracts::tool::TaskState::Completed),
            ..Default::default()
        });
        let out = run_with_host(
            serde_json::json!({"task_id": "task-9"}),
            std::sync::Arc::clone(&host),
        )
        .await
        .unwrap();
        let text = text_of(&out);
        assert!(text.contains("state=\"completed\""), "{text}");
        assert!(text.contains("output so far"));
        let seen = host.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].resume.as_deref(), Some("task-9"));
        assert!(seen[0].prompt.is_empty(), "status probe carries no prompt");
        assert!(!seen[0].background);
    }

    #[tokio::test]
    async fn resume_with_a_prompt_forwards_the_task_id() {
        let host = std::sync::Arc::new(RecordingHost::default());
        run_with_host(
            serde_json::json!({
                "description": "more digging",
                "prompt": "now check the session store too",
                "task_id": "task-3"
            }),
            std::sync::Arc::clone(&host),
        )
        .await
        .unwrap();
        let seen = host.seen.lock().unwrap();
        assert_eq!(seen[0].resume.as_deref(), Some("task-3"));
        assert_eq!(seen[0].prompt, "now check the session store too");
    }

    #[tokio::test]
    async fn unknown_task_id_error_reaches_the_model() {
        let host = std::sync::Arc::new(RecordingHost {
            fail_with: Some("unknown task id 'task-nope' — tracked: task-1".into()),
            ..Default::default()
        });
        let err = run_with_host(
            serde_json::json!({"task_id": "task-nope"}),
            std::sync::Arc::clone(&host),
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("unknown task id"), "{}", err.message);
        assert!(err.message.contains("tracked: task-1"));
    }

    #[tokio::test]
    async fn defaults_subagent_type_to_default() {
        let out = run(serde_json::json!({"description": "write docs", "prompt": "draft README"}))
            .await
            .unwrap();
        assert_eq!(out.details["subagent_type"], "default");
        assert_eq!(out.details["state"], "capture");
        assert!(text_of(&out).contains("state=\"capture\""));
    }

    /// Scripted host recording each delegated request (M53 shape).
    #[derive(Default)]
    struct RecordingHost {
        seen: std::sync::Mutex<Vec<epsilon_contracts::tool::TaskRequest>>,
        fail_with: Option<String>,
        /// When set, `task_snapshot` answers with this state.
        snapshot: Option<epsilon_contracts::tool::TaskState>,
    }

    fn text_of(out: &ToolOutput) -> String {
        match &out.content[0] {
            ContentBlock::Text { text } => text.clone(),
            _ => panic!("text expected"),
        }
    }

    #[async_trait]
    impl epsilon_contracts::tool::TaskHost for RecordingHost {
        async fn run_task(
            &self,
            request: epsilon_contracts::tool::TaskRequest,
        ) -> Result<epsilon_contracts::tool::TaskOutcome, String> {
            if let Some(e) = &self.fail_with {
                return Err(e.clone());
            }
            let background = request.background;
            let probe = request.prompt.is_empty() && request.resume.is_some();
            let (description, subagent_type) =
                (request.description.clone(), request.subagent_type.clone());
            let resume_id = request.resume.clone();
            let request_model = request.model.clone();
            self.seen.lock().unwrap().push(request);
            if background || probe {
                // Background start → Running; status probe → the scripted
                // snapshot (a state + output the tool must render).
                let state = if probe {
                    self.snapshot
                        .clone()
                        .unwrap_or(epsilon_contracts::tool::TaskState::Running)
                } else {
                    epsilon_contracts::tool::TaskState::Running
                };
                return Ok(epsilon_contracts::tool::TaskOutcome::Running(
                    epsilon_contracts::tool::TaskSnapshot {
                        id: resume_id.unwrap_or_else(|| "task-7".into()),
                        description,
                        subagent_type,
                        model: request_model,
                        state,
                        output: if probe {
                            "output so far".into()
                        } else {
                            String::new()
                        },
                        turns: if probe { 2 } else { 0 },
                        usage: None,
                    },
                ));
            }
            Ok(epsilon_contracts::tool::TaskOutcome::Completed(
                epsilon_contracts::tool::TaskHostResult {
                    text: "child says hi".into(),
                    usage: None,
                    // Core reports the resolved model in `details`; mirror that
                    // so the tool's rendering path is exercised.
                    details: serde_json::json!({
                        "produced": 1,
                        "task_id": "task-7",
                        "model": request_model,
                    }),
                },
            ))
        }

        fn task_snapshot(&self, id: &str) -> Option<epsilon_contracts::tool::TaskSnapshot> {
            self.snapshot
                .as_ref()
                .map(|state| epsilon_contracts::tool::TaskSnapshot {
                    id: id.to_string(),
                    description: "status probe".into(),
                    subagent_type: "default".into(),
                    model: None,
                    state: state.clone(),
                    output: "output so far".into(),
                    turns: 2,
                    usage: None,
                })
        }
    }

    async fn run_with_host(
        args: serde_json::Value,
        host: std::sync::Arc<RecordingHost>,
    ) -> Result<ToolOutput, ToolFailure> {
        let ctx = ToolContext {
            session_id: "t".into(),
            observation_scope: std::sync::Arc::new(()),
            cwd: std::env::temp_dir(),
            spill_dir: std::env::temp_dir(),
            token: Default::default(),
            task_host: Some(host as std::sync::Arc<dyn epsilon_contracts::tool::TaskHost>),
            goal_host: None,
        };
        TaskTool::new()
            .execute(
                ToolInvocation {
                    tool_call_id: "tc-task-1".into(),
                    args,
                },
                ctx,
                Arc::new(|_| {}),
            )
            .await
    }

    #[tokio::test]
    async fn isolated_host_receives_subagent_type_verbatim() {
        let host = std::sync::Arc::new(RecordingHost::default());
        let out = run_with_host(
            serde_json::json!({"description": "scout code", "prompt": "look around", "subagent_type": "scout"}),
            Arc::clone(&host),
        )
        .await
        .unwrap();
        assert_eq!(out.details["host"], "isolated");
        assert_eq!(out.details["subagent_type"], "scout");
        let seen = host.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        // The type passes through untouched: routing (unknown-type refusal,
        // def lookup) lives in the host, not the tool — the tool never
        // second-guesses or normalizes it.
        assert_eq!(seen[0].subagent_type, "scout");
    }

    #[tokio::test]
    async fn isolated_host_failure_surfaces_naming_type() {
        // M50: nested-delegation refusal (or unknown-type refusal) arrives as
        // a tool failure carrying the host's message — the model sees WHY the
        // fan-out didn't happen (opencode: failures carry the reason).
        let host = std::sync::Arc::new(RecordingHost {
            fail_with: Some(
                "subagent depth limit reached (1). Increase the limit to allow nested subagents."
                    .into(),
            ),
            ..Default::default()
        });
        let err = run_with_host(
            serde_json::json!({"description": "nested work", "prompt": "go deeper"}),
            host,
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("depth limit"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn model_parameter_reaches_the_host_and_the_result_names_it() {
        // M56: the delegation's model is the caller's cost lever — it must
        // travel in the request and come back in the report.
        let host = std::sync::Arc::new(RecordingHost::default());
        let out = run_with_host(
            serde_json::json!({
                "description": "cheap scout",
                "prompt": "List the files under crates/epsilon-llm/src and return the list.",
                "model": "deepseek-flash",
            }),
            host.clone(),
        )
        .await
        .unwrap();
        let seen = host.seen.lock().unwrap();
        assert_eq!(seen[0].model.as_deref(), Some("deepseek-flash"));
        drop(seen);
        let text = text_of(&out);
        assert!(text.contains("<model>deepseek-flash</model>"), "{text}");
        assert_eq!(out.details["model"], "deepseek-flash");
    }

    #[tokio::test]
    async fn model_is_optional_and_advertised_in_the_schema() {
        // Omitted model = inherit; the schema still advertises the parameter so
        // the model can discover the lever.
        let host = std::sync::Arc::new(RecordingHost::default());
        let _ = run_with_host(
            serde_json::json!({"description": "no model", "prompt": "do the thing"}),
            host.clone(),
        )
        .await
        .unwrap();
        assert_eq!(host.seen.lock().unwrap()[0].model, None);
        let schema =
            serde_json::to_string(&TaskTool::new().descriptor().parameters_schema).unwrap();
        assert!(
            schema.contains("\"model\""),
            "schema advertises model: {schema}"
        );
    }
}
