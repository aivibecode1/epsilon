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
        assert!(matches!(
            serde_json::from_str::<Op>(r#"{"op":"set_model"}"#).unwrap(),
            Op::Unknown
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

        let v =
            parse(&dispatch_line(r#"{"id":"req_9","op":"set_model"}"#, &agent, &approvals).await);
        assert_eq!(v["success"], false);
        assert_eq!(v["id"], "req_9");
        assert_eq!(v["command"], "set_model");
        assert_eq!(v["error"], "unknown op: set_model");

        let v = parse(
            &dispatch_line(
                r#"{"id":"req_10","type":"cycle_model"}"#,
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(v["success"], false);
        assert_eq!(v["id"], "req_10");
        assert_eq!(v["error"], "unknown op: cycle_model");

        // A pi-shaped unknown op carrying a realistic payload: unknown fields are
        // tolerated (serde's internally tagged unit variants ignore the rest of the
        // map), so the reply is still the named `unknown op` with the echoed id —
        // never a serde parse error.
        let v = parse(
            &dispatch_line(
                r#"{"id":"req_11","type":"set_model","provider":"anthropic","modelId":"x"}"#,
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(v["success"], false);
        assert_eq!(v["id"], "req_11");
        assert_eq!(v["command"], "set_model");
        assert_eq!(v["error"], "unknown op: set_model");
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
}
