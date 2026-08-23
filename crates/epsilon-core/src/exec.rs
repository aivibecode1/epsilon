//! Tool batch execution: bounded parallel pool, sequential barriers, doom-loop brake.
//!
//! Semantics (ported + improved):
//! - preflight every call sequentially (resolve, validate, gate) — pi's ordering;
//! - any `Sequential` tool in the batch makes the whole batch a barrier — all-or-nothing
//!   ordering is easier to reason about than partial serialization (Epsilon simplification
//!   of pi's per-call mode, documented in ADR-002);
//! - parallel-safe calls run on a semaphore-bounded pool; results commit in MODEL order
//!   regardless of completion order (DSH rule) so transcripts stay replay-stable.

use std::sync::Arc;

use epsilon_contracts::agent_event::{AgentEvent, PermissionRequest, TurnEndReason};
use epsilon_contracts::content::ContentBlock;
use epsilon_contracts::ids::new_id;
use epsilon_contracts::message::{AssistantMessage, ToolResultMessage};
use epsilon_contracts::tool::{ToolContext, ToolFailure, ToolInvocation, ToolOutput};

use crate::{Agent, RunError};

/// Result of one executed batch.
pub struct BatchOutcome {
    /// Tool results in model order.
    pub results: Vec<ToolResultMessage>,
    /// True when EVERY result asked to terminate the run after this batch (pi/DSH rule:
    /// one tool cannot hijack the stop decision alone).
    pub terminate: bool,
    /// Sticky reason contribution (Blocked when the gate denied calls).
    pub reason: TurnEndReason,
}

impl BatchOutcome {
    fn from_results(results: Vec<ToolResultMessage>, denied_any: bool) -> Self {
        let terminate = !results.is_empty()
            && results.iter().all(|r| {
                r.details
                    .get("terminate")
                    .and_then(|t| t.as_bool())
                    .unwrap_or(false)
            });
        let reason = if denied_any {
            TurnEndReason::Blocked
        } else {
            TurnEndReason::Completed
        };
        Self {
            results,
            terminate,
            reason,
        }
    }
}

/// Fail every call in a MaxTokens message: arguments may be truncated JSON, and executing
/// plausible-looking garbage is worse than an explicit retry request (pi invariant).
pub fn fail_all_truncated(assistant: &AssistantMessage) -> BatchOutcome {
    let results = assistant
        .tool_calls()
        .map(|(id, name, _)| ToolResultMessage {
            id: new_id("msg"),
            timestamp_ms: now_ms(),
            tool_call_id: id.to_string(),
            tool_name: name.to_string(),
            content: vec![ContentBlock::text(format!(
                "Tool '{name}' was not executed: the response hit the output token limit, \
                 so its arguments may be truncated. Re-issue the tool call with complete \
                 arguments."
            ))],
            details: serde_json::json!({"truncated": true}),
            usage: None,
            is_error: true,
        })
        .collect();
    BatchOutcome {
        results,
        terminate: false,
        reason: TurnEndReason::MaxTokens,
    }
}

/// Most identifying argument for rule matching: path-ish fields for file tools, the
/// command head (first token) for shell, `*` otherwise.
///
/// Path arguments resolving outside [`Agent.workspace_root`] downgrade the capability to
/// `external_dir` so defaults can ask before any tool code runs (tools re-check defensively).
fn gate_subject(
    name: &str,
    args: &serde_json::Value,
    workspace: &std::path::Path,
) -> (String, String) {
    let path_key = ["path", "file_path", "dir"]
        .iter()
        .find_map(|k| args.get(k).and_then(|v| v.as_str()).map(|s| s.to_string()));
    if let Some(raw) = path_key {
        let candidate = std::path::Path::new(&raw);
        let joined = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            workspace.join(candidate)
        };
        let mut normalized = std::path::PathBuf::new();
        for comp in joined.components() {
            match comp {
                std::path::Component::ParentDir => {
                    normalized.pop();
                }
                std::path::Component::CurDir => {}
                other => normalized.push(other.as_os_str()),
            }
        }
        let inside = normalized.starts_with(workspace);
        return if inside {
            (name.to_string(), raw)
        } else {
            ("external_dir".to_string(), raw)
        };
    }
    if (name == "bash" || name == "shell")
        && let Some(cmd) = args.get("command").and_then(|v| v.as_str())
    {
        return (
            name.to_string(),
            cmd.split_whitespace().next().unwrap_or("*").to_string(),
        );
    }
    (name.to_string(), "*".to_string())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct PreparedCall {
    index: usize,
    invocation: ToolInvocation,
    ctx_tool_name: String,
    blocked_result: Option<ToolResultMessage>,
}

/// Execute one assistant message's tool calls.
pub(super) async fn execute_batch(
    agent: &Arc<Agent>,
    assistant: &AssistantMessage,
) -> Result<BatchOutcome, RunError> {
    let registry = &agent.tools;
    let gate = &agent.gate;
    let limits = &agent.limits;

    // ---- preflight sequentially ------------------------------------------------------
    let mut prepared: Vec<PreparedCall> = Vec::new();
    let mut sequential_batch = false;
    let mut denied_any = false;

    for (i, (call_id, name, args)) in assistant.tool_calls().enumerate() {
        let Some(tool) = registry.get(name) else {
            prepared.push(PreparedCall {
                index: i,
                invocation: ToolInvocation {
                    tool_call_id: call_id.to_string(),
                    args: args.clone(),
                },
                ctx_tool_name: name.to_string(),
                blocked_result: Some(error_result(
                    call_id,
                    name,
                    format!("Tool '{name}' not found"),
                )),
            });
            continue;
        };

        if tool.descriptor().execution_mode == epsilon_contracts::tool::ExecutionMode::Sequential {
            sequential_batch = true;
        }

        if let Err(vmsg) = tool.descriptor().basic_validate(args) {
            prepared.push(PreparedCall {
                index: i,
                invocation: ToolInvocation {
                    tool_call_id: call_id.to_string(),
                    args: args.clone(),
                },
                ctx_tool_name: name.to_string(),
                blocked_result: Some(error_result(
                    call_id,
                    name,
                    format!("Invalid arguments for '{name}': {vmsg}. Please rewrite the input."),
                )),
            });
            continue;
        }

        // ---- capability gate ------------------------------------------------------------
        // Every registered call passes the gate; policies decide silently or interactively.
        // The subject is the most identifying argument (path / command head) so rules like
        // `read *.env -> ask` can match precisely. For interactive asks BOTH events are
        // emitted with the same id so frontends can correlate them (M10 fix for M6-era
        // empty request_ids); the pair lands after resolution because the gate owns the
        // ask timing (the approval channel notifies interactive frontends live).
        let auth = {
            let (capability, pattern) = gate_subject(name, args, &agent.workspace_root);
            let req = PermissionRequest {
                request_id: new_id("perm"),
                capability,
                pattern,
                detail: args.clone(),
            };
            let a = gate.authorize(req.clone()).await;
            if a.asked {
                agent.emit(AgentEvent::PermissionRequested {
                    request: req.clone(),
                });
                agent.emit(AgentEvent::PermissionDecided {
                    request_id: req.request_id.clone(),
                    decision: a.decision.clone(),
                });
            }
            a
        };
        if !auth.allowed() {
            let guidance = auth
                .decision
                .guidance()
                .map(|g| g.to_string())
                .unwrap_or_else(|| {
                    "Permission denied by policy. Adjust approach or ask the user.".to_string()
                });
            prepared.push(PreparedCall {
                index: i,
                invocation: ToolInvocation {
                    tool_call_id: call_id.to_string(),
                    args: args.clone(),
                },
                ctx_tool_name: name.to_string(),
                blocked_result: Some(error_result(call_id, name, guidance)),
            });
            denied_any = true;
            continue;
        }

        // ---- doom-loop brake ----------------------------------------------------------
        // Streak state lives on the Agent (run-scoped): opencode counts consecutive
        // identical calls across batches, not within one batch.
        let canonical = format!("{name}\u{0}{args}");
        let streak = {
            let mut map = agent
                .doom_streaks
                .lock()
                .map_err(|_| RunError::Internal("doom streaks poisoned".into()))?;
            let e = map.entry(canonical.clone()).or_insert(0);
            *e += 1;
            *e
        };

        if streak >= limits.doom_loop_threshold && streak % limits.doom_loop_threshold == 0 {
            let req = PermissionRequest {
                request_id: new_id("perm"),
                capability: "doom_loop".into(),
                pattern: name.to_string(),
                detail: serde_json::json!({
                    "consecutive_calls": streak,
                    "arguments": args,
                }),
            };
            agent.emit(AgentEvent::PermissionRequested {
                request: req.clone(),
            });
            let verdict = gate.authorize(req.clone()).await;
            match verdict.decision {
                d if d.is_denial() => {
                    agent.emit(AgentEvent::PermissionDecided {
                        request_id: req.request_id.clone(),
                        decision: d.clone(),
                    });
                    let guidance = d.guidance().map(|g| g.to_string()).unwrap_or_else(|| {
                        "Blocked by doom-loop guard: identical call repeated. \
                             Change approach or ask the user."
                            .to_string()
                    });
                    prepared.push(PreparedCall {
                        index: i,
                        invocation: ToolInvocation {
                            tool_call_id: call_id.to_string(),
                            args: args.clone(),
                        },
                        ctx_tool_name: name.to_string(),
                        blocked_result: Some(error_result(call_id, name, guidance)),
                    });
                    denied_any = true;
                    continue;
                }
                decision => {
                    agent.emit(AgentEvent::PermissionDecided {
                        request_id: req.request_id.clone(),
                        decision,
                    });
                    // Allowed -> reset this call's streak.
                    if let Ok(mut map) = agent.doom_streaks.lock() {
                        map.insert(canonical.clone(), 0);
                    }
                }
            }
        }

        prepared.push(PreparedCall {
            index: i,
            invocation: ToolInvocation {
                tool_call_id: call_id.to_string(),
                args: args.clone(),
            },
            ctx_tool_name: name.to_string(),
            blocked_result: None,
        });
    }

    // ---- execute ----------------------------------------------------------------------
    let mut slots: Vec<Option<ToolResultMessage>> = vec![None; prepared.len()];

    if sequential_batch {
        for p in prepared.iter_mut() {
            let idx = p.index;
            if let Some(blocked) = p.blocked_result.take() {
                slots[idx] = Some(blocked);
                continue;
            }
            slots[idx] = Some(run_one(agent, p).await);
        }
    } else {
        let sem = Arc::new(tokio::sync::Semaphore::new(limits.max_parallel_tool_calls));
        let mut tasks = Vec::with_capacity(prepared.len());
        for mut p in prepared.into_iter() {
            let idx = p.index;
            match p.blocked_result.take() {
                Some(blocked) => slots[idx] = Some(blocked),
                None => {
                    let agent = Arc::clone(agent);
                    let permit = Arc::clone(&sem);
                    tasks.push(tokio::spawn(async move {
                        let _permit = permit.acquire_owned().await.expect("semaphore open");
                        (idx, run_one(&agent, &mut p).await)
                    }));
                }
            }
        }
        for t in tasks {
            match t.await {
                Ok((idx, result)) => slots[idx] = Some(result),
                Err(join_err) => {
                    return Err(RunError::Internal(format!(
                        "tool task join failure: {join_err}"
                    )));
                }
            }
        }
    }

    let results = slots
        .into_iter()
        .flatten()
        .collect::<Vec<ToolResultMessage>>();

    Ok(BatchOutcome::from_results(results, denied_any))
}

async fn run_one(agent: &Arc<Agent>, p: &mut PreparedCall) -> ToolResultMessage {
    let name = p.ctx_tool_name.clone();
    let tool_call_id = p.invocation.tool_call_id.clone();
    agent.emit(AgentEvent::ToolExecutionStart {
        tool_call_id: tool_call_id.clone(),
        tool_name: name.clone(),
        args: p.invocation.args.clone(),
    });

    let Some(tool) = agent.tools.get(&name) else {
        return error_result(
            &tool_call_id,
            &name,
            format!("Tool '{name}' vanished mid-run"),
        );
    };

    let ctx = ToolContext {
        session_id: String::from("inline"),
        // Tools operate on the AGENT's declared workspace, not the process's incidental cwd.
        cwd: agent.workspace_root.clone(),
        spill_dir: agent.workspace_root.join(".epsilon").join("spill"),
        token: tokio_util::sync::CancellationToken::new(),
    };
    let sink: epsilon_contracts::tool::ToolUpdateSink =
        Arc::new(|_partial: epsilon_contracts::tool::ToolPartial| {});

    let outcome: Result<ToolOutput, ToolFailure> = tool
        .execute(p.invocation.clone(), ctx, Arc::clone(&sink))
        .await;

    let is_error = outcome.is_err();
    agent.emit(AgentEvent::ToolExecutionEnd {
        tool_call_id: tool_call_id.clone(),
        tool_name: name.clone(),
        is_error,
    });

    match outcome {
        Ok(out) => ToolResultMessage {
            id: new_id("msg"),
            timestamp_ms: now_ms(),
            tool_call_id,
            tool_name: name,
            content: out.content,
            details: attach_terminate(out.details, out.terminate),
            usage: out.usage,
            is_error: false,
        },
        Err(fail) => error_result(&tool_call_id, &name, fail.message),
    }
}

/// Encode the pi/DSH terminate hint inside details (results are the only carrier that
/// reaches BatchOutcome without widening the message type).
fn attach_terminate(mut details: serde_json::Value, terminate: bool) -> serde_json::Value {
    if let Some(obj) = details.as_object_mut() {
        obj.insert("terminate".into(), serde_json::Value::Bool(terminate));
    }
    details
}

fn error_result(call_id: &str, name: &str, message: String) -> ToolResultMessage {
    ToolResultMessage {
        id: new_id("msg"),
        timestamp_ms: now_ms(),
        tool_call_id: call_id.to_string(),
        tool_name: name.to_string(),
        content: vec![ContentBlock::text(message)],
        details: serde_json::json!({"blocked": true}),
        usage: None,
        is_error: true,
    }
}
