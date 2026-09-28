//! rpc mode (ADR-011): line-delimited JSON over stdio for editor embedding.
//!
//! Protocol: each inbound line is one request — `{"op": "...", ...}`. pi's frames tag
//! the same object `type` instead of `op` (`rpc-types.ts:20-73`); the tag is accepted as
//! an alias, and an optional `id` is echoed on that request's reply (pi's client resolves
//! only `type:'response'` frames whose id matches a pending request —
//! `rpc-client.ts:508-522`). Replies carry pi's envelope (`type`, `command`, `id`,
//! `success`, `data`|`error`) *and* keep epsilon's original `ok`/`error` keys with the
//! same shapes, so clients written against the first wire keep working. Every
//! [`epsilon_contracts::AgentEvent`] is streamed outbound as `{"event": {...}}` as it
//! happens; a dropped burst is reported as `{"lagged":{"dropped":N}}`, the line-protocol
//! form of serve mode's `lagged` SSE event. Same runtime + approval mechanics as serve
//! mode — the transport is the only difference.

use std::sync::Arc;

use epsilon_contracts::agent_event::PermissionDecision;
use epsilon_contracts::{LaggedNotice, SessionState};
use epsilon_core::Agent;
use epsilon_safety::ChannelApprovalUi;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

/// Inbound operations.
///
/// The correlation header (`id`) is read separately (see [`RequestHead`]) so a reply can
/// echo it; it is not part of any variant's payload.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Op {
    /// Start a run with a user task.
    Prompt { task: String },
    /// Queue a mid-run interjection.
    Steer { text: String },
    /// Queue post-run work.
    ///
    /// **The wire name is pi's `follow_up` (`rpc-types.ts:30`), not `followup`.** Serde's
    /// `rename_all = "snake_case"` would have produced `followup`, and that is what this
    /// op was called until a probe printed its own reply: `unknown op: follow_up`. A
    /// pi-shaped client sending the documented name was refused, and the pre-existing test
    /// had never dispatched this op at all — it read the *variant* name, not the wire one,
    /// so it agreed with the code and missed the wire.
    ///
    /// Both spellings are accepted from here on: `follow_up` is what pi documents and what
    /// a client will send, and refusing a name pi uses is the failure mode this whole
    /// surface is supposed to avoid.
    #[serde(rename = "follow_up", alias = "followup")]
    Followup { text: String },
    /// List pending permission asks.
    Approvals,
    /// Answer one ask.
    ///
    /// Known wart: this `id` is the *ask* id, and it shares its top-level JSON key with
    /// the correlation header ([`RequestHead::id`]) — the wire has one `id` field, so an
    /// approve reply echoes the ask id rather than a client-chosen request id (legacy
    /// clients ignore reply `id`s; separating the names would change the request shape).
    Approve {
        id: String,
        decision: PermissionDecision,
    },
    /// Liveness.
    Ping,
    /// Cancel the active run; the reply carries core's answer.
    Abort,
    /// Report a [`SessionState`] snapshot.
    GetState,
    /// Set the session thinking level.
    SetThinkingLevel {
        level: epsilon_contracts::ThinkingLevel,
    },
    /// Step the thinking level one rung up the ladder, wrapping at the top.
    CycleThinkingLevel,
    /// List the levels `cycle_thinking_level` walks, in order.
    GetAvailableThinkingLevels,
    /// Switch the model for subsequent runs.
    ///
    /// pi's shape is `provider` + `modelId` (`rpc-types.ts:33`). The pair is resolved the
    /// way every other entry point resolves it — `provider/model`, or a bare model id — so
    /// a pi-shaped client and a human typing `/model opus` reach the same row.
    SetModel {
        #[serde(default)]
        provider: Option<String>,
        #[serde(default)]
        model_id: Option<String>,
    },
    /// List catalog rows, optionally filtered to one provider.
    GetAvailableModels {
        #[serde(default)]
        provider: Option<String>,
    },
    /// Step to the next catalog row in the current provider.
    CycleModel,
    /// The most recent assistant message's text, or null when there is none.
    GetLastAssistantText,

    /// Empty both pending queues and return what was in them.
    ClearQueue,
    /// The conversation so far, as `Message` values.
    GetMessages,
    /// Everything invocable by name: built-in commands, skills, prompt templates.
    GetCommands,
    /// Token, cost and cache totals for the session.
    GetSessionStats,

    /// A well-formed frame naming an op this build does not implement.
    ///
    /// A named variant rather than a parse error, so the reply can name the op and echo
    /// its id instead of relaying serde's variant list. The unimplemented families stay
    /// unimplemented on purpose until their core support exists (set_model, cycle_model,
    /// queue modes, compaction over the wire, session/fork/tree/export/messages,
    /// extension_ui, bash) — a half-wired acknowledgement is the failure mode RPC-01
    /// just closed.
    #[serde(other)]
    Unknown,
}

/// The correlation header of an inbound line, readable before the op is interpreted.
#[derive(Debug, Deserialize)]
struct RequestHead {
    /// Correlation id (pi's client always sends one: `req_<n>`, `rpc-client.ts:568`).
    #[serde(default)]
    id: Option<String>,
    /// The op tag; pi's `type` has already been pointed here by [`normalize_tag`].
    #[serde(default)]
    op: Option<String>,
}

/// What one prompt request actually did (RPC-01).
#[derive(Debug, PartialEq, Eq)]
enum Admission {
    /// The run owns the agent now; its outcome arrives through the event stream.
    Started,
    /// No work started; the string is the attributable reason.
    Rejected(String),
}

/// Admit a prompt run and report what actually happened.
///
/// `Agent::run` decides admission itself and settles it before its first await:
/// a rejected run returns `AlreadyRunning` immediately, an admitted one emits
/// `AgentStart` immediately after the check. Racing the spawned run against its
/// own first event therefore answers truthfully without waiting for the work to
/// finish. The old shape consulted a helper that always returned "not busy" and
/// replied `started` to a request the agent had already rejected (pi emits
/// success only through prompt preflight — `rpc-mode.ts:394-415`).
async fn admit_prompt(
    agent: &Arc<Agent>,
    prompt: epsilon_contracts::message::Message,
) -> Admission {
    enum Step {
        Started,
        Finished,
        Rejected(String),
        Again,
    }

    // Subscribe before spawning: here, on this line, no runtime can deliver the
    // AgentStart that decides this request before we are listening for it.
    let mut adm = agent.subscribe();
    let a = Arc::clone(agent);
    let mut handle = tokio::spawn(async move { a.run(prompt).await });
    let step = loop {
        let step = tokio::select! {
            // The event arm first, and biased: a run that was admitted and then failed
            // has its AgentStart queued before its outcome resolves, so it is reported
            // as started — its events already streamed and its prompt is already durable.
            // Answering "no work started" to it would invite a retry that double-prompts.
            biased;
            ev = adm.recv() => match ev {
                Ok(ev) => {
                    if matches!(&*ev, epsilon_contracts::agent_event::AgentEvent::AgentStart) {
                        Step::Started
                    } else {
                        // Other work (a finishing previous run, notices) does not
                        // decide this request.
                        Step::Again
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => Step::Again,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    Step::Rejected("event stream closed".to_string())
                }
            },
            res = &mut handle => match res {
                // No AgentStart was observed and the run is already over: the agent
                // rejected the request outright (`AlreadyRunning` is returned before a
                // run's first await), or the task itself failed.
                Ok(Err(e)) => Step::Rejected(e.to_string()),
                Ok(Ok(_)) => Step::Finished,
                Err(join) => Step::Rejected(format!("run task failed: {join}")),
            },
        };
        match step {
            Step::Again => continue,
            settled => break settled,
        }
    };
    match step {
        Step::Started => {
            // A failure *after* admission must not vanish: the old code discarded
            // the run result entirely. The rejection case is already answered in
            // the reply, so only real post-admission failures are logged.
            tokio::spawn(async move {
                if let Ok(Err(e)) = handle.await {
                    eprintln!("rpc: admitted run failed: {e}");
                }
            });
            Admission::Started
        }
        Step::Finished => Admission::Started,
        // Unreachable by construction — the loop above continues on `Again` and only
        // breaks settled — but reported as a rejection rather than panicking, so a
        // future edit cannot turn this into a crash.
        Step::Again => Admission::Rejected("admission was not settled".to_string()),
        Step::Rejected(why) => Admission::Rejected(why),
    }
}

/// Build the user message for `text`, resolving any `@` references (M74). Same constructor the
/// TUI and `-p` use, so a reference behaves identically however the prompt arrives.
fn user_message(agent: &Agent, text: String) -> epsilon_contracts::message::Message {
    epsilon_core::references::user_message(agent.workspace_root(), &text)
}

/// A success reply: pi's envelope plus epsilon's original `ok` key.
///
/// pi's client reads the payload from `data` (`rpc-client.ts:540-590`); epsilon's first
/// wire read `ok`. Both keys carry the same value so neither generation of client needs
/// a translation layer, and `id` is present exactly when the request carried one — a
/// first-wire client's reply is the legacy object extended, never re-keyed.
fn ok_line(command: &str, id: Option<&str>, data: serde_json::Value) -> String {
    let mut frame = serde_json::json!({
        "type": "response",
        "command": command,
        "success": true,
        "data": data.clone(),
        "ok": data,
    });
    if let Some(id) = id {
        frame["id"] = serde_json::Value::String(id.to_string());
    }
    frame.to_string()
}

/// A failure reply: `success:false` with the same `error` key and string as the first wire.
///
/// `command` and `id` are echoed when the request was readable enough to know them; a
/// line that is not parseable JSON carries neither, because guessing a correlation id
/// would be worse than the honest refusal.
fn err_line(command: Option<&str>, id: Option<&str>, msg: impl std::fmt::Display) -> String {
    let mut frame = serde_json::json!({
        "type": "response",
        "success": false,
        "error": msg.to_string(),
    });
    if let Some(command) = command {
        frame["command"] = serde_json::Value::String(command.to_string());
    }
    if let Some(id) = id {
        frame["id"] = serde_json::Value::String(id.to_string());
    }
    frame.to_string()
}

/// A dropped-event notice, matching serve mode's frame.
///
/// HTTP reports a burst the subscriber missed as an SSE event named `lagged` carrying
/// the shared [`LaggedNotice`] as its data (see `epsilon-server`'s `lagged_data`); on the
/// line protocol the frame kind is the top-level key, so the notice is
/// `{"lagged":<notice>}` — the same object, nested. The previous code silently
/// continued, which hid the gap from the client entirely.
fn lag_notice(dropped: u64) -> String {
    serde_json::json!({ "lagged": LaggedNotice { dropped } }).to_string()
}

/// The shared `get_state` payload, read live from `agent`.
///
/// One struct for both wire surfaces ([`SessionState`], in `epsilon-contracts`), so the
/// stdio reply and `GET /state` cannot disagree about shape or values.
fn state_value(agent: &Agent) -> serde_json::Value {
    let state = SessionState {
        model: agent.model_id(),
        thinking_level: agent.thinking_level(),
        is_streaming: agent.is_running(),
        message_count: agent.history().len(),
        pending_message_count: agent.steer_queue_len(),
        session_file: agent.session_path().map(|p| p.display().to_string()),
        estimated_context_tokens: agent.estimated_context_tokens(),
    };
    // Every field is a scalar/string; serialization cannot fail.
    serde_json::to_value(state).unwrap_or_else(|e| serde_json::json!({ "error": e.to_string() }))
}

/// Build the catalog reference a `set_model` frame implies, from pi's pair or a bare id.
///
/// pi sends `provider` and `modelId` as separate fields (`rpc-types.ts:33`), because its
/// catalog keys on the pair. Epsilon's catalog keys on a single `provider/model` string, so
/// the pair is rejoined here rather than by a second lookup path that could disagree with
/// the first. A bare `modelId` with no provider is accepted because a client that only knows
/// the model should not have to know the provider to use it.
fn model_reference(provider: Option<&str>, model_id: Option<&str>) -> Result<String, &'static str> {
    match (provider, model_id) {
        (Some(p), Some(m)) if !p.is_empty() && !m.is_empty() => Ok(format!("{p}/{m}")),
        (_, Some(m)) if !m.is_empty() => Ok(m.to_string()),
        (Some(p), None) if !p.is_empty() => Err("set_model needs modelId, or a model id alone"),
        _ => Err("set_model needs a model: {\"provider\", \"modelId\"} or a bare model id"),
    }
}

/// One catalog row as the wire sees it.
///
/// A projection of [`ModelInfo`] rather than the struct, so the reply carries the fields a
/// client renders (label, limits, prices) and not the internals it cannot use. Prices are
/// micro-USD per million tokens, the same unit the rest of the harness reports, so a client
/// computing cost agrees with the TUI.
fn model_value(info: &epsilon_llm::catalog::ModelInfo) -> serde_json::Value {
    serde_json::json!({
        "id": info.id,
        "provider": info.provider,
        "label": info.label,
        "context_window": info.context_window,
        "max_output_tokens": info.max_output_tokens,
        "input_cost_micro_per_mtok": info.input_cost_micro_per_mtok,
        "output_cost_micro_per_mtok": info.output_cost_micro_per_mtok,
        "cache_read_cost_micro_per_mtok": info.cache_read_cost_micro_per_mtok,
        "cache_write_cost_micro_per_mtok": info.cache_write_cost_micro_per_mtok,
        "supports_tools": info.supports_tools,
    })
}

/// Point pi's `type` tag at `op` so one enum reads both wire dialects (`rpc-types.ts:20-73`).
///
/// `type` is removed as it is copied. Leaving it in place would make a pi-shaped frame look
/// duplicate-tagged to serde (`duplicate field op`), and a frame carrying both tags would
/// fail the same way — `op` is the one that wins, and pi's value is dropped.
fn normalize_tag(value: &mut serde_json::Value) {
    let Some(map) = value.as_object_mut() else {
        return;
    };
    let Some(tag) = map.remove("type") else {
        return;
    };
    if !map.contains_key("op") {
        map.insert("op".to_string(), tag);
    }
}

/// Interpret one request line and return its reply line (no trailing newline).
///
/// Extracted from the stdio loop so the dispatch path is testable in-process: the loop
/// owns stdin/stdout, which is the only reason the old code could not be exercised
/// without a pipe, while every decision worth asserting lives here.
async fn dispatch_line(
    line: &str,
    agent: &Arc<Agent>,
    approvals: &Arc<ChannelApprovalUi>,
) -> String {
    let mut value: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        // Unparseable JSON carries no id: nothing to echo, and guessing one would be
        // worse than the honest failure.
        Err(e) => return err_line(None, None, format!("bad request: {e}")),
    };
    normalize_tag(&mut value);
    let head: RequestHead = match serde_json::from_value(value.clone()) {
        Ok(head) => head,
        Err(e) => return err_line(None, None, format!("bad request: {e}")),
    };
    let id = head.id.as_deref();
    let Some(op_name) = head.op.as_deref() else {
        return err_line(None, id, "bad request: missing \"op\"");
    };
    let op: Op = match serde_json::from_value(value) {
        Ok(op) => op,
        Err(e) => return err_line(Some(op_name), id, format!("bad request: {e}")),
    };

    match op {
        Op::Ping => ok_line("ping", id, serde_json::json!({ "pong": true })),
        Op::Prompt { task } => {
            let prompt = user_message(agent, task);
            match admit_prompt(agent, prompt).await {
                Admission::Started => ok_line("prompt", id, serde_json::json!({ "started": true })),
                Admission::Rejected(why) => err_line(Some("prompt"), id, why),
            }
        }
        Op::Steer { text } => {
            agent.steer(user_message(agent, text));
            ok_line("steer", id, serde_json::json!({ "queued": "steer" }))
        }
        Op::Followup { text } => {
            agent.follow_up(user_message(agent, text));
            ok_line("followup", id, serde_json::json!({ "queued": "followup" }))
        }
        Op::Approvals => ok_line(
            "approvals",
            id,
            serde_json::json!({ "pending": approvals.pending() }),
        ),
        Op::Approve {
            id: ask_id,
            decision,
        } => {
            // The reply's `id` is the correlation header, which for approve requests is
            // the same field as the ask id (see `Op::Approve`) — one value, echoed below.
            if approvals.resolve(&ask_id, decision) {
                ok_line("approve", id, serde_json::json!({ "resolved": true }))
            } else {
                err_line(
                    Some("approve"),
                    id,
                    "unknown or already resolved request id",
                )
            }
        }
        Op::Abort => ok_line(
            "abort",
            id,
            serde_json::json!({ "aborted": agent.abort_current_run() }),
        ),
        Op::GetState => ok_line("get_state", id, state_value(agent)),
        Op::SetThinkingLevel { level } => {
            agent.set_thinking_level(level);
            // Read the level back rather than acknowledging the input: the reply reports
            // what is now in effect, so it cannot claim a change core did not make.
            ok_line(
                "set_thinking_level",
                id,
                serde_json::json!({ "thinking_level": agent.thinking_level() }),
            )
        }
        Op::CycleThinkingLevel => {
            let level = agent.thinking_level().next();
            agent.set_thinking_level(level);
            // Read back, for the same reason `set_thinking_level` does: the reply reports
            // what is in effect, so it cannot claim a change core did not make.
            ok_line(
                "cycle_thinking_level",
                id,
                serde_json::json!({ "thinking_level": agent.thinking_level() }),
            )
        }
        Op::GetAvailableThinkingLevels => ok_line(
            "get_available_thinking_levels",
            id,
            serde_json::json!({
                "levels": epsilon_contracts::ThinkingLevel::ALL
                    .iter()
                    .map(|l| l.as_str())
                    .collect::<Vec<_>>()
            }),
        ),
        Op::SetModel { provider, model_id } => {
            match model_reference(provider.as_deref(), model_id.as_deref()) {
                // A name that resolves to nothing is refused. Silently keeping the old model
                // would leave a client believing it had switched, and the mismatch would
                // surface one request later as a provider error with the wrong model named.
                Err(why) => err_line(Some("set_model"), id, why),
                Ok(reference) => {
                    // `resolve_model_for_run` is the resolver every other entry point uses
                    // (`/model`, `--model`, doctor), so an RPC client reaches the same row a
                    // human would rather than a second, subtly different lookup.
                    let Some(resolved) = epsilon_llm::catalog::resolve_model_for_run(&reference)
                    else {
                        return err_line(
                            Some("set_model"),
                            id,
                            format!("unknown model: {reference}"),
                        );
                    };
                    if !epsilon_llm::catalog::row_credentials_ready(resolved.info) {
                        return err_line(
                            Some("set_model"),
                            id,
                            format!(
                                "no credentials for {} — set {} first",
                                resolved.info.provider,
                                epsilon_llm::catalog::env_key_for(resolved.info.id)
                                    .unwrap_or("the provider key")
                            ),
                        );
                    }
                    agent.set_model_id(resolved.info.id);
                    ok_line(
                        "set_model",
                        id,
                        serde_json::json!({
                            "model": model_value(resolved.info),
                            // The bare id, so a client can round-trip it without reading the object.
                            "model_id": resolved.info.id,
                        }),
                    )
                }
            }
        }
        Op::GetAvailableModels { provider } => {
            let rows: Vec<serde_json::Value> = epsilon_llm::catalog::curated()
                .iter()
                .filter(|m| provider.as_deref().is_none_or(|p| m.provider == p))
                // `credentials_ready` on every row: a model list that offers 40 rows the
                // user cannot run is the same misleading listing the picker avoids by
                // grouping providers.
                .map(|m| {
                    let mut v = model_value(m);
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert(
                            "credentials_ready".into(),
                            serde_json::json!(epsilon_llm::catalog::row_credentials_ready(m)),
                        );
                    }
                    v
                })
                .collect();
            ok_line(
                "get_available_models",
                id,
                serde_json::json!({ "models": rows, "count": rows.len() }),
            )
        }
        Op::CycleModel => {
            let current = agent.model_id();
            let rows = epsilon_llm::catalog::curated();
            // Cycle within the current model's own provider. Crossing providers mid-cycle
            // would land the user on a model with no credentials, and the run would fail on
            // the next request rather than here.
            let current_provider = epsilon_llm::catalog::lookup_model(&current)
                .map(|m| m.provider)
                .unwrap_or("");
            let same: Vec<&epsilon_llm::catalog::ModelInfo> = rows
                .iter()
                .filter(|m| m.provider == current_provider)
                .collect();
            if same.is_empty() {
                return err_line(
                    Some("cycle_model"),
                    id,
                    format!("no catalog rows for the current model: {current}"),
                );
            }
            let at = same
                .iter()
                .position(|m| m.id == current)
                .map(|i| (i + 1) % same.len())
                .unwrap_or(0);
            agent.set_model_id(same[at].id);
            ok_line(
                "cycle_model",
                id,
                serde_json::json!({ "model": model_value(same[at]), "model_id": same[at].id }),
            )
        }
        Op::GetLastAssistantText => {
            // pi returns the last assistant *text*; a message that is only a tool call has
            // none, so the answer is null rather than an empty string. The difference
            // matters: a client using this to decide whether the agent spoke at all would
            // read "" as "it said nothing" and "" as "it said nothing" either way, but null
            // distinguishes "no text" from "text that happened to be empty".
            let text = agent.history().iter().rev().find_map(|m| match m {
                epsilon_contracts::Message::Assistant(a) => {
                    let joined: Vec<String> = a
                        .content
                        .iter()
                        .filter_map(|b| match b {
                            epsilon_contracts::ContentBlock::Text { text } => Some(text.clone()),
                            _ => None,
                        })
                        .collect();
                    (!joined.is_empty()).then(|| joined.join("\n"))
                }
                _ => None,
            });
            ok_line(
                "get_last_assistant_text",
                id,
                serde_json::json!({ "text": text }),
            )
        }
        Op::ClearQueue => {
            // pi's shape is `{ steering, followUp }` (rpc-types.ts:59) -- BOTH queues. Draining
            // only steering would leave every follow-up queued and still report a clear, and
            // follow-ups are the messages that run *after* the agent would otherwise stop, so
            // the user would watch the run continue and have no way to see why.
            let steering = agent.drain_steering_all();
            let follow_up = agent.drain_followups_all();
            ok_line(
                "clear_queue",
                id,
                serde_json::json!({
                    "steering": steering.len(),
                    "followUp": follow_up.len(),
                }),
            )
        }
        Op::GetMessages => {
            // pi returns the conversation as `SessionMessage[]`. Epsilon's `Message` is a
            // serde `tag = "role"` enum, so it lands as an array of role-tagged objects --
            // the same JSON shape a pi client expects, from a different Rust type.
            let history = agent.history();
            ok_line(
                "get_messages",
                id,
                serde_json::json!({ "messages": history }),
            )
        }
        Op::GetCommands => {
            // pi's `RpcSlashCommand` (rpc-types.ts:130-141) carries name, description, and a
            // `sourceInfo` with a path and line range, so a client can offer "open the file
            // that defines this command". Epsilon has no line numbers for skills (the front
            // matter is parsed, not indexed), so `line` is 0 and `endLine` is null rather
            // than a fabricated range -- a client that jumps to a wrong line is worse than
            // one that knows it cannot.
            //
            // `!define` is reported as not-defined rather than omitted: a palette that hides
            // it looks broken when the key does nothing.
            // The TUI's own `picker::SLASH_ITEMS` — already "pi's command registry shape" by
            // its own doc comment, so reading it here means the palette a user sees and the
            // list a client fetches cannot describe different sets of commands. A second
            // hand-maintained table in this file would drift from it, and a client building
            // a command list would then offer commands the harness does not accept.
            let mut commands: Vec<serde_json::Value> = epsilon_tui::picker::SLASH_ITEMS
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "name": c.value.trim_start_matches('/'),
                        "description": c.description,
                        "sourceInfo": serde_json::json!({
                            "path": "builtin",
                            "line": 0,
                            "endLine": serde_json::Value::Null,
                        }),
                    })
                })
                .collect();

            // pi's `get_commands` appends `!define` (`rpc-types.ts:141`), which writes new
            // slash commands into a config file. Epsilon has no equivalent, so it is
            // reported as explicitly not-defined rather than omitted: a palette that hides
            // it looks broken, and one that lists it without the flag would have a client
            // write a file nothing reads.
            commands.push(serde_json::json!({
                "name": "!define",
                "defined": false,
                "description": "Not supported by this build: epsilon has no !define.",
                "sourceInfo": serde_json::Value::Null,
            }));

            for skill in agent.skills() {
                commands.push(serde_json::json!({
                    "name": skill.name,
                    "description": skill.description,
                    "sourceInfo": serde_json::json!({
                        "path": skill.dir.to_string_lossy(),
                        "line": 0,
                        "endLine": serde_json::Value::Null,
                    }),
                }));
            }
            ok_line(
                "get_commands",
                id,
                serde_json::json!({ "commands": commands, "count": commands.len() }),
            )
        }
        Op::GetSessionStats => {
            // pi returns `{ contextTokens, inputTokens, outputTokens, cacheReadTokens,
            // cacheWriteTokens, totalCostUsd }` (`rpc-types.ts:48-56`).
            let usage = agent.session_usage();
            let by_model = agent.session_usage_by_model();

            // Price per model, then sum — the same `breakdown_per_model` the TUI's header
            // uses, fed the same way, so an RPC client and a user looking at the screen see
            // one number rather than two implementations that agree today. Pricing a flat
            // total at the *current* model would under-bill every response a previous model
            // produced, so the per-model split is load-bearing rather than a nicety.
            //
            // `None` when nothing has run: a session that never called a provider has no
            // bill, and inventing `$0.00` would be a fact about a request that never
            // happened. `isFloor` is carried because a model missing from the price
            // registry makes the total a lower bound, and a client showing it as exact
            // would be lying by omission.
            let cost = if by_model.is_empty() {
                None
            } else {
                Some(epsilon_tui::ui::cost::breakdown_per_model(
                    &by_model
                        .iter()
                        .map(|(id, u)| {
                            (
                                id.clone(),
                                epsilon_tui::ui::cost::TokenCounts {
                                    input: u.input,
                                    output: u.output,
                                    cache_read: u.cache_read,
                                    cache_write: u.cache_write,
                                },
                            )
                        })
                        .collect(),
                    |model_id| {
                        epsilon_llm::catalog::lookup_model(model_id).map(|info| {
                            epsilon_tui::ui::cost::TokenPrices {
                                input: info.input_cost_micro_per_mtok,
                                output: info.output_cost_micro_per_mtok,
                                cache_read: info.cache_read_cost_micro_per_mtok,
                                cache_write: info.cache_write_cost_micro_per_mtok,
                            }
                        })
                    },
                ))
            };
            ok_line(
                "get_session_stats",
                id,
                serde_json::json!({
                    "contextTokens": agent.estimated_context_tokens(),
                    "inputTokens": usage.input,
                    "outputTokens": usage.output,
                    "cacheReadTokens": usage.cache_read,
                    "cacheWriteTokens": usage.cache_write,
                    // pi's field is dollars; ours is micro-USD internally, as in the TUI.
                    // Both the exact figure and the flag are reported so a client cannot
                    // mistake a lower bound for a bill.
                    "totalCostUsd": cost.as_ref().map(|c| c.total() as f64 / 1_000_000.0),
                    "costIsFloor": cost.as_ref().is_some_and(|c| c.is_floor()),
                    "models": by_model.keys().collect::<Vec<_>>(),
                }),
            )
        }
        Op::Unknown => err_line(Some(op_name), id, format!("unknown op: {op_name}")),
    }
}

/// Run the stdio loop until stdin closes. Returns the process exit code.
pub async fn run(agent: Arc<Agent>, approvals: Arc<ChannelApprovalUi>) -> i32 {
    let stdin = tokio::io::stdin();
    let mut lines = tokio::io::BufReader::new(stdin).lines();
    let stdout = tokio::io::stdout();
    let writer = Arc::new(tokio::sync::Mutex::new(stdout));

    // Event pump: every lifecycle event becomes an outbound frame; a dropped burst
    // becomes a lagged notice instead of vanishing (RPC-02).
    let mut events_rx = agent.subscribe();
    let pump_writer = Arc::clone(&writer);
    tokio::spawn(async move {
        loop {
            match events_rx.recv().await {
                Ok(ev) => {
                    let frame = format!(
                        "{{\"event\":{}}}\n",
                        serde_json::to_string(&*ev).unwrap_or_else(|_| "null".into())
                    );
                    let mut w = pump_writer.lock().await;
                    w.write_all(frame.as_bytes()).await.ok();
                    w.flush().await.ok();
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(dropped)) => {
                    let frame = format!("{}\n", lag_notice(dropped));
                    let mut w = pump_writer.lock().await;
                    w.write_all(frame.as_bytes()).await.ok();
                    w.flush().await.ok();
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let reply = dispatch_line(&line, &agent, &approvals).await;

        let mut w = writer.lock().await;
        w.write_all(reply.as_bytes()).await.ok();
        w.write_all(b"\n").await.ok();
        w.flush().await.ok();
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An agent whose provider pauses `delay_ms` before each scripted step, so a run
    /// stays in flight while the next request is dispatched; `0` behaves like the plain
    /// faux provider.
    fn agent_with(steps: Vec<epsilon_llm::FauxStep>, delay_ms: u64) -> Arc<Agent> {
        epsilon_core::Agent::new(
            epsilon_core::LoopLimits::default(),
            Arc::new(epsilon_llm::PanicGuard(
                epsilon_core::testing::SlowProvider::new(
                    epsilon_llm::FauxProvider::new(steps),
                    delay_ms,
                ),
            )),
            Arc::new(epsilon_core::ToolRegistry::new()),
            Arc::new(epsilon_core::AutoAllow),
            std::env::temp_dir(),
        )
    }

    fn approvals() -> Arc<ChannelApprovalUi> {
        Arc::new(ChannelApprovalUi::new())
    }

    fn parse(reply: &str) -> serde_json::Value {
        serde_json::from_str(reply).unwrap_or_else(|e| panic!("reply is not one JSON line: {e}"))
    }

    /// Consume events until the run's terminal `AgentEnd`, or fail.
    ///
    /// Waiting for the EVENT rather than for `history()` to look settled is deliberate and
    /// was learned the hard way in M186: a run appends messages and *then* emits `AgentEnd`,
    /// so a poll can observe a half-built transcript and report numbers that are about to
    /// change. The event is the only moment the run's accounting is final.
    async fn wait_for_end(
        events: &mut tokio::sync::broadcast::Receiver<
            Arc<epsilon_contracts::agent_event::AgentEvent>,
        >,
    ) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(Ok(ev)) => {
                    if matches!(
                        &*ev,
                        epsilon_contracts::agent_event::AgentEvent::AgentEnd { .. }
                    ) {
                        return;
                    }
                }
                Ok(Err(e)) => panic!("event stream failed before the run ended: {e}"),
                Err(_) => panic!("the run did not reach its terminal event within 10s"),
            }
        }
    }

    #[test]
    fn ops_deserialize_with_snake_tags() {
        let p: Op = serde_json::from_str(r#"{"op":"prompt","task":"x"}"#).unwrap();
        assert!(matches!(p, Op::Prompt { .. }));
        let a: Op =
            serde_json::from_str(r#"{"op":"approve","id":"p1","decision":{"type":"allow_once"}}"#)
                .unwrap();
        assert!(matches!(a, Op::Approve { .. }));
        assert!(matches!(
            serde_json::from_str::<Op>(r#"{"op":"get_state"}"#).unwrap(),
            Op::GetState
        ));
        assert!(matches!(
            serde_json::from_str::<Op>(r#"{"op":"set_thinking_level","level":"high"}"#).unwrap(),
            Op::SetThinkingLevel { .. }
        ));
        // pi's one-word spelling is accepted at the op level too (serde alias on the
        // XHigh variant; the canonical serde id stays `x_high`).
        assert!(matches!(
            serde_json::from_str::<Op>(r#"{"op":"set_thinking_level","level":"xhigh"}"#).unwrap(),
            Op::SetThinkingLevel {
                level: epsilon_contracts::ThinkingLevel::XHigh
            }
        ));
        assert!(matches!(
            serde_json::from_str::<Op>(r#"{"op":"abort"}"#).unwrap(),
            Op::Abort
        ));
        // An op this build does not implement is a named variant, not a parse crash:
        // the dispatch path reports it by name (see the unknown-op test below).
        //
        // `set_model` USED to be the example here, and stopped being one in M186 when it
        // was implemented. `set_auto_compaction` takes its place -- still genuinely
        // unimplemented, so the assertion is still about the real fallback rather than a
        // stale claim. A test whose example silently stops being true is worse than no
        // example, because it keeps passing for the wrong reason.
        assert!(matches!(
            serde_json::from_str::<Op>(r#"{"op":"set_auto_compaction","enabled":true}"#).unwrap(),
            Op::Unknown
        ));
        // The ops M186 added are real variants, with their fields parsed.
        assert!(matches!(
            serde_json::from_str::<Op>(
                r#"{"op":"set_model","provider":"anthropic","model_id":"m"}"#
            )
            .unwrap(),
            Op::SetModel { .. }
        ));
        assert!(matches!(
            serde_json::from_str::<Op>(r#"{"op":"cycle_thinking_level"}"#).unwrap(),
            Op::CycleThinkingLevel
        ));
        assert!(matches!(
            serde_json::from_str::<Op>(r#"{"op":"get_available_models"}"#).unwrap(),
            Op::GetAvailableModels { .. }
        ));
        // `set_model` with no model still parses (both fields are optional) so the DISPATCH
        // can refuse it with a message about the missing field, rather than serde reporting
        // a parse error. A missing argument is a client mistake worth naming, not a
        // malformed frame.
        assert!(matches!(
            serde_json::from_str::<Op>(r#"{"op":"set_model"}"#).unwrap(),
            Op::SetModel { .. }
        ));
    }

    #[test]
    fn replies_are_single_lines_and_keep_the_legacy_keys() {
        let ok = ok_line("ping", Some("req_1"), serde_json::json!({"pong": true}));
        assert!(!ok.contains('\n'));
        let v = parse(&ok);
        assert_eq!(v["type"], "response");
        assert_eq!(v["command"], "ping");
        assert_eq!(v["success"], true);
        assert_eq!(v["id"], "req_1");
        assert_eq!(v["ok"]["pong"], true, "the first wire's key");
        assert_eq!(
            v["data"]["pong"], true,
            "pi reads `data` (rpc-client.ts:540-590)"
        );

        let err = err_line(Some("prompt"), Some("req_2"), "boom");
        assert!(!err.contains('\n'));
        let v = parse(&err);
        assert_eq!(v["type"], "response");
        assert_eq!(v["command"], "prompt");
        assert_eq!(v["success"], false);
        assert_eq!(v["id"], "req_2");
        assert_eq!(v["error"], "boom");

        // No id on the request -> no id on the reply (a first-wire client's shape).
        let v = parse(&ok_line("ping", None, serde_json::json!({"pong": true})));
        assert!(v.get("id").is_none(), "{v}");
    }

    #[test]
    fn lagged_notice_matches_the_http_frame_shape() {
        // Both surfaces build the notice from the shared `LaggedNotice` (serve's SSE
        // `lagged` event data is the same struct serialized); this pins the frame and
        // the bytes.
        let v = parse(&lag_notice(7));
        assert_eq!(
            lag_notice(7),
            r#"{"lagged":{"dropped":7}}"#,
            "byte-identical to the pre-shared form"
        );
        assert_eq!(
            v["lagged"],
            serde_json::json!({ "dropped": 7 }),
            "the wire bytes are unchanged"
        );
        assert_eq!(
            v["lagged"],
            serde_json::to_value(LaggedNotice { dropped: 7 }).unwrap(),
            "and they are the shared contract type's serialization"
        );
    }

    /// RPC-02: each request's id comes back on its own reply, in request order (this is
    /// pi's contract — `rpc-client.ts:508-522` — where the old wire correlated by
    /// arrival order alone).
    #[tokio::test]
    async fn ids_echo_per_request_in_arrival_order() {
        let agent = agent_with(vec![epsilon_llm::FauxStep::say("hi")], 400);
        let approvals = approvals();

        let first = dispatch_line(
            r#"{"id":"req_1","op":"prompt","task":"one"}"#,
            &agent,
            &approvals,
        )
        .await;
        let second = dispatch_line(
            r#"{"id":"req_2","op":"prompt","task":"two"}"#,
            &agent,
            &approvals,
        )
        .await;

        let first = parse(&first);
        let second = parse(&second);
        assert_eq!(first["id"], "req_1");
        assert_eq!(second["id"], "req_2");
        assert_eq!(first["command"], "prompt");
        assert_eq!(second["command"], "prompt");
        assert_eq!(first["success"], true, "the first request owns the run");
        assert_eq!(
            second["success"], false,
            "the second must not get a false success"
        );
        assert!(
            second["error"].as_str().unwrap().contains("another run"),
            "{}",
            second["error"]
        );

        assert!(agent.abort_current_run());
    }

    /// pi's client sends `type` where epsilon sends `op`; the same frame must dispatch.
    #[tokio::test]
    async fn pi_type_tag_is_read_as_op() {
        let agent = agent_with(vec![epsilon_llm::FauxStep::say("hi")], 0);
        let reply =
            dispatch_line(r#"{"id":"req_7","type":"get_state"}"#, &agent, &approvals()).await;
        let v = parse(&reply);
        assert_eq!(v["success"], true, "{}", reply);
        assert_eq!(v["id"], "req_7");
        assert_eq!(v["command"], "get_state");
    }

    /// A frame carrying BOTH tags must not trip serde's duplicate-field check
    /// (`normalize_tag` removes `type` before pointing it at `op`), and when the two
    /// disagree, `op` wins — the documented precedence, not an error.
    #[tokio::test]
    async fn both_tags_on_one_frame_are_read_as_one_op() {
        let agent = agent_with(vec![epsilon_llm::FauxStep::say("hi")], 0);
        let approvals = approvals();

        // The same tag in both dialects: pi's `type` and epsilon's `op` on one frame.
        let v = parse(&dispatch_line(r#"{"type":"ping","op":"ping"}"#, &agent, &approvals).await);
        assert_eq!(v["success"], true, "{v}");
        assert_eq!(v["command"], "ping");

        // Disagreeing tags: `op` is the one that wins; pi's value is dropped, not read
        // as a second tag.
        let v =
            parse(&dispatch_line(r#"{"type":"get_state","op":"ping"}"#, &agent, &approvals).await);
        assert_eq!(v["success"], true, "{v}");
        assert_eq!(v["command"], "ping", "op wins when both tags are present");
    }

    /// A first-wire request (no id, `op` tag only) still gets its old keys and no `id`.
    #[tokio::test]
    async fn legacy_requests_still_work_without_ids() {
        let agent = agent_with(vec![epsilon_llm::FauxStep::say("hi")], 0);
        let v = parse(&dispatch_line(r#"{"op":"ping"}"#, &agent, &approvals()).await);
        assert_eq!(v["ok"]["pong"], true);
        assert_eq!(v["success"], true);
        assert_eq!(v["command"], "ping");
        assert!(v.get("id").is_none(), "{v}");
    }

    /// RPC-02: an unimplemented op fails explicitly, names itself, and echoes the id —
    /// including when it arrives pi-shaped.
    #[tokio::test]
    async fn unknown_op_reports_failure_with_the_echoed_id() {
        let agent = agent_with(vec![epsilon_llm::FauxStep::say("hi")], 0);
        let approvals = approvals();

        // M186 replaced every example this test used to carry. `set_model`, `cycle_model`
        // and the pi-shaped `modelId` frame are all implemented now, so using them as
        // "unknown op" examples would have kept asserting something false. The point of
        // the test is the FALLBACK, and the examples must therefore be ops that are still
        // genuinely unimplemented -- which is a smaller and shrinking set, so the next
        // person to implement one has to update this too. That is the intended friction.

        let v =
            parse(&dispatch_line(r#"{"id":"req_9","op":"get_tree"}"#, &agent, &approvals).await);
        assert_eq!(v["success"], false, "{v}");
        assert_eq!(v["id"], "req_9", "the correlation id still echoes: {v}");
        assert_eq!(v["command"], "get_tree");
        assert_eq!(v["error"], "unknown op: get_tree");

        // The same op under pi's `type` tag: both dialects reach the same fallback, so a
        // pi-shaped client is not the one that gets a parse error instead of a named one.
        let v = parse(
            &dispatch_line(
                r#"{"id":"req_10","type":"export_html"}"#,
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(v["success"], false, "{v}");
        assert_eq!(v["id"], "req_10", "{v}");
        assert_eq!(v["error"], "unknown op: export_html");

        // A pi-shaped unknown op carrying a realistic payload: unknown fields are
        // tolerated (serde's internally tagged unit variants ignore the rest of the
        // map), so the reply is still the named `unknown op` with the echoed id —
        // never a serde parse error.
        let v = parse(
            &dispatch_line(
                r#"{"id":"req_11","type":"fork","entryId":"e1"}"#,
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(v["success"], false, "{v}");
        assert_eq!(v["id"], "req_11", "{v}");
        assert_eq!(v["command"], "fork");
        assert_eq!(v["error"], "unknown op: fork");
    }

    /// A malformed line still fails in the legacy vocabulary (`error`), and a known op
    /// with a bad payload keeps the `bad request:` prefix.
    #[tokio::test]
    async fn malformed_lines_fail_without_inventing_a_correlation() {
        let agent = agent_with(vec![epsilon_llm::FauxStep::say("hi")], 0);
        let approvals = approvals();

        let v = parse(&dispatch_line("not json", &agent, &approvals).await);
        assert_eq!(v["success"], false);
        assert!(v["error"].as_str().unwrap().starts_with("bad request:"));
        assert!(v.get("id").is_none(), "{v}");

        let v = parse(&dispatch_line(r#"{"op":"prompt"}"#, &agent, &approvals).await);
        assert_eq!(v["success"], false);
        assert_eq!(v["command"], "prompt");
        assert!(v["error"].as_str().unwrap().starts_with("bad request:"));

        // A well-formed object with neither tag: the id is readable, the op is not.
        let v = parse(&dispatch_line(r#"{"id":"req_3"}"#, &agent, &approvals).await);
        assert_eq!(v["success"], false);
        assert_eq!(v["id"], "req_3");
        assert_eq!(v["error"], "bad request: missing \"op\"");

        let v = parse(
            &dispatch_line(
                r#"{"op":"set_thinking_level","level":"ludicrous"}"#,
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(v["success"], false);
        assert_eq!(v["command"], "set_thinking_level");
        assert!(v["error"].as_str().unwrap().contains("ludicrous"), "{v}");
    }

    /// RPC-02: abort with no run reports core's `false` — never a fabricated success.
    #[tokio::test]
    async fn abort_on_an_idle_agent_returns_cores_false() {
        let agent = agent_with(vec![epsilon_llm::FauxStep::say("hi")], 0);
        let reply = dispatch_line(r#"{"id":"req_1","op":"abort"}"#, &agent, &approvals()).await;
        let v = parse(&reply);
        assert_eq!(v["success"], true, "the request was served: {reply}");
        assert_eq!(v["command"], "abort");
        assert_eq!(v["ok"]["aborted"], false, "{reply}");
        assert_eq!(v["data"]["aborted"], false);
        assert!(!agent.is_running());
    }

    /// RPC-02: abort during an admitted run reports core's `true` and the run actually
    /// ends — proven by its terminal event, not by a poll that could race it.
    #[tokio::test]
    async fn abort_during_an_admitted_run_reports_true_and_ends_it() {
        let agent = agent_with(vec![epsilon_llm::FauxStep::say("hi")], 400);
        let approvals = approvals();
        let mut events = agent.subscribe();

        let v = parse(
            &dispatch_line(
                r#"{"id":"req_1","op":"prompt","task":"slow work"}"#,
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(v["ok"]["started"], true, "{v}");
        assert!(agent.is_running(), "the slow provider holds the run");

        let v = parse(&dispatch_line(r#"{"id":"req_2","op":"abort"}"#, &agent, &approvals).await);
        assert_eq!(v["ok"]["aborted"], true, "{v}");

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut ended = false;
        while !ended {
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(Ok(ev)) => {
                    ended = matches!(
                        &*ev,
                        epsilon_contracts::agent_event::AgentEvent::AgentEnd { .. }
                    );
                }
                Ok(Err(e)) => panic!("event stream failed before the run ended: {e}"),
                Err(_) => break,
            }
        }
        assert!(ended, "the aborted run must reach its terminal event");
        assert!(!agent.is_running());
    }

    /// RPC-02: `get_state` reports the live agent's own values through the shared
    /// [`SessionState`] struct.
    #[tokio::test]
    async fn get_state_values_equal_the_live_agents() {
        use epsilon_contracts::content::UserContent;
        use epsilon_contracts::message::{Message, UserMessage};

        let agent = agent_with(vec![epsilon_llm::FauxStep::say("hi")], 0);
        agent.set_model_id("m92-test-model");
        agent.set_thinking_level(epsilon_contracts::ThinkingLevel::Medium);
        agent.steer(Message::User(UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: 0,
            content: vec![UserContent::Text {
                text: "queued".to_string(),
            }],
        }));

        let reply = dispatch_line(r#"{"id":"req_1","op":"get_state"}"#, &agent, &approvals()).await;
        let v = parse(&reply);
        let ok = &v["ok"];
        // The literal below is the pin: comparing against `agent.model_id()` here would
        // be tautological (the payload is built from that same getter).
        assert_eq!(ok["model"], "m92-test-model");
        assert_eq!(ok["thinking_level"], "medium");
        assert_eq!(ok["is_streaming"], false);
        assert_eq!(
            ok["message_count"].as_u64().unwrap() as usize,
            agent.history().len()
        );
        assert_eq!(ok["pending_message_count"], 1);
        assert_eq!(ok["session_file"], serde_json::Value::Null);
        assert_eq!(
            ok["estimated_context_tokens"].as_u64().unwrap() as usize,
            agent.estimated_context_tokens()
        );
        assert_eq!(v["data"], v["ok"], "both keys carry the shared payload");
    }

    /// RPC-02: `set_thinking_level` reports the level now in effect (read back, not
    /// echoed), and the next `get_state` agrees. The agent starts at a *different*
    /// level, so an echo of the request could not satisfy the assertions: the live
    /// state must actually have changed.
    #[tokio::test]
    async fn set_thinking_level_round_trips_through_the_agent() {
        let agent = agent_with(vec![epsilon_llm::FauxStep::say("hi")], 0);
        let approvals = approvals();
        agent.set_thinking_level(epsilon_contracts::ThinkingLevel::Low);

        let v = parse(
            &dispatch_line(
                r#"{"op":"set_thinking_level","level":"high"}"#,
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(v["ok"]["thinking_level"], "high");
        assert_eq!(
            agent.thinking_level(),
            epsilon_contracts::ThinkingLevel::High,
            "the agent's state actually changed"
        );
        assert_ne!(
            agent.thinking_level(),
            epsilon_contracts::ThinkingLevel::Low,
            "not merely the pre-existing level echoed back"
        );

        let v = parse(&dispatch_line(r#"{"op":"get_state"}"#, &agent, &approvals).await);
        assert_eq!(v["ok"]["thinking_level"], "high");
    }

    /// Review B1: pi's spelling `xhigh` is accepted on input (serde alias) while the
    /// outbound vocabulary keeps the canonical `x_high` — the session-persistence id —
    /// so the supported loop is: a pi client sends `xhigh`, reads `x_high` back.
    #[tokio::test]
    async fn set_thinking_level_accepts_pis_xhigh_spelling() {
        let agent = agent_with(vec![epsilon_llm::FauxStep::say("hi")], 0);
        let approvals = approvals();
        assert_eq!(
            agent.thinking_level(),
            epsilon_contracts::ThinkingLevel::Off
        );

        let v = parse(
            &dispatch_line(
                r#"{"id":"req_1","type":"set_thinking_level","level":"xhigh"}"#,
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(v["success"], true, "{v}");
        assert_eq!(
            agent.thinking_level(),
            epsilon_contracts::ThinkingLevel::XHigh
        );
        assert_eq!(
            v["ok"]["thinking_level"], "x_high",
            "outbound spelling stays the canonical persistence id, not pi's alias"
        );

        // Our own outbound spelling is accepted back on the next request.
        let v = parse(
            &dispatch_line(
                r#"{"op":"set_thinking_level","level":"x_high"}"#,
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(v["success"], true, "{v}");
        assert_eq!(
            agent.thinking_level(),
            epsilon_contracts::ThinkingLevel::XHigh
        );
    }

    /// RPC-01: overlapping prompts must not both receive success. Admission is
    /// decided by the run itself, so the second request receives an attributable
    /// rejection while the first is active — and admission frees when it ends.
    #[tokio::test]
    async fn a_second_prompt_is_rejected_while_a_run_is_admitted() {
        use epsilon_contracts::content::UserContent;
        use epsilon_contracts::message::{Message, UserMessage};

        let agent = agent_with(vec![epsilon_llm::FauxStep::say("hi")], 300);
        let prompt = |text: &str| {
            Message::User(UserMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: 0,
                content: vec![UserContent::Text {
                    text: text.to_string(),
                }],
            })
        };

        let first = admit_prompt(&agent, prompt("one")).await;
        assert_eq!(first, Admission::Started, "the first request is admitted");

        match admit_prompt(&agent, prompt("two")).await {
            Admission::Rejected(why) => assert!(
                why.contains("another run"),
                "the rejection must name the real reason: {why}"
            ),
            Admission::Started => panic!("rejected work must not receive a false success"),
        }

        // Ending the run releases admission: the rejection described the active
        // run, not a permanent state.
        assert!(agent.abort_current_run());
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut admitted_again = false;
        while tokio::time::Instant::now() < deadline {
            if admit_prompt(&agent, prompt("three")).await == Admission::Started {
                admitted_again = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            admitted_again,
            "admission must be released when the run ends"
        );
        let _ = agent.abort_current_run();
    }

    // ---- M186: the pi RPC commands that map onto existing core capability ----
    //
    // These six were `#[serde(other)]` -> `unknown op` until now, despite the core
    // capability already existing (`ThinkingLevel::ALL`/`.next()`, `catalog::curated()`,
    // `Agent::set_model_id`, the history). RPC-01 closed "a half-wired acknowledgement
    // pretending to work"; these are the ops that could be wired *completely*, so they
    // are now.

    /// The ladder is reported, and `cycle_thinking_level` walks exactly it.
    ///
    /// Asserted against `ThinkingLevel::ALL` rather than a hardcoded list, so adding a
    /// level cannot make the reply quietly wrong.
    #[tokio::test]
    async fn available_thinking_levels_match_the_canonical_ladder() {
        let agent = agent_with(vec![], 0);
        let reply = parse(
            &dispatch_line(
                r#"{"id":"r1","op":"get_available_thinking_levels"}"#,
                &agent,
                &approvals(),
            )
            .await,
        );

        assert_eq!(reply["success"], true, "{reply}");
        assert_eq!(reply["id"], "r1", "the id echoes: {reply}");
        let got: Vec<String> = reply["data"]["levels"]
            .as_array()
            .expect("an array")
            .iter()
            .map(|v| v.as_str().unwrap_or_default().to_string())
            .collect();
        let want: Vec<String> = epsilon_contracts::ThinkingLevel::ALL
            .iter()
            .map(|l| l.as_str().to_string())
            .collect();
        assert_eq!(got, want, "the wire list is the ladder, in order");
    }

    /// Cycling changes the level and reports what is in effect, not what was requested.
    #[tokio::test]
    async fn cycle_thinking_level_advances_and_reports_the_new_level() {
        let agent = agent_with(vec![], 0);
        let approvals = approvals();
        let start = agent.thinking_level();

        let reply =
            parse(&dispatch_line(r#"{"op":"cycle_thinking_level"}"#, &agent, &approvals).await);
        assert_eq!(reply["success"], true, "{reply}");
        assert_eq!(
            reply["data"]["thinking_level"],
            serde_json::json!(start.next()),
            "the reply names the level now in effect"
        );
        assert_eq!(
            agent.thinking_level(),
            start.next(),
            "and core agrees -- the reply is a read-back, not an echo of the request"
        );
    }

    /// Cycling wraps. A ladder that stops at the top would leave a client unable to
    /// return to `off` without a second command.
    #[tokio::test]
    async fn cycle_thinking_level_wraps_at_the_top() {
        let agent = agent_with(vec![], 0);
        let approvals = approvals();
        agent.set_thinking_level(epsilon_contracts::ThinkingLevel::Max);
        let reply =
            parse(&dispatch_line(r#"{"op":"cycle_thinking_level"}"#, &agent, &approvals).await);
        assert_eq!(
            reply["data"]["thinking_level"], "off",
            "max wraps to off: {reply}"
        );
    }

    /// pi's `type` tag works on the new ops too, not just the old ones.
    ///
    /// Worth its own test because `normalize_tag` is shared plumbing: a new variant that
    /// only worked under `op` would be a silent wire incompatibility for exactly the
    /// clients this port exists to serve.
    #[tokio::test]
    async fn new_ops_accept_pis_type_tag() {
        let agent = agent_with(vec![], 0);
        let reply = parse(
            &dispatch_line(
                r#"{"id":"r7","type":"get_available_thinking_levels"}"#,
                &agent,
                &approvals(),
            )
            .await,
        );
        assert_eq!(reply["success"], true, "pi's tag is accepted: {reply}");
        assert_eq!(reply["id"], "r7", "{reply}");
        assert!(
            reply["data"]["levels"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
        );
    }

    /// A model name that resolves to nothing is refused rather than silently ignored.
    ///
    /// The alternative -- keeping the old model and replying `success` -- is the failure
    /// this guards: a client would believe it had switched, and the mismatch would appear
    /// one request later as a provider error naming a model the user never chose.
    #[tokio::test]
    async fn set_model_refuses_an_unresolvable_name() {
        let agent = agent_with(vec![], 0);
        let before = agent.model_id();
        let reply = parse(
            &dispatch_line(
                r#"{"op":"set_model","model_id":"definitely-not-a-real-model"}"#,
                &agent,
                &approvals(),
            )
            .await,
        );
        assert_eq!(reply["success"], false, "{reply}");
        assert_eq!(agent.model_id(), before, "the model is unchanged");
    }

    /// `set_model` with no model at all is a bad request, not a silent no-op.
    #[tokio::test]
    async fn set_model_without_a_model_is_refused() {
        let agent = agent_with(vec![], 0);
        let reply = parse(&dispatch_line(r#"{"op":"set_model"}"#, &agent, &approvals()).await);
        assert_eq!(reply["success"], false, "{reply}");
    }

    /// The provider/model pair pi sends is rejoined, so both wire dialects reach the same
    /// catalog row.
    #[test]
    fn model_reference_accepts_both_wire_shapes() {
        assert_eq!(
            model_reference(Some("anthropic"), Some("claude-sonnet-4")),
            Ok("anthropic/claude-sonnet-4".to_string())
        );
        assert_eq!(
            model_reference(None, Some("claude-sonnet-4")),
            Ok("claude-sonnet-4".to_string()),
            "a bare model id is enough"
        );
        assert!(
            model_reference(Some("anthropic"), None).is_err(),
            "a provider with no model cannot identify a row"
        );
        assert!(model_reference(None, None).is_err());
        assert!(
            model_reference(Some(""), Some("")).is_err(),
            "empty strings are not a model"
        );
    }

    /// The model list carries the fields a client renders, and marks credentials.
    #[tokio::test]
    async fn available_models_carry_renderable_fields_and_credential_state() {
        let agent = agent_with(vec![], 0);
        let reply = parse(
            &dispatch_line(
                r#"{"op":"get_available_models","provider":"anthropic"}"#,
                &agent,
                &approvals(),
            )
            .await,
        );
        assert_eq!(reply["success"], true, "{reply}");
        let models = reply["data"]["models"].as_array().expect("an array");
        assert!(!models.is_empty(), "anthropic has catalog rows");
        for m in models {
            assert_eq!(m["provider"], "anthropic", "the filter is honoured: {m}");
            for field in ["id", "label", "context_window", "max_output_tokens"] {
                assert!(!m[field].is_null(), "{field} is present: {m}");
            }
            assert!(
                m["credentials_ready"].is_boolean(),
                "a client needs to know which rows it can actually run: {m}"
            );
        }
        assert_eq!(reply["data"]["count"], models.len(), "{reply}");
    }

    /// `get_last_assistant_text` returns null when the agent has not spoken.
    ///
    /// Null rather than "": a client deciding whether the agent produced an answer cannot
    /// tell "no text" from "text that was empty", and the two mean different things.
    #[tokio::test]
    async fn last_assistant_text_is_null_before_any_reply() {
        let agent = agent_with(vec![], 0);
        let reply = parse(
            &dispatch_line(r#"{"op":"get_last_assistant_text"}"#, &agent, &approvals()).await,
        );
        assert_eq!(reply["success"], true, "{reply}");
        assert_eq!(reply["data"]["text"], serde_json::Value::Null, "{reply}");
    }

    /// After a run, the text is the assistant's, not the user's.
    #[tokio::test]
    async fn last_assistant_text_is_the_assistants_reply() {
        let agent = agent_with(vec![epsilon_llm::FauxStep::say("the answer is 42")], 0);
        let approvals = approvals();
        // Wait for the terminal EVENT, not for `history()` to be non-empty. Polling
        // history was my first attempt and it is a race: a run appends messages and then
        // emits `AgentEnd`, so a poll can observe a half-built transcript. The event is the
        // moment the history is settled, which is the only moment "the last assistant
        // text" is a meaningful answer. The same pattern the abort test above uses.
        let mut events = agent.subscribe();
        let started = parse(
            &dispatch_line(
                r#"{"op":"prompt","task":"what is it?"}"#,
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(started["data"]["started"], true, "{started}");
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut ended = false;
        while !ended {
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(Ok(ev)) => {
                    ended = matches!(
                        &*ev,
                        epsilon_contracts::agent_event::AgentEvent::AgentEnd { .. }
                    );
                }
                Ok(Err(e)) => panic!("event stream failed before the run ended: {e}"),
                Err(_) => break,
            }
        }
        assert!(ended, "the run must reach its terminal event");
        let reply =
            parse(&dispatch_line(r#"{"op":"get_last_assistant_text"}"#, &agent, &approvals).await);
        assert_eq!(reply["success"], true, "{reply}");
        assert_eq!(reply["data"]["text"], "the answer is 42", "{reply}");
    }

    /// The SUCCESS path of `set_model`, which the first version of this slice never
    /// tested — and the control caught that.
    ///
    /// Removing `agent.set_model_id(...)` from the handler left all 25 RPC tests passing,
    /// because every `set_model` test I had written asserted a FAILURE: an unresolvable
    /// name, a missing model, a bare parse. A handler that refuses everything correctly
    /// passes all of them. Fifth time this session, and the second time a control rather
    /// than a reading of the code found it.
    ///
    /// `faux-1` is the catalog row that needs no credentials (`row_credentials_ready`
    /// treats an empty `env_key` as always-runnable), so the readiness gate is exercised
    /// on its passing side rather than bypassed.
    ///
    /// The agent is parked on a DIFFERENT id first. The default is already `faux-1`, and a
    /// test that sets the model to the value it already holds would pass against a handler
    /// that does nothing — which is exactly the control. A precondition that the test
    /// depends on has to be the thing the control would break.
    #[tokio::test]
    async fn set_model_switches_the_model_and_reports_the_new_one() {
        let agent = agent_with(vec![], 0);
        agent.set_model_id("m186-starting-model");
        assert_ne!(
            agent.model_id(),
            "faux-1",
            "the precondition: the agent must start elsewhere, or this asserts nothing"
        );

        let reply = parse(
            &dispatch_line(
                r#"{"id":"req_1","op":"set_model","provider":"faux","model_id":"faux-1"}"#,
                &agent,
                &approvals(),
            )
            .await,
        );
        assert_eq!(reply["success"], true, "{reply}");
        assert_eq!(reply["id"], "req_1", "{reply}");
        assert_eq!(reply["data"]["model_id"], "faux-1", "{reply}");
        // THE ASSERTION THE CONTROL WAS CAUGHT BY: core's own state changed.
        assert_eq!(
            agent.model_id(),
            "faux-1",
            "core must actually hold the new model -- a success reply is not evidence"
        );
        // And `get_state` agrees, so a client that only polls state sees the switch too.
        let state = parse(&dispatch_line(r#"{"op":"get_state"}"#, &agent, &approvals()).await);
        assert_eq!(state["data"]["model"], "faux-1", "{state}");
    }

    /// pi's bare-model spelling reaches the same row as the provider/model pair.
    #[tokio::test]
    async fn set_model_accepts_a_bare_model_id() {
        let agent = agent_with(vec![], 0);
        agent.set_model_id("m186-starting-model");
        let reply = parse(
            &dispatch_line(
                r#"{"op":"set_model","model_id":"faux-1"}"#,
                &agent,
                &approvals(),
            )
            .await,
        );
        assert_eq!(reply["success"], true, "{reply}");
        assert_eq!(agent.model_id(), "faux-1", "{reply}");
    }

    /// The model list narrows by provider, so a client can populate a picker.
    #[tokio::test]
    async fn available_models_can_be_filtered_by_provider() {
        let agent = agent_with(vec![], 0);
        let all =
            parse(&dispatch_line(r#"{"op":"get_available_models"}"#, &agent, &approvals()).await);
        let faux = parse(
            &dispatch_line(
                r#"{"op":"get_available_models","provider":"faux"}"#,
                &agent,
                &approvals(),
            )
            .await,
        );
        let n_all = all["data"]["models"].as_array().unwrap().len();
        let n_faux = faux["data"]["models"].as_array().unwrap().len();
        assert!(
            n_faux < n_all,
            "the filter narrows the list: {n_faux} vs {n_all}"
        );
        assert!(n_faux >= 1, "and faux has at least one row");
    }

    /// Cycling moves to another row in the SAME provider and never crosses providers.
    ///
    /// Crossing providers mid-cycle would park the user on a model with no credentials, and
    /// the failure would surface on the next request rather than here — so the assertion is
    /// that the provider is unchanged, not merely that the id changed.
    #[tokio::test]
    async fn cycle_model_stays_within_the_current_provider() {
        let agent = agent_with(vec![], 0);
        agent.set_model_id("faux-1");
        let reply = parse(&dispatch_line(r#"{"op":"cycle_model"}"#, &agent, &approvals()).await);
        assert_eq!(reply["success"], true, "{reply}");
        let provider = reply["data"]["model"]["provider"].as_str().unwrap_or("");
        assert_eq!(provider, "faux", "the provider must not change: {reply}");
        assert_eq!(
            agent.model_id(),
            reply["data"]["model_id"].as_str().unwrap_or(""),
            "core holds the cycled model: {reply}"
        );
    }

    // ---- M188: clear_queue, get_messages, get_commands, get_session_stats ----
    //
    // These four were refused as `unknown op` not because the work was missing but because
    // the numbers `get_session_stats` needs lived in the TUI's `UiState` — a view that RPC
    // has no instance of. The fix moved the session total onto `Agent`, which is where a
    // fact about a session belongs.

    /// A fresh agent has spent nothing, and says so with `null` rather than `$0.00`.
    ///
    /// A session that never called a provider has no bill. Reporting zero would be a fact
    /// about a request that never happened, and a client cannot tell it from a free run.
    #[tokio::test]
    async fn session_stats_cost_is_null_before_anything_runs() {
        let agent = agent_with(vec![], 0);
        let approvals = approvals();
        let reply =
            parse(&dispatch_line(r#"{"op":"get_session_stats"}"#, &agent, &approvals).await);
        assert_eq!(reply["success"], true, "{reply}");
        assert_eq!(reply["data"]["inputTokens"], 0, "{reply}");
        assert_eq!(reply["data"]["outputTokens"], 0, "{reply}");
        assert_eq!(
            reply["data"]["totalCostUsd"],
            serde_json::Value::Null,
            "an unrun session has no bill to report: {reply}"
        );
    }

    /// After a run the totals are the ones core accumulated, and `contextTokens` is live.
    ///
    /// The control for this test removes the `session_usage` fold-in. It must fail here: a
    /// handler that echoed the reply it was given, or that re-derived zeros, would otherwise
    /// pass a "before" test and fail nothing.
    #[tokio::test]
    async fn session_stats_report_the_run_core_actually_recorded() {
        let agent = agent_with(vec![epsilon_llm::FauxStep::say("counted")], 0);
        let approvals = approvals();
        let mut events = agent.subscribe();
        let started = parse(
            &dispatch_line(
                r#"{"op":"prompt","task":"say counted"}"#,
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(started["data"]["started"], true, "{started}");
        let _ = wait_for_end(&mut events).await;

        // Core's own accumulator, read directly. The reply below must equal THIS, and the
        // assertion is written against it rather than against a hard-coded number, so a
        // change in what the faux provider bills does not silently unmake the test.
        let core = agent.session_usage();
        let reply =
            parse(&dispatch_line(r#"{"op":"get_session_stats"}"#, &agent, &approvals).await);
        assert_eq!(reply["success"], true, "{reply}");
        assert_eq!(reply["data"]["inputTokens"], core.input, "{reply}");
        assert_eq!(reply["data"]["outputTokens"], core.output, "{reply}");
        assert!(
            core.output > 0,
            "the precondition: the faux provider must actually bill, or this asserts nothing"
        );
        assert!(
            reply["data"]["contextTokens"].as_u64().unwrap_or(0) > 0,
            "a session with an assistant message has context in use: {reply}"
        );
    }

    /// The per-model map is what makes the cost a real number rather than a guess.
    ///
    /// Price is a property of the model that consumed the tokens, so a reply that summed
    /// everything and priced it at one row would pass every test above and be wrong for any
    /// session that switched models. This asserts the split exists and names the model.
    #[tokio::test]
    async fn session_stats_bill_each_response_to_its_own_model() {
        let agent = agent_with(vec![epsilon_llm::FauxStep::say("hi")], 0);
        let approvals = approvals();
        let mut events = agent.subscribe();
        let _ = dispatch_line(r#"{"op":"prompt","task":"hi"}"#, &agent, &approvals).await;
        let _ = wait_for_end(&mut events).await;

        let by_model = agent.session_usage_by_model();
        assert_eq!(by_model.len(), 1, "one model ran: {by_model:?}");
        let (model, usage) = by_model.iter().next().expect("checked above");
        assert_eq!(model, "faux-1", "{by_model:?}");
        assert!(usage.output > 0, "{usage:?}");

        let reply =
            parse(&dispatch_line(r#"{"op":"get_session_stats"}"#, &agent, &approvals).await);
        assert_eq!(
            reply["data"]["models"],
            serde_json::json!(["faux-1"]),
            "the reply must name the models it priced: {reply}"
        );
    }

    /// `get_messages` returns the conversation as role-tagged objects.
    ///
    /// Epsilon's `Message` is a serde `tag = "role"` enum, so this is an array of
    /// role-tagged objects — the shape a pi client expects, from a different Rust type.
    #[tokio::test]
    async fn get_messages_returns_the_conversation() {
        let agent = agent_with(vec![epsilon_llm::FauxStep::say("pong")], 0);
        let approvals = approvals();
        let mut events = agent.subscribe();
        let _ = dispatch_line(r#"{"op":"prompt","task":"ping"}"#, &agent, &approvals).await;
        let _ = wait_for_end(&mut events).await;

        let reply = parse(&dispatch_line(r#"{"op":"get_messages"}"#, &agent, &approvals).await);
        assert_eq!(reply["success"], true, "{reply}");
        let messages = reply["data"]["messages"]
            .as_array()
            .unwrap_or_else(|| panic!("messages must be an array: {reply}"));
        assert!(messages.len() >= 2, "user and assistant at least: {reply}");
        assert_eq!(messages[0]["role"], "user", "{reply}");
        assert!(
            messages.iter().any(|m| m["role"] == "assistant"),
            "the agent spoke, so an assistant message must be present: {reply}"
        );
    }

    /// `get_messages` on a fresh agent is an empty array, not an error.
    #[tokio::test]
    async fn get_messages_is_empty_before_anything_is_said() {
        let agent = agent_with(vec![], 0);
        let approvals = approvals();
        let reply = parse(&dispatch_line(r#"{"op":"get_messages"}"#, &agent, &approvals).await);
        assert_eq!(reply["success"], true, "{reply}");
        assert_eq!(reply["data"]["messages"], serde_json::json!([]), "{reply}");
    }

    /// `get_commands` reports the TUI's own registry, not a second hand-kept list.
    ///
    /// A duplicate table would drift, and a client building a command palette would then
    /// offer commands the harness does not accept. The assertion reads the same constant
    /// the handler reads, so a drift between them is a compile error rather than a silent
    /// divergence.
    #[tokio::test]
    async fn get_commands_reads_the_tuis_own_registry() {
        let agent = agent_with(vec![], 0);
        let approvals = approvals();
        let reply = parse(&dispatch_line(r#"{"op":"get_commands"}"#, &agent, &approvals).await);
        assert_eq!(reply["success"], true, "{reply}");
        let commands = reply["data"]["commands"]
            .as_array()
            .unwrap_or_else(|| panic!("commands must be an array: {reply}"));

        let expected: Vec<String> = epsilon_tui::picker::SLASH_ITEMS
            .iter()
            .map(|c| c.value.trim_start_matches('/').to_string())
            .collect();
        for name in &expected {
            assert!(
                commands.iter().any(|c| c["name"] == name.as_str()),
                "every slash command the TUI offers must be listed here: missing {name:?} in {reply}"
            );
        }
        assert!(
            commands.iter().any(|c| c["name"] == "!define"),
            "pi appends !define; reported as not-defined rather than omitted, so a palette \
             does not look broken: {reply}"
        );
        let define = commands
            .iter()
            .find(|c| c["name"] == "!define")
            .expect("checked above");
        assert_eq!(
            define["defined"], false,
            "epsilon has no !define and must not claim it does: {reply}"
        );
    }

    /// `get_commands` lists a discovered skill with the path a client would open.
    ///
    /// pi's `RpcSlashCommand` carries `sourceInfo` so an editor can jump to the file that
    /// defines the command; a command with no path is only half of what the shape is for.
    #[tokio::test]
    async fn get_commands_carries_a_source_path_for_each_command() {
        let agent = agent_with(vec![], 0);
        let approvals = approvals();
        let reply = parse(&dispatch_line(r#"{"op":"get_commands"}"#, &agent, &approvals).await);
        let commands = reply["data"]["commands"]
            .as_array()
            .unwrap_or_else(|| panic!("commands must be an array: {reply}"));
        for c in commands {
            if c["name"] == "!define" {
                continue; // explicitly unsupported, so it carries no path
            }
            assert!(
                c["sourceInfo"]["path"].is_string(),
                "{} carries no source path: {reply}",
                c["name"]
            );
        }
    }

    /// `clear_queue` empties BOTH queues and reports both counts.
    ///
    /// The control here is draining only steering. Follow-ups are the messages that run
    /// *after* the agent would otherwise stop, so leaving them queued while reporting a
    /// clear means the user watches the run continue with no way to see why — the exact
    /// shape of the TUI defect where steering was restored to the editor and follow-ups
    /// had nowhere to go.
    #[tokio::test]
    async fn clear_queue_empties_both_queues_and_reports_both_counts() {
        let approvals = approvals();
        // An IDLE agent, deliberately. A run in flight drains both queues at its own
        // drain points, and a measured probe showed a follow-up queued mid-run is consumed
        // before the next statement -- so a test against a running agent is a race that
        // passes when the scheduler is kind. `clear_queue` is most useful exactly when a
        // run is NOT yet going to consume the work, which is also the only state in which
        // the assertion below is deterministic.
        let agent = agent_with(vec![], 0);
        let _ = dispatch_line(
            r#"{"op":"steer","text":"actually stop"}"#,
            &agent,
            &approvals,
        )
        .await;
        let _ = dispatch_line(
            r#"{"op":"follow_up","text":"and this too"}"#,
            &agent,
            &approvals,
        )
        .await;

        // The precondition, as an assertion rather than an assumption. A queue that was
        // already empty makes every count below pass for the wrong reason -- and the
        // control for this test (drain steering only) still empties a one-item queue, so
        // "0" alone proves nothing.
        assert_eq!(agent.steer_queue_len(), 1, "the steer must be queued");
        assert_eq!(
            agent.followup_queue_len(),
            1,
            "the follow-up must be queued, or this asserts nothing"
        );

        let reply = parse(&dispatch_line(r#"{"op":"clear_queue"}"#, &agent, &approvals).await);
        assert_eq!(reply["success"], true, "{reply}");
        // pi's shape is `{ steering, followUp }` (rpc-types.ts:59) -- both counts.
        assert_eq!(
            reply["data"]["steering"].as_u64(),
            Some(1),
            "one steering message was queued and must be reported as cleared: {reply}"
        );
        assert_eq!(
            reply["data"]["followUp"].as_u64(),
            Some(1),
            "one follow-up was queued and must be reported as cleared: {reply}"
        );
        assert_eq!(
            agent.steer_queue_len(),
            0,
            "steering must be empty: {reply}"
        );
        assert_eq!(
            agent.followup_queue_len(),
            0,
            "the followup queue must be empty too -- this is what the control breaks: {reply}"
        );
    }

    /// Both tags reach the four new ops, and unknown names still fail.
    #[tokio::test]
    async fn the_four_new_ops_accept_pis_type_tag() {
        let agent = agent_with(vec![], 0);
        let approvals = approvals();
        for op in [
            "clear_queue",
            "get_messages",
            "get_commands",
            "get_session_stats",
        ] {
            let reply =
                parse(&dispatch_line(&format!(r#"{{"type":"{op}"}}"#), &agent, &approvals).await);
            assert_eq!(reply["success"], true, "{op} via pi's type tag: {reply}");
        }
    }

    /// pi's documented op name reaches the handler — the test whose absence let a
    /// mis-spelled wire name ship.
    ///
    /// The pre-existing `ops_deserialize_with_snake_tags` test could not catch this: it
    /// deserialized the enum and read the *variant* name, so it agreed with whatever the
    /// code did. Only dispatching a frame through `dispatch_line` and reading the reply
    /// exercises the wire. That is the whole difference between this test and the one it
    /// replaces, and it is why the assertion is on the reply's `success`, not on a
    /// deserialized value.
    #[tokio::test]
    async fn pi_spelled_op_names_reach_their_handlers() {
        let approvals = approvals();
        let agent = agent_with(vec![], 0);
        // The exact strings from `rpc-types.ts:27-31`, dispatched as sent.
        for frame in [
            r#"{"op":"steer","text":"x"}"#,
            // pi: `type: "follow_up"`, NOT "followup" (rpc-types.ts:30)
            r#"{"op":"follow_up","text":"x"}"#,
        ] {
            let reply = parse(&dispatch_line(frame, &agent, &approvals).await);
            assert_eq!(
                reply["success"], true,
                "{frame} must reach its handler, not the unknown-op arm: {reply}"
            );
        }
    }

    /// The legacy `followup` spelling still works, so nothing already in flight broke.
    #[tokio::test]
    async fn the_legacy_followup_spelling_still_reaches_its_handler() {
        let approvals = approvals();
        let agent = agent_with(vec![], 0);
        let reply =
            parse(&dispatch_line(r#"{"op":"followup","text":"x"}"#, &agent, &approvals).await);
        assert_eq!(reply["success"], true, "{reply}");
    }
}
