//! Tool batch execution: bounded parallel pool, sequential barriers, doom-loop brake.
//!
//! Semantics (ported + improved):
//! - preflight every call sequentially (resolve, validate, gate) — pi's ordering;
//! - a call's ordering is per-resource (M72): a `Sequential` tool with no declared resource
//!   (`bash`, `task` — reach unknown) still makes the batch a barrier, while one that names its
//!   resource (`write`, `edit`) is ordered only against the calls it conflicts with, so three
//!   edits to three files run at the same time and two edits to one file stay in call order;
//! - parallel-safe calls run on a semaphore-bounded pool; results commit in MODEL order
//!   regardless of completion order (DSH rule) so transcripts stay replay-stable.

use std::sync::Arc;

use epsilon_contracts::agent_event::{AgentEvent, PermissionRequest, TurnEndReason};
use epsilon_contracts::content::ContentBlock;
use epsilon_contracts::ids::new_id;
use epsilon_contracts::message::{AssistantMessage, ToolResultMessage};
use epsilon_contracts::tool::ExecutionMode;
use epsilon_contracts::tool::{
    AfterToolCallContext, AfterToolCallResult, BeforeToolCallContext, ToolContext, ToolFailure,
    ToolInvocation, ToolOutput,
};

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
/// Path arguments resolving outside [`Agent.workspace_root`] add an `external_dir`
/// authorization after the original tool capability. Both must allow execution; an
/// external-path approval must never replace tool restrictions (tools also re-check).
///
/// Plan 004 (SECURITY-03): the returned subject is LEXICALLY NORMALIZED — leading
/// `./` stripped, `a/../b` collapsed, trailing `/` removed — and the workspace prefix
/// is re-relativized, so `.env`, `./.env`, and `src/../.env` all produce the SAME
/// subject. Rule matching and session-memory keys become consistent (one approval
/// covers all spellings of one file) instead of tripling prompts on path spelling.
///
/// **Containment is decided by `paths::resolve_for_access`, not here.** This function
/// used to carry its own copy of the normalizer and its own `starts_with` check, and
/// both were lexical, so a path that spelled its way inside the workspace through a
/// symlink asked nobody: the gate said `write`, `write` is `Allow` by default, and the
/// tool followed the link out. Two copies of one rule is how that happened — the tool
/// copy and this copy could not have disagreed visibly, because neither of them
/// consulted the filesystem.
///
/// The **subject** stays lexical, and deliberately so: the approval prompt has to name
/// the file the model asked about, not the real path behind a link, or the user is
/// asked about something they never typed. The one exception is a path that escaped —
/// there the real target is the whole point, so `escaped_to()` becomes the subject and
/// the prompt says where the call was actually going.
fn gate_subject(
    name: &str,
    args: &serde_json::Value,
    workspace: &std::path::Path,
) -> (String, String) {
    let path_key = ["path", "file_path", "dir"]
        .iter()
        .find_map(|k| args.get(k).and_then(|v| v.as_str()).map(|s| s.to_string()));
    if let Some(raw) = path_key {
        let resolution = crate::paths::resolve_for_access(workspace, &raw);
        // Re-relativize inside-workspace paths so rules match on stable relative
        // subjects regardless of how the model spelled the path.
        let subject = if let Some(target) = resolution.escaped_to() {
            crate::paths::display_canonical(target)
                .trim_end_matches('/')
                .to_string()
        } else if resolution.inside {
            resolution
                .path
                .strip_prefix(workspace)
                .unwrap_or(&resolution.path)
                .to_string_lossy()
                .trim_end_matches('/')
                .to_string()
        } else {
            raw.trim_end_matches('/').to_string()
        };
        return if resolution.inside {
            (name.to_string(), subject)
        } else {
            ("external_dir".to_string(), subject)
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
    /// Validated args (kept for the before-hook context; `invocation.args` is
    /// the same value — this alias documents intent at hook call sites).
    validated_args: serde_json::Value,
    /// The one resource this call acts on, when the tool declares it (M72).
    resource: Option<std::path::PathBuf>,
    /// The tool's declared concurrency class.
    mode: epsilon_contracts::tool::ExecutionMode,
}

/// Execute one assistant message's tool calls.
/// What the planner needs to know about one call in a batch.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CallShape {
    /// The tool's declared concurrency class.
    pub mode: ExecutionMode,
    /// The single resource the call acts on, when the tool names one.
    pub resource: Option<std::path::PathBuf>,
}

/// How a batch's calls may be ordered against each other.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BatchPlan {
    /// A call whose reach is unknown (a shell command, a subagent): one call at a time.
    Barrier,
    /// Conflict groups, each holding its calls in the order the model asked for them. Groups
    /// are independent by construction, so they may run at the same time.
    Segments(Vec<Vec<usize>>),
}

/// Decide what may run concurrently.
///
/// The old rule was all-or-nothing: one `Sequential` tool and the whole batch serialized, so a
/// model sending three edits to three different files paid for them one at a time. The rule here
/// is per-resource — a call is ordered against the calls it actually conflicts with:
///
/// - **Unknown reach** (`Sequential` with no declared resource: `bash`, `task`): the batch is a
///   barrier. A shell command can touch anything; pretending otherwise is how files get lost.
/// - **Same resource**: one group, in call order. Two readers are the exception (nothing to
///   order between them), which keeps a batch of workspace-wide searches concurrent.
/// - **Different resources**: independent groups. Two edits to different files are exactly that.
///
/// Keys conflict by *prefix*, not equality: a `grep` over `src/` conflicts with a `write` to
/// `src/lib.rs`, even though the two paths are different strings.
pub(crate) fn plan_batch(calls: &[CallShape]) -> BatchPlan {
    if calls
        .iter()
        .any(|c| c.mode == ExecutionMode::Sequential && c.resource.is_none())
    {
        return BatchPlan::Barrier;
    }

    // Union-find over conflicting pairs: every conflict class becomes one ordered group.
    let n = calls.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for i in 0..n {
        for j in (i + 1)..n {
            if conflicts(&calls[i], &calls[j]) {
                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                if a != b {
                    parent[a] = b;
                }
            }
        }
    }

    let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
    for i in 0..n {
        let r = root(&mut parent, i);
        match groups.iter_mut().find(|(existing, _)| *existing == r) {
            Some((_, members)) => members.push(i),
            None => groups.push((r, vec![i])),
        }
    }
    BatchPlan::Segments(groups.into_iter().map(|(_, members)| members).collect())
}

/// Do two calls touch the same resource in a way that needs ordering?
fn conflicts(a: &CallShape, b: &CallShape) -> bool {
    let (Some(pa), Some(pb)) = (a.resource.as_deref(), b.resource.as_deref()) else {
        return false;
    };
    if !(pa.starts_with(pb) || pb.starts_with(pa)) {
        return false;
    }
    // Two readers have nothing to order between them.
    !(a.mode == ExecutionMode::Parallel && b.mode == ExecutionMode::Parallel)
}

/// Run one tool by name on the caller's behalf, outside a model turn.
///
/// **The TUI's `!` shell escape is the reason this exists.** Preparing a
/// `PreparedCall` needs the registry lookup, `basic_validate`, the resource and
/// mode classification, and the capability gate — and every one of those is a
/// rule that decides *whether a command may run*. Building a call at the call
/// site would skip them, so a `!` command could do something the same command
/// issued by the model could not. So the preparation happens here, next to the
/// code that knows it, and the frontend gets one function instead of a second
/// execution path.
///
/// The sequence below is `execute_batch`'s, in the same order, on purpose.
pub async fn run_local(
    agent: &Arc<Agent>,
    name: &str,
    args: serde_json::Value,
) -> Result<String, String> {
    let Some(tool) = agent.tools.get(name) else {
        return Err(format!("no tool named {name}"));
    };
    let mode = tool.descriptor().execution_mode;
    let resource = tool.resource_key(&args, &agent.workspace_root);
    tool.descriptor()
        .basic_validate(&args)
        .map_err(|v| format!("invalid arguments for '{name}': {v}"))?;

    // The capability gate, exactly as a model-issued call meets it. `asked` is
    // honoured too: a `!` command can therefore raise an approval prompt, which
    // is the point — the user is running it, so the user is asked.
    let (capability, pattern) = gate_subject(name, &args, &agent.workspace_root);
    let capabilities =
        std::iter::once(name.to_string()).chain((capability != name).then_some(capability));
    // `allow_once` is the neutral starting point: the first `authorize` call
    // overwrites it, and the loop below always makes at least one.
    let mut auth = crate::Authorization::allow_once();
    for capability in capabilities {
        let req = PermissionRequest {
            request_id: new_id("perm"),
            capability,
            pattern: pattern.clone(),
            detail: args.clone(),
        };
        auth = agent.gate.authorize(req.clone()).await;
        if auth.asked {
            agent.emit(AgentEvent::PermissionRequested {
                request: req.clone(),
            });
            agent.emit(AgentEvent::PermissionDecided {
                request_id: req.request_id.clone(),
                decision: auth.decision.clone(),
            });
        }
        if !auth.allowed() {
            break;
        }
    }
    if !auth.allowed() {
        return Err(auth
            .decision
            .guidance()
            .map(|g| g.to_string())
            .unwrap_or_else(|| "Permission denied by policy.".to_string()));
    }

    let mut call = PreparedCall {
        index: 0,
        invocation: ToolInvocation {
            tool_call_id: "local".to_string(),
            args: args.clone(),
        },
        ctx_tool_name: name.to_string(),
        blocked_result: None,
        validated_args: args,
        resource,
        mode,
    };
    let result = run_one(agent, &mut call).await;
    let text = result
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if result.is_error {
        Err(if text.is_empty() {
            format!("{name} failed")
        } else {
            text
        })
    } else {
        Ok(text)
    }
}

pub(super) async fn execute_batch(
    agent: &Arc<Agent>,
    assistant: &AssistantMessage,
) -> Result<BatchOutcome, RunError> {
    let registry = &agent.tools;
    let gate = &agent.gate;
    let limits = &agent.limits;

    // ---- preflight sequentially ------------------------------------------------------
    let mut prepared: Vec<PreparedCall> = Vec::new();
    let mut denied_any = false;
    // A stalled run is a property of the batch, not of each call in it: ask once.
    let mut asked_no_progress = false;

    for (i, (call_id, name, args)) in assistant.tool_calls().enumerate() {
        let Some(tool) = registry.get(name) else {
            prepared.push(PreparedCall {
                index: i,
                invocation: ToolInvocation {
                    tool_call_id: call_id.to_string(),
                    args: args.clone(),
                },
                ctx_tool_name: name.to_string(),
                validated_args: args.clone(),
                // Blocked before execution, so it needs no resource and must not force the
                // batch to serialize.
                resource: None,
                mode: ExecutionMode::Parallel,
                blocked_result: Some(error_result(
                    call_id,
                    name,
                    format!("Tool '{name}' not found"),
                )),
            });
            continue;
        };

        let mode = tool.descriptor().execution_mode;
        // The planner's input: what this call touches, and whether it only reads.
        let resource = tool.resource_key(args, &agent.workspace_root);
        // A validated call that will run: remember where it works, so that directory's
        // AGENTS.md family can be loaded into the next request's context (M73). Recorded here
        // rather than after execution because the *intent* is what locates the conventions —
        // a call that failed on a missing file still says what part of the tree is in play.
        if let Some(path) = resource.as_deref() {
            agent.note_touched(path);
        }

        if let Err(vmsg) = tool.descriptor().basic_validate(args) {
            prepared.push(PreparedCall {
                index: i,
                invocation: ToolInvocation {
                    tool_call_id: call_id.to_string(),
                    args: args.clone(),
                },
                ctx_tool_name: name.to_string(),
                validated_args: args.clone(),
                // Blocked before execution, so it needs no resource and must not force the
                // batch to serialize.
                resource: None,
                mode: ExecutionMode::Parallel,
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
            // Derived path permission is additive, never a replacement for the
            // registered tool's policy. Preserve subject selection and short-circuit
            // on denial; each real ask keeps its own event pair and session key.
            let capabilities =
                std::iter::once(name.to_string()).chain((capability != name).then_some(capability));
            let mut auth = crate::Authorization::allow_once();
            for capability in capabilities {
                let req = PermissionRequest {
                    request_id: new_id("perm"),
                    capability,
                    pattern: pattern.clone(),
                    detail: args.clone(),
                };
                auth = gate.authorize(req.clone()).await;
                if auth.asked {
                    agent.emit(AgentEvent::PermissionRequested {
                        request: req.clone(),
                    });
                    agent.emit(AgentEvent::PermissionDecided {
                        request_id: req.request_id.clone(),
                        decision: auth.decision.clone(),
                    });
                }
                if !auth.allowed() {
                    break;
                }
            }
            auth
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
                validated_args: args.clone(),
                // Blocked before execution, so it needs no resource and must not force the
                // batch to serialize.
                resource: None,
                mode: ExecutionMode::Parallel,
                blocked_result: Some(error_result(call_id, name, guidance)),
            });
            denied_any = true;
            continue;
        }

        // ---- stuck-run brake (port plan item 7) ---------------------------------------
        // The doom-loop brake below catches identical *calls*; this catches a model that keeps
        // working while the world stops changing — different commands, same result. The warn
        // rung already rode along with the result itself; this is the block rung, and it uses the
        // same permission surface so a frontend can see and answer it.
        let stalled = (!asked_no_progress)
            .then(|| agent.progress_streak())
            .flatten()
            .filter(|streak| {
                *streak >= limits.no_progress_threshold && limits.no_progress_threshold > 0
            });
        if let Some(streak) = stalled {
            asked_no_progress = true;
            let req = PermissionRequest {
                request_id: new_id("perm"),
                capability: "no_progress".into(),
                pattern: name.to_string(),
                detail: serde_json::json!({
                    "identical_results": streak,
                    "tool": name,
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
                        format!(
                            "Blocked by the no-progress guard: the last {streak} tool results were \
                             identical. Report what is blocking you, or change approach."
                        )
                    });
                    prepared.push(PreparedCall {
                        index: i,
                        invocation: ToolInvocation {
                            tool_call_id: call_id.to_string(),
                            args: args.clone(),
                        },
                        ctx_tool_name: name.to_string(),
                        validated_args: args.clone(),
                        // Blocked before execution, so it needs no resource and must not force the
                        // batch to serialize.
                        resource: None,
                        mode: ExecutionMode::Parallel,
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
                }
            }
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
                        validated_args: args.clone(),
                        // Blocked before execution, so it needs no resource and must not force the
                        // batch to serialize.
                        resource: None,
                        mode: ExecutionMode::Parallel,
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
            validated_args: args.clone(),
            resource,
            mode,
            blocked_result: None,
        });
    }

    // ---- execute ----------------------------------------------------------------------
    let mut slots: Vec<Option<ToolResultMessage>> = vec![None; prepared.len()];

    let shapes: Vec<CallShape> = prepared
        .iter()
        .map(|p| CallShape {
            mode: p.mode,
            resource: p.resource.clone(),
        })
        .collect();

    match plan_batch(&shapes) {
        BatchPlan::Barrier => {
            for p in prepared.iter_mut() {
                let idx = p.index;
                if let Some(blocked) = p.blocked_result.take() {
                    slots[idx] = Some(blocked);
                    continue;
                }
                slots[idx] = Some(run_one(agent, p).await);
            }
        }
        BatchPlan::Segments(groups) => {
            // One task per conflict group; the calls *inside* a group run in call order, which
            // is what makes "read this, then edit it" (and "edit the same file twice") mean the
            // same thing it meant when the whole batch ran one call at a time.
            let sem = Arc::new(tokio::sync::Semaphore::new(limits.max_parallel_tool_calls));
            let mut queue: Vec<Option<PreparedCall>> = prepared.into_iter().map(Some).collect();
            let mut tasks = Vec::with_capacity(groups.len());
            for members in groups {
                let mut mine: Vec<PreparedCall> = Vec::with_capacity(members.len());
                for idx in members {
                    if let Some(p) = queue.get_mut(idx).and_then(Option::take) {
                        mine.push(p);
                    }
                }
                let agent = Arc::clone(agent);
                let permit = Arc::clone(&sem);
                tasks.push(tokio::spawn(async move {
                    let mut out: Vec<(usize, ToolResultMessage)> = Vec::with_capacity(mine.len());
                    for mut p in mine {
                        let idx = p.index;
                        if let Some(blocked) = p.blocked_result.take() {
                            out.push((idx, blocked));
                            continue;
                        }
                        // Cloned per call: the permit is released when the call returns, so a
                        // group with several calls never holds one while it waits.
                        let _permit = Arc::clone(&permit)
                            .acquire_owned()
                            .await
                            .expect("semaphore open");
                        out.push((idx, run_one(&agent, &mut p).await));
                    }
                    out
                }));
            }
            for t in tasks {
                match t.await {
                    Ok(pairs) => {
                        for (idx, result) in pairs {
                            slots[idx] = Some(result);
                        }
                    }
                    Err(join_err) => {
                        return Err(RunError::Internal(format!(
                            "tool task join failure: {join_err}"
                        )));
                    }
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

    // Abort checkpoint (F20): a call that has not started must not start once the run
    // is cancelled. This is the single choke point for both batch plans, so a cancelled
    // batch never begins new work — in-flight calls still settle, exactly as pi awaits
    // scheduled calls and emits their results.
    if agent.current_run_token().is_some_and(|t| t.is_cancelled()) {
        return aborted_result(&tool_call_id, &name);
    }

    agent.emit(AgentEvent::ToolExecutionStart {
        tool_call_id: tool_call_id.clone(),
        tool_name: name.clone(),
        args: p.invocation.args.clone(),
    });

    // ---- before-hook (pi `prepareToolCall` → `config.beforeToolCall`) --------
    // Runs after validation + gate (denied calls never reach here — they carry
    // `blocked_result`), before execution. A `block` becomes an error result
    // with the hook's reason; `terminate` rides in details for the batch rule.
    // A hook that FAILS (returns Err) blocks the call with its message (pi
    // surfaces hook errors as error results, never loop panics).
    let before = {
        let hooks = agent.tool_hooks();
        let ctx = BeforeToolCallContext {
            tool_call_id: tool_call_id.clone(),
            tool_name: name.clone(),
            args: p.validated_args.clone(),
        };
        match hooks.before_tool_call(ctx).await {
            Ok(r) => r,
            Err(e) => epsilon_contracts::tool::BeforeToolCallResult {
                block: true,
                reason: Some(e),
                terminate: false,
            },
        }
    };
    if before.block {
        let reason = before
            .reason
            .unwrap_or_else(|| "Tool execution was blocked".to_string());
        let mut result = error_result(&tool_call_id, &name, reason);
        if before.terminate {
            result.details = attach_terminate(result.details, true);
        }
        return result;
    }

    let Some(tool) = agent.tools.get(&name) else {
        return error_result(
            &tool_call_id,
            &name,
            format!("Tool '{name}' vanished mid-run"),
        );
    };

    let ctx = ToolContext {
        session_id: String::from("inline"),
        observation_scope: agent
            .observation_scope
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
        // Tools operate on the AGENT's declared workspace, not the process's incidental cwd.
        cwd: agent.workspace_root.clone(),
        spill_dir: agent.workspace_root.join(".epsilon").join("spill"),
        // Run-scoped cancellation (M12): abort_current_run() reaches in-flight tools.
        token: agent.current_run_token().unwrap_or_default(),
        // G4: the loop itself is the isolated host — tools call back without depending on the loop crate.
        task_host: Some(agent.clone_task_host()),
    };
    let sink: epsilon_contracts::tool::ToolUpdateSink = {
        // Partial tool output reaches frontends as events (pi streams bash
        // progress the same way). This used to be a no-op sink, so a running
        // command was invisible — the tool had the callback and nobody listened.
        let agent = Arc::clone(agent);
        let ev_call_id = tool_call_id.clone();
        let ev_name = name.clone();
        Arc::new(move |partial: epsilon_contracts::tool::ToolPartial| {
            agent.emit(AgentEvent::ToolExecutionUpdate {
                tool_call_id: ev_call_id.clone(),
                tool_name: ev_name.clone(),
                partial,
            });
        })
    };

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
        Ok(out) => {
            // Shape before the literal: `tool_name: name` moves the name, so the shaping call
            // has to happen while it can still be borrowed.
            let content = shape_content(&name, out.content);
            let base = ToolResultMessage {
                id: new_id("msg"),
                timestamp_ms: now_ms(),
                tool_call_id,
                tool_name: name,
                content,
                details: attach_terminate(out.details, out.terminate),
                usage: out.usage,
                is_error: false,
            };
            // ---- after-hook (pi `finalizeExecutedToolCall` → `config.afterToolCall`)
            // Field-by-field merge; `None` keeps the executed value. A hook
            // failure replaces the result with an error result (pi parity).
            apply_after_hook(agent, base, &p.validated_args).await
        }
        Err(fail) => error_result(&tool_call_id, &name, fail.message),
    }
}

/// Apply untrusted-content shaping to a settled result's text blocks (`crate::untrusted`).
///
/// Only text is touched: an image carries no instructions, and a non-text block has to survive
/// byte-for-byte for the provider wire. Local tools pass through untouched — their content is
/// the user's own workspace, and wrapping it would bury the context the model works in.
fn shape_content(tool_name: &str, content: Vec<ContentBlock>) -> Vec<ContentBlock> {
    if !crate::untrusted::is_untrusted_source(tool_name) {
        return content;
    }
    content
        .into_iter()
        .map(|block| match block {
            ContentBlock::Text { text } => {
                ContentBlock::text(crate::untrusted::shape_untrusted(tool_name, &text))
            }
            other => other,
        })
        .collect()
}

/// Run the after-hook merge for one settled result.
async fn apply_after_hook(
    agent: &Arc<Agent>,
    base: ToolResultMessage,
    args: &serde_json::Value,
) -> ToolResultMessage {
    let hooks = agent.tool_hooks();
    let ctx = AfterToolCallContext {
        tool_call_id: base.tool_call_id.clone(),
        tool_name: base.tool_name.clone(),
        args: args.clone(),
        result: base.clone(),
        is_error: base.is_error,
    };
    let patch: Result<Option<AfterToolCallResult>, String> = hooks.after_tool_call(ctx).await;
    match patch {
        Ok(None) => base,
        Ok(Some(patch)) => ToolResultMessage {
            id: base.tool_call_id.clone(),
            content: patch.content.unwrap_or(base.content),
            details: match patch.terminate {
                Some(t) => attach_terminate(patch.details.unwrap_or(base.details), t),
                None => patch.details.unwrap_or(base.details),
            },
            usage: patch.usage.or(base.usage),
            is_error: patch.is_error.unwrap_or(base.is_error),
            ..base
        },
        Err(e) => error_result(&base.tool_call_id, &base.tool_name, e),
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

/// Result for a call the run refused to start after cancellation (F20).
///
/// pi stops scheduling the remainder of an aborted batch and marks each unstarted call
/// as "Operation aborted" (`agent-loop.ts:629-635`); the result still arrives, in call
/// order, so the transcript never carries a call without an answer and the model is
/// told plainly that the work did not happen.
fn aborted_result(call_id: &str, name: &str) -> ToolResultMessage {
    ToolResultMessage {
        id: new_id("msg"),
        timestamp_ms: now_ms(),
        tool_call_id: call_id.to_string(),
        tool_name: name.to_string(),
        content: vec![ContentBlock::text(
            "[harness] this call was not executed: the run was cancelled before it started.",
        )],
        details: serde_json::json!({"aborted": true}),
        usage: None,
        is_error: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn reader(path: &str) -> CallShape {
        CallShape {
            mode: ExecutionMode::Parallel,
            resource: Some(PathBuf::from(path)),
        }
    }

    fn writer(path: &str) -> CallShape {
        CallShape {
            mode: ExecutionMode::Sequential,
            resource: Some(PathBuf::from(path)),
        }
    }

    fn shell() -> CallShape {
        // `bash`: Sequential with no declared resource.
        CallShape {
            mode: ExecutionMode::Sequential,
            resource: None,
        }
    }

    fn unkeyed_reader() -> CallShape {
        CallShape {
            mode: ExecutionMode::Parallel,
            resource: None,
        }
    }

    #[test]
    fn edits_to_different_files_are_independent_and_share_no_group() {
        let plan = plan_batch(&[writer("/ws/a.rs"), writer("/ws/b.rs"), writer("/ws/c.rs")]);
        match plan {
            BatchPlan::Segments(groups) => {
                assert_eq!(groups.len(), 3, "three files, three groups: {groups:?}");
                for (i, g) in groups.iter().enumerate() {
                    assert_eq!(g, &vec![i], "each group holds exactly its own call");
                }
            }
            other => panic!("expected segments, got {other:?}"),
        }
    }

    #[test]
    fn two_writes_to_one_file_stay_in_call_order() {
        let plan = plan_batch(&[
            writer("/ws/a.rs"),
            writer("/ws/other.rs"),
            writer("/ws/a.rs"),
        ]);
        match plan {
            BatchPlan::Segments(groups) => {
                let same: Vec<&Vec<usize>> = groups.iter().filter(|g| g.len() > 1).collect();
                assert_eq!(same.len(), 1, "one ordered chain: {groups:?}");
                assert_eq!(
                    same[0],
                    &vec![0, 2],
                    "in call order, not sorted: {groups:?}"
                );
                let other = groups
                    .iter()
                    .find(|g| g == &&vec![1])
                    .expect("the other file");
                assert_eq!(other, &vec![1]);
            }
            other => panic!("expected segments, got {other:?}"),
        }
    }

    #[test]
    fn a_reader_and_a_writer_of_one_file_share_a_group() {
        // "read it, then edit it" only means something if the read happens first.
        let plan = plan_batch(&[reader("/ws/a.rs"), writer("/ws/a.rs")]);
        match plan {
            BatchPlan::Segments(groups) => {
                assert_eq!(groups, vec![vec![0, 1]], "one chain, read first");
            }
            other => panic!("expected one group, got {other:?}"),
        }
    }

    #[test]
    fn two_readers_of_one_file_do_not_wait_for_each_other() {
        // Nothing to order between them, and this is what keeps a batch of workspace-wide
        // searches (all keyed on the root) concurrent.
        let plan = plan_batch(&[reader("/ws/src"), reader("/ws/src"), reader("/ws/src")]);
        match plan {
            BatchPlan::Segments(groups) => {
                assert_eq!(groups.len(), 3, "three independent groups: {groups:?}");
            }
            other => panic!("expected segments, got {other:?}"),
        }
    }

    #[test]
    fn a_directory_key_conflicts_with_what_is_inside_it() {
        let plan = plan_batch(&[reader("/ws/src"), writer("/ws/src/lib.rs")]);
        match plan {
            BatchPlan::Segments(groups) => {
                assert_eq!(groups, vec![vec![0, 1]], "prefix conflict: {groups:?}");
            }
            other => panic!("expected one group, got {other:?}"),
        }
        // ...but not with a sibling directory.
        let plan = plan_batch(&[reader("/ws/src"), writer("/ws/tests/lib.rs")]);
        match plan {
            BatchPlan::Segments(groups) => assert_eq!(groups.len(), 2, "{groups:?}"),
            other => panic!("expected segments, got {other:?}"),
        }
    }

    #[test]
    fn a_shell_command_still_stops_the_whole_batch() {
        // Its reach is unknown: it could touch anything, including the files other calls name.
        for calls in [
            vec![writer("/ws/a.rs"), shell(), writer("/ws/b.rs")],
            vec![shell()],
            vec![reader("/ws/a.rs"), shell()],
        ] {
            assert_eq!(
                plan_batch(&calls),
                BatchPlan::Barrier,
                "a shell call must serialize: {calls:?}"
            );
        }
    }

    #[test]
    fn blocked_calls_neither_serialize_nor_conflict() {
        // A call that failed validation or was denied never executes: it must not cost the
        // batch its parallelism (it used to, when every Sequential tool meant a barrier).
        let plan = plan_batch(&[unkeyed_reader(), writer("/ws/a.rs"), writer("/ws/b.rs")]);
        match plan {
            BatchPlan::Segments(groups) => assert_eq!(groups.len(), 3, "{groups:?}"),
            other => panic!("expected segments, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_batch_plans_to_nothing() {
        assert_eq!(plan_batch(&[]), BatchPlan::Segments(vec![]));
    }
}
