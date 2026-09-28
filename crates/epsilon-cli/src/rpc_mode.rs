//! rpc mode (ADR-011): line-delimited JSON over stdio for editor embedding.
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
    /// The session's raw entries, optionally only those after `since`.
    GetEntries {
        /// The last entry id the caller already has.
        #[serde(default)]
        since: Option<String>,
    },
    /// The current branch as nodes, plus the leaf's id.
    GetTree,
    /// Name the session (a `Custom { "session_name" }` entry, as `/name` writes).
    SetSessionName { name: String },
    /// Fork the session at `entry_id` into a NEW file, and rebind to it.
    ///
    /// pi's field is `entryId` (`rpc-types.ts:70`), and the reply is
    /// `{ text, cancelled }` -- the selected entry's text, so a client can show what it
    /// forked from without a second round trip.
    #[serde(rename = "fork")]
    Fork {
        #[serde(rename = "entryId", alias = "entry_id")]
        entry_id: String,
    },
    /// Fork at the current leaf: pi's "duplicate this session here".
    Clone,
    /// Start an empty session, optionally recording a parent session path.
    NewSession {
        #[serde(rename = "parentSession", alias = "parent_session", default)]
        parent_session: Option<String>,
    },
    /// Reopen a different session file, and rebind to it.
    SwitchSession {
        #[serde(rename = "sessionPath", alias = "session_path")]
        session_path: String,
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
/// pi's `SessionEntry` as RPC sends it: `{ type, id, parentId, timestamp }`.
///
/// Three conversions, each load-bearing. pi uses camelCase (`parentId`); the store's Rust
/// struct is snake_case. pi's `timestamp` is an **ISO-8601 string**; ours is epoch millis,
/// and a client doing `new Date(entry.timestamp)` on a number gets 1970. The `type` tag
/// comes from the entry's own codec, so an entry this build cannot decode still reports the
/// tag the file gave it rather than vanishing from the tree.
fn entry_as_pi(e: &epsilon_contracts::session_entry::SessionEntry) -> serde_json::Value {
    serde_json::json!({
        "type": e.kind.wire_tag(),
        "id": e.id,
        "parentId": e.parent_id,
        "timestamp": epoch_millis_to_iso8601(e.timestamp_ms),
    })
}

/// Epoch millis to an ISO-8601 string, the way JavaScript's `new Date(n).toISOString()`
/// renders it.
///
/// A civil-date conversion by hand, on purpose: a date library is a new dependency for one
/// formatting call, and this is the standard days-from-civil algorithm (Hinnant's, inverted
/// so the era shift keeps every term positive) with that attribution in the name. UTC only,
/// with a `Z` suffix, because a timestamp with no offset is a bug waiting for a client in
/// another timezone -- and pi's `toISOString()` is always UTC with a `Z`.
fn epoch_millis_to_iso8601(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        y,
        m,
        d,
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60,
        ms % 1000
    )
}

/// True when following `parent_id` from `start` upward reaches `goal`.
///
/// Linking `start` under `goal` is safe only if `goal` is not one of `start`'s own
/// ancestors. A file edited by hand can contain `a → b → a`; without this check those two
/// entries are unreachable from any root, the emitter would build them as nodes that are
/// never attached, and the session would silently lose them.
///
/// The walk is bounded by the entry count. A longer walk means a cycle, which by definition
/// means `goal` IS reachable, so the bound cannot produce a wrong answer -- it only stops a
/// walk that would otherwise not terminate.
fn reaches(
    entries: &[epsilon_contracts::session_entry::SessionEntry],
    index: &std::collections::HashMap<&str, usize>,
    start: usize,
    goal: usize,
) -> bool {
    let mut cursor = start;
    for _ in 0..entries.len() {
        if cursor == goal {
            return true;
        }
        let next = match entries[cursor].parent_id.as_deref() {
            None => return false,
            Some(p) => match index.get(p) {
                Some(&n) => n,
                None => return false, // orphan: the chain ends here
            },
        };
        cursor = next;
    }
    true // the bounded walk hit a cycle, so goal is reachable
}

/// Point the agent at a different session file, and re-derive its conversation from it.
///
/// This is the RPC equivalent of pi's `rebindSession()`, which every one of `fork`,
/// `clone`, `new_session` and `switch_session` calls before replying. Reading pi's
/// `rpc-mode.ts` rather than its type declarations is what showed the four ops share one
/// step — a type would have said only `success(id, cmd, result)`.
///
/// Two halves, in this order, and the order is the whole point:
///
/// 1. **`attach_session_store`** swaps the mirror. Every subsequent append lands in the new
///    file, so this must happen before anything can write to the old one.
/// 2. **`reproject_current_branch`** re-derives the model-facing conversation from the new
///    store's shadowed view. Without it the agent would keep sending the *old* session's
///    messages to the provider while persisting to the *new* file — a session whose
///    transcript and whose model context silently disagree, which is worse than either
///    being wrong alone.
///
/// `store` is passed in by the caller when it has already been opened (`new_session` and
/// `switch_session` need the opened value to read its header); `fork` and `clone` name a
/// file that exists but has not been opened, so it is opened here.
fn rebind_to(
    agent: &Agent,
    path: &std::path::Path,
    store: Option<epsilon_core::SessionStore>,
) -> Result<(), String> {
    let store = match store {
        Some(s) => s,
        None => epsilon_core::SessionStore::open(path).map_err(|e| e.to_string())?,
    };
    agent.attach_session_store(Arc::new(std::sync::Mutex::new(store)));
    // The count is the conversation the new file implies. It is not an error for it to be
    // 0 -- an empty session is a real state -- but the re-projection failing IS, because it
    // means the agent is now writing to a file it cannot read back.
    agent.reproject_current_branch()?;
    Ok(())
}

/// Build pi's nested `SessionTreeNode[]` from a flat entry list.
///
/// pi's `getTree()` (`core/session-manager.ts`), reproduced rather than approximated:
///
/// - every entry becomes a node `{ entry, children, label, labelTimestamp }`;
/// - a node is a **root** when its `parentId` is `None`, equals its own id (pi explicitly
///   tolerates a self-parent), or names a parent that is **not in this file** -- orphans
///   become roots, so a truncated or hand-edited JSONL renders everything rather than
///   dropping the tail silently;
/// - children are sorted by timestamp ascending **at every level**.
///
/// The emitter recurses, which is safe because the link pass above has already broken every
/// cycle: no node is its own ancestor, so depth is bounded by the entry count. That buys
/// the same guarantee as pi's iterative stack, by a cheaper route -- pi cannot mutate its
/// input, so it has nowhere to record the decision.
fn build_tree(
    entries: &[epsilon_contracts::session_entry::SessionEntry],
) -> Vec<serde_json::Value> {
    let index: std::collections::HashMap<&str, usize> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.id.as_str(), i))
        .collect();

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); entries.len()];
    let mut is_root = vec![false; entries.len()];
    for (i, e) in entries.iter().enumerate() {
        // pi's three root cases, in one place: no parent, a self-parent, or a parent that
        // is not in the file. Anything else is linked, unless the link would close a loop.
        let parent = e.parent_id.as_deref().filter(|p| *p != e.id.as_str());
        match parent {
            None => is_root[i] = true,
            Some(p) => match index.get(p) {
                // A parent that is an ancestor of this entry would close a loop; make the
                // entry a root so it is still rendered, exactly once.
                Some(&pi) if reaches(entries, &index, pi, i) => is_root[i] = true,
                Some(&pi) => children[pi].push(i),
                None => is_root[i] = true, // orphan
            },
        }
    }

    fn emit(
        entries: &[epsilon_contracts::session_entry::SessionEntry],
        i: usize,
        children: &[Vec<usize>],
    ) -> serde_json::Value {
        let mut kids = children[i].clone();
        kids.sort_by_key(|&c| entries[c].timestamp_ms);
        serde_json::json!({
            "entry": entry_as_pi(&entries[i]),
            "children": kids
                .iter()
                .map(|&c| emit(entries, c, children))
                .collect::<Vec<_>>(),
            // pi fills these from its label maps; epsilon has no per-entry labels, so
            // they are present-and-null rather than absent, so a client reading
            // `node.label` sees null instead of undefined.
            "label": serde_json::Value::Null,
            "labelTimestamp": serde_json::Value::Null,
        })
    }

    let mut roots: Vec<usize> = (0..entries.len()).filter(|&i| is_root[i]).collect();
    roots.sort_by_key(|&i| entries[i].timestamp_ms);
    roots
        .into_iter()
        .map(|i| emit(entries, i, &children))
        .collect()
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
            // pi's payload is `{ steering: string[], followUp: string[] }`
            // (`rpc-types.ts`, RpcResponse) -- the cleared messages' TEXT, not a count.
            // I first wrote `len()` here, which satisfies "both keys are present" and is
            // what a reader would expect from a clear, but a client that wants to show
            // the user what it just dropped has nothing to show. Returning the text is
            // also what lets a client put it back, which is the entire point of a queue
            // clear the user might regret.
            //
            // Both queues are drained. Leaving follow-ups queued while reporting a clear
            // is the shape of the TUI defect where steering was restored to the editor
            // and follow-ups had nowhere to go -- and follow-ups are the messages that
            // run *after* the agent would otherwise stop, so the run would continue with
            // no visible reason.
            let text_of = |msgs: Vec<epsilon_contracts::Message>| -> Vec<String> {
                msgs.into_iter().map(|m| m.text()).collect()
            };
            let steering = text_of(agent.drain_steering_all());
            let follow_up = text_of(agent.drain_followups_all());
            ok_line(
                "clear_queue",
                id,
                serde_json::json!({ "steering": steering, "followUp": follow_up }),
            )
        }
        Op::GetEntries { since } => {
            // pi: `{ entries: SessionEntry[]; leafId: string | null }`, optionally
            // `since` to fetch only what is newer.
            //
            // The store is optional, and an agent with no session mirror is a real state
            // (RPC mode run without a session). An error naming the cause beats an empty
            // list, which reads like "this session has no history" and would send a client
            // hunting for a bug that is not there.
            let Some(store) = agent.session_store() else {
                return err_line(
                    Some("get_entries"),
                    id,
                    "no session store attached: this agent is not persisting a session",
                );
            };
            // One lock, one read. The first version of this re-locked the store inside an
            // `and_then` to fetch the leaf, which is both a clippy `and_then(|x| Ok(y))`
            // and a self-deadlock waiting for a debug build to notice -- `Mutex` is not
            // reentrant. Both reads come from the same guard.
            let read = store
                .lock()
                .map_err(|_| "session store poisoned".to_string())
                .map(|guard| (guard.read_all().map_err(|e| e.to_string()), guard.leaf_id()));
            let (all, leaf) = match read {
                Ok((Ok(all), leaf)) => (all, leaf),
                Ok((Err(why), _)) => return err_line(Some("get_entries"), id, why),
                Err(why) => return err_line(Some("get_entries"), id, why),
            };
            // `since` names the last entry the caller saw; the reply is everything after
            // it. An id matching nothing returns the WHOLE file rather than nothing: a
            // `since` the client cannot resolve means it has fallen behind, and an empty
            // reply would have it conclude the session is empty. The asymmetry is
            // deliberate -- over-sending is recoverable, under-sending looks like loss.
            let filtered = match since.as_deref() {
                None => all,
                Some(from) => match all.iter().position(|e| e.id == from) {
                    Some(pos) => all[pos + 1..].to_vec(),
                    None => all,
                },
            };
            // **D29, and it is D27's twin.** `get_entries` returned the raw
            // `SessionEntry` structs, so a client saw epsilon's internal shape --
            // `parent_id`, `seq`, `timestamp_ms`, and a `kind` OBJECT -- while
            // `get_tree`, four lines below, returned pi's. Two ops, one reference type,
            // two shapes.
            //
            //   raw:      { id, kind: { type, ... }, parent_id, seq, timestamp_ms }
            //   pi:       { type, id, parentId, timestamp: "2026-09-28T10:18:40.545Z" }
            //
            // Found by driving the INSTALLED binary over real stdio, not by a test: the
            // in-process M190 tests asserted `get_entries` against itself, so raw and
            // converted agreed by construction -- the same defect as D27, one function
            // over. `entry_as_pi` is the single conversion both ops now share, which is
            // what makes a second shape here impossible rather than merely absent.
            //
            // pi's `SessionEntryBase` is one type behind both ops
            // (`session-manager.ts`), and its subclasses add per-type payloads
            // (`message`, `thinkingLevel`, `modelId`, `summary`, ...). Those are NOT
            // ported here -- see the milestone's Limits. The base four are what a client
            // needs to rebuild the tree, and inventing the rest from epsilon's `kind`
            // enum would be another shape guessed rather than read.
            let entries: Vec<serde_json::Value> = filtered.iter().map(entry_as_pi).collect();
            ok_line(
                "get_entries",
                id,
                serde_json::json!({ "entries": entries, "leafId": leaf }),
            )
        }
        Op::GetTree => {
            // pi: `{ tree: SessionTreeNode[]; leafId: string | null }`, where
            // `SessionTreeNode` is `{ entry, children, label?, labelTimestamp? }` -- a
            // RECURSIVE forest over EVERY entry, not a flat list of the current branch.
            //
            // **This is D27, and it was my own M189 work.** The first version returned
            // `tree` as a flat array of the branch with `{id, parentId, seq, timestampMs,
            // kind}`. Three separate mismatches: flat vs nested, branch vs whole file, and
            // a number where pi sends an ISO-8601 string. It passed 1,446 tests because
            // the tests asserted my shape. `rpc-types.ts` says `SessionTreeNode[]`, and
            // that type gives the field NAMES; only `session-manager.ts`'s `getTree()`
            // gives the structure. A client written against pi would find
            // `node.children === undefined` and draw nothing.
            let Some(store) = agent.session_store() else {
                return err_line(
                    Some("get_tree"),
                    id,
                    "no session store attached: this agent is not persisting a session",
                );
            };
            let read = store
                .lock()
                .map_err(|_| "session store poisoned".to_string())
                .map(|guard| (guard.read_all().map_err(|e| e.to_string()), guard.leaf_id()));
            let (all, leaf) = match read {
                Ok((Ok(all), leaf)) => (all, leaf),
                Ok((Err(why), _)) => return err_line(Some("get_tree"), id, why),
                Err(why) => return err_line(Some("get_tree"), id, why),
            };
            let tree = build_tree(&all);
            ok_line(
                "get_tree",
                id,
                serde_json::json!({ "tree": tree, "leafId": leaf }),
            )
        }
        Op::SetSessionName { name } => {
            // pi: `{ type: "set_session_name", name }`, success with no payload.
            //
            // A session's name is a `Custom { "session_name" }` entry, not a header
            // field -- that is how `/name` already stores it and how `discover.rs` reads
            // it back off the tail. Writing it the same way is what makes the RPC op and
            // the slash command agree; a parallel header field would be invisible to both.
            let Some(store) = agent.session_store() else {
                return err_line(
                    Some("set_session_name"),
                    id,
                    "no session store attached: this agent is not persisting a session",
                );
            };
            let written = store
                .lock()
                .map_err(|_| "session store poisoned".to_string())
                .and_then(|mut guard| {
                    guard
                        .append(epsilon_contracts::session_entry::EntryKind::Custom {
                            custom_type: "session_name".to_string(),
                            data: Some(serde_json::json!({ "name": name })),
                        })
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                });
            if let Err(why) = written {
                return err_line(Some("set_session_name"), id, why);
            }
            ok_line("set_session_name", id, serde_json::json!({ "name": name }))
        }
        Op::Fork { entry_id } => {
            // pi: `success(id, "fork", { text, cancelled })` -- the fork happens and the
            // client is told what it forked from.
            //
            // **This is what pi's `rebindSession()` is for**, and the RPC agent can do the
            // same thing in two steps: a new store for the forked file, then re-derive the
            // conversation from it. `attach_session_store` exists precisely so a caller can
            // hand the agent a different file without rebuilding it -- the same reason pi
            // re-subscribes instead of re-constructing its host.
            let Some(handle) = agent.session_store() else {
                return err_line(
                    Some("fork"),
                    id,
                    "no session store attached: this agent is not persisting a session",
                );
            };
            let (dest, text) = {
                let st = handle
                    .lock()
                    .map_err(|_| "session store poisoned".to_string())
                    .and_then(
                        |guard| -> Result<(std::path::PathBuf, serde_json::Value), String> {
                            // `branch_to` is private; the public read is `branch_entries`,
                            // which is the same ancestry walk the store itself uses. `fork_branch_at`
                            // does its own walk, so this lookup is only for the reply's `text`.
                            let text = guard
                                .branch_entries()
                                .map_err(|e| e.to_string())?
                                .into_iter()
                                .find(|e| e.id == entry_id)
                                .map(|e| entry_as_pi(&e))
                                .unwrap_or(serde_json::Value::Null);
                            // `fork_branch_at` was written and tested (8 references, every one
                            // a test) with no production caller until now. It is pi's fork
                            // primitive: write the selected ancestry into a NEW file.
                            let dir = guard
                                .path()
                                .parent()
                                .map(std::path::Path::to_path_buf)
                                .ok_or_else(|| {
                                    "the session file has no parent directory".to_string()
                                })?;
                            let dest = guard
                                .fork_branch_at(&entry_id, &dir)
                                .map_err(|e| e.to_string())?;
                            Ok((dest, text))
                        },
                    );
                match st {
                    Ok(v) => v,
                    Err(why) => return err_line(Some("fork"), id, why),
                }
            };
            if let Err(why) = rebind_to(agent.as_ref(), &dest, None) {
                return err_line(Some("fork"), id, why);
            }
            ok_line(
                "fork",
                id,
                serde_json::json!({ "text": text, "cancelled": false }),
            )
        }
        Op::Clone => {
            // pi refuses when there is no leaf, rather than forking an empty ancestry and
            // reporting success. Same rule, same reason: a client that asked to duplicate
            // "here" and got a session with no history has been told a falsehood.
            let Some(handle) = agent.session_store() else {
                return err_line(
                    Some("clone"),
                    id,
                    "no session store attached: this agent is not persisting a session",
                );
            };
            let dest = {
                let leaf = handle
                    .lock()
                    .map_err(|_| "session store poisoned".to_string())
                    .and_then(|guard| {
                        guard
                            .leaf_id()
                            .ok_or_else(|| "no current entry selected".to_string())
                    });
                let leaf = match leaf {
                    Ok(l) => l,
                    Err(why) => return err_line(Some("clone"), id, why),
                };
                let forked = handle
                    .lock()
                    .map_err(|_| "session store poisoned".to_string())
                    .and_then(|guard| {
                        let dir = guard
                            .path()
                            .parent()
                            .map(std::path::Path::to_path_buf)
                            .ok_or("the session file has no parent directory")?;
                        guard.fork_branch_at(&leaf, &dir).map_err(|e| e.to_string())
                    });
                match forked {
                    Ok(p) => p,
                    Err(why) => return err_line(Some("clone"), id, why),
                }
            };
            if let Err(why) = rebind_to(agent.as_ref(), &dest, None) {
                return err_line(Some("clone"), id, why);
            }
            ok_line("clone", id, serde_json::json!({ "cancelled": false }))
        }
        Op::NewSession { parent_session } => {
            // pi: `success(id, "new_session", result)` where result is the runtime host's
            // own reply -- it names the new file so a client can show it.
            //
            // A new file in the same directory, so `list_sessions` finds it and pi's
            // `switch_session` can come back to it. `open_child_in` when a parent is named
            // (that is what records the lineage), plain `open_in` otherwise.
            let Some(handle) = agent.session_store() else {
                return err_line(
                    Some("new_session"),
                    id,
                    "no session store attached: this agent is not persisting a session",
                );
            };
            let dir = match handle
                .lock()
                .map_err(|_| "session store poisoned".to_string())
                .and_then(|g| {
                    g.path()
                        .parent()
                        .map(std::path::Path::to_path_buf)
                        .ok_or_else(|| "the session file has no parent directory".to_string())
                }) {
                Ok(d) => d,
                Err(why) => return err_line(Some("new_session"), id, why),
            };
            let cwd = dir
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| dir.clone());
            let file = dir.join(format!("{}.jsonl", epsilon_contracts::new_id("ses")));
            let built = match parent_session {
                Some(parent) => epsilon_core::SessionStore::open_child_in(&file, &cwd, &parent),
                None => epsilon_core::SessionStore::open_in(&file, &cwd),
            };
            let store = match built {
                Ok(s) => s,
                Err(e) => return err_line(Some("new_session"), id, e.to_string()),
            };
            if let Err(why) = rebind_to(agent.as_ref(), &file, Some(store)) {
                return err_line(Some("new_session"), id, why);
            }
            ok_line(
                "new_session",
                id,
                serde_json::json!({
                    "cancelled": false,
                    "sessionFile": file.to_string_lossy(),
                }),
            )
        }
        Op::SwitchSession { session_path } => {
            // pi: `success(id, "switch_session", result)`.
            //
            // Opened with the *target's own recorded cwd* when it has one, because that is
            // the directory the session was created in and the one its relative paths were
            // written against. Reusing the current session's cwd would silently reinterpret
            // every relative path in the file we just opened.
            let path = std::path::PathBuf::from(&session_path);
            if !path.exists() {
                return err_line(
                    Some("switch_session"),
                    id,
                    format!("no session file at {session_path}"),
                );
            }
            let opened = epsilon_core::SessionStore::open(&path);
            let store = match opened {
                Ok(s) => s,
                Err(e) => return err_line(Some("switch_session"), id, e.to_string()),
            };
            let cwd = store
                .header()
                .map(|h| std::path::PathBuf::from(h.cwd.clone()))
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| {
                    path.parent()
                        .map(std::path::Path::to_path_buf)
                        .unwrap_or_else(|| std::path::PathBuf::from("."))
                });
            if let Err(why) = rebind_to(agent.as_ref(), &path, Some(store)) {
                return err_line(Some("switch_session"), id, why);
            }
            let _ = cwd;
            ok_line(
                "switch_session",
                id,
                serde_json::json!({
                    "cancelled": false,
                    "sessionFile": path.to_string_lossy(),
                }),
            )
        }
        Op::GetMessages => {
            // pi returns the conversation as `SessionMessage[]`. Epsilon's `Message` is a
            // serde `tag = "role"` enum, so it lands as an array of role-tagged objects --
            // the same JSON shape a pi client expects, from a different Rust type.
            // pi's client reads `getData<AgentMessage[]>(response).messages` -- a BARE
            // array, unwrapped. M188 sent `{messages: [...]}`, which was wrong: a client
            // written against pi iterates the object, gets key names instead of messages,
            // and renders nothing. Part of D27.
            let history = agent.history();
            ok_line(
                "get_messages",
                id,
                serde_json::Value::Array(
                    history
                        .into_iter()
                        .map(|m| {
                            serde_json::to_value(m)
                                .unwrap_or_else(|e| panic!("a message must serialize: {e}"))
                        })
                        .collect(),
                ),
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
            // A bare array, like `get_messages`: pi's client reads
            // `getData<RpcSlashCommand[]>(response).commands`. The `count` key is not in
            // pi's shape, and a client doing `reply.length` on the wrapper would get the
            // number of object keys (1) rather than the number of commands.
            ok_line("get_commands", id, serde_json::Value::Array(commands))
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

        // This set keeps shrinking as ops land, and that is the point of writing the
        // comment: `set_model` (M186), `get_entries`/`set_session_name` (M189),
        // `new_session` (M192) have each been the example and each stopped being true
        // when implemented. A test whose example silently stops being true keeps passing
        // for the wrong reason, so `compact` takes over until M19x lands it too.
        let v = parse(&dispatch_line(r#"{"id":"req_9","op":"compact"}"#, &agent, &approvals).await);
        assert_eq!(v["success"], false, "{v}");
        assert_eq!(v["id"], "req_9", "the correlation id still echoes: {v}");
        assert_eq!(v["command"], "compact");
        assert_eq!(v["error"], "unknown op: compact");

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
        // `fork` was the M186-M191 example here and is implemented from M192, so the
        // third example moved to `set_auto_retry` -- still genuinely absent, and with a
        // payload that shows unknown fields are tolerated rather than fatal.
        let v = parse(
            &dispatch_line(
                r#"{"id":"req_11","type":"set_auto_retry","enabled":true}"#,
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(v["success"], false, "{v}");
        assert_eq!(v["id"], "req_11", "{v}");
        assert_eq!(v["command"], "set_auto_retry");
        assert_eq!(v["error"], "unknown op: set_auto_retry");
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
        // D27: pi's `getMessages()` returns a BARE array. Reading `data.messages` here is
        // what M188's test did, and it agreed with the wrong shape for the same reason
        // the shape was wrong: the test and the code were written from the same guess.
        let messages = reply["data"]
            .as_array()
            .unwrap_or_else(|| panic!("data must BE the array, as pi sends it: {reply}"));
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
        assert_eq!(reply["data"], serde_json::json!([]), "{reply}");
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
        // D27: pi's `getCommands()` returns a bare `RpcSlashCommand[]`, so `data` IS the
        // array. The `{commands, count}` wrapper M188 sent is not in pi's shape at all.
        let commands = reply["data"]
            .as_array()
            .unwrap_or_else(|| panic!("data must BE the array, as pi sends it: {reply}"));

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
        // D27: pi's `getCommands()` returns a bare `RpcSlashCommand[]`, so `data` IS the
        // array. The `{commands, count}` wrapper M188 sent is not in pi's shape at all.
        let commands = reply["data"]
            .as_array()
            .unwrap_or_else(|| panic!("data must BE the array, as pi sends it: {reply}"));
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
        // pi's payload is `{ steering: string[], followUp: string[] }` -- the cleared
        // messages' TEXT, not a count, so a client can show the user what it dropped and
        // put it back. Asserting the text is also a stronger check than the count was:
        // a count of 1 is satisfied by the wrong message, the text is not.
        assert_eq!(
            reply["data"]["steering"],
            serde_json::json!(["actually stop"]),
            "the cleared steering text must come back: {reply}"
        );
        assert_eq!(
            reply["data"]["followUp"],
            serde_json::json!(["and this too"]),
            "the cleared follow-up text must come back -- this is what the control breaks: \
             {reply}"
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

    /// **The case `session_usage_by_model` exists for: a session that switched models.**
    ///
    /// M188 shipped the per-model split on the argument that price belongs to the model that
    /// *consumed* the tokens, and asserted it through one model's worth of rows. That is
    /// half the claim. This is the other half: two runs, two models, two rows, each priced
    /// with its own rate.
    ///
    /// The faux provider reports whatever `ModelRef` it is handed (`faux.rs`:
    /// `build_events(&model.id, step)`), so switching `Agent::set_model_id` between runs
    /// produces genuinely differently-billed messages without a credential or a network.
    /// `deepseek/deepseek-chat-v3.1` is a real priced catalog row; `faux-1` is priced at
    /// zero. The test therefore proves the *split* and the *per-row pricing* together —
    /// a single flat total could not produce these two different answers.
    #[tokio::test]
    async fn a_session_that_switches_models_is_billed_per_model() {
        let agent = agent_with(
            vec![
                epsilon_llm::FauxStep::say("first"),
                epsilon_llm::FauxStep::say("second"),
            ],
            0,
        );
        let approvals = approvals();

        // The precondition, stated explicitly. `Agent::new` defaults to `faux-1`
        // (epsilon-core/src/lib.rs), which is priced at ZERO -- so without this line
        // "run 1 on the priced model" is a comment rather than a fact, and both runs
        // are billed to the same free model. That is not a hypothetical: the first
        // version of this test failed with one row instead of two, and the cause was
        // here and not in the split.
        agent.set_model_id("deepseek/deepseek-chat-v3.1");
        assert_eq!(agent.model_id(), "deepseek/deepseek-chat-v3.1");

        // Run 1 on the real priced model.
        let mut events = agent.subscribe();
        let _ = dispatch_line(r#"{"op":"prompt","task":"one"}"#, &agent, &approvals).await;
        wait_for_end(&mut events).await;

        // Run 2 after switching to the free one.
        agent.set_model_id("faux-1");
        let mut events2 = agent.subscribe();
        let _ = dispatch_line(r#"{"op":"prompt","task":"two"}"#, &agent, &approvals).await;
        wait_for_end(&mut events2).await;

        let by_model = agent.session_usage_by_model();
        assert_eq!(
            by_model.len(),
            2,
            "two models ran, so there must be two rows: {by_model:?}"
        );
        let priced = by_model
            .get("deepseek/deepseek-chat-v3.1")
            .unwrap_or_else(|| panic!("the first run's model must have its own row: {by_model:?}"));
        let free = by_model.get("faux-1").expect("the second run's model too");
        assert!(priced.output > 0, "{priced:?}");
        assert!(free.output > 0, "{free:?}");

        // The flat total is the sum of both, which is what makes the split a split rather
        // than a second copy.
        let flat = agent.session_usage();
        assert_eq!(
            flat.output,
            priced.output + free.output,
            "the flat total must be the sum of the per-model rows: {flat:?} vs {by_model:?}"
        );

        // The reply names BOTH models -- the thing no flat total could report.
        let reply =
            parse(&dispatch_line(r#"{"op":"get_session_stats"}"#, &agent, &approvals).await);
        let mut named: Vec<String> = reply["data"]["models"]
            .as_array()
            .unwrap_or_else(|| panic!("models must be an array: {reply}"))
            .iter()
            .map(|v| v.as_str().unwrap_or_default().to_string())
            .collect();
        named.sort();
        assert_eq!(
            named,
            vec![
                "deepseek/deepseek-chat-v3.1".to_string(),
                "faux-1".to_string()
            ],
            "the reply must name every model it priced: {reply}"
        );
    }

    /// The cost of a switched session is the sum of each model's own rate, not one rate
    /// applied twice — the concrete arithmetic the split exists to get right.
    ///
    /// `faux-1` is priced at 0/Mtok, so it contributes nothing. If the implementation
    /// priced the flat total at the *current* (free) model, the whole session would read
    /// as free; if it priced it at the first, the second would be over-billed. Neither
    /// equals the sum, and the test says which figure it expects.
    #[tokio::test]
    async fn a_switched_sessions_cost_is_not_its_cheapest_models_price() {
        let agent = agent_with(
            vec![
                epsilon_llm::FauxStep::say("first"),
                epsilon_llm::FauxStep::say("second"),
            ],
            0,
        );
        let approvals = approvals();
        // Priced first, then free -- see the sibling test for why this line is the
        // precondition and not a detail.
        agent.set_model_id("deepseek/deepseek-chat-v3.1");

        let mut events = agent.subscribe();
        let _ = dispatch_line(r#"{"op":"prompt","task":"one"}"#, &agent, &approvals).await;
        wait_for_end(&mut events).await;
        // Switch to the zero-priced model for the second run.
        agent.set_model_id("faux-1");
        let mut events2 = agent.subscribe();
        let _ = dispatch_line(r#"{"op":"prompt","task":"two"}"#, &agent, &approvals).await;
        wait_for_end(&mut events2).await;

        let reply =
            parse(&dispatch_line(r#"{"op":"get_session_stats"}"#, &agent, &approvals).await);
        let cost = reply["data"]["totalCostUsd"]
            .as_f64()
            .expect("a cost must be reported");

        // The faux run is genuinely free, so the session's cost is exactly the priced run's
        // -- and strictly greater than zero, which is the assertion that would fail if the
        // whole session were priced at the free model.
        assert!(
            cost > 0.0,
            "one run was on a priced model, so the session cannot be free: {reply}"
        );
        // And the free run must not have *added* to it: cost is the priced row alone.
        let priced_only = *agent
            .session_usage_by_model()
            .get("deepseek/deepseek-chat-v3.1")
            .expect("the priced run's row");
        let priced_cost = epsilon_tui::ui::cost::breakdown_per_model(
            &std::collections::BTreeMap::from([(
                "deepseek/deepseek-chat-v3.1".to_string(),
                epsilon_tui::ui::cost::TokenCounts {
                    input: priced_only.input,
                    output: priced_only.output,
                    cache_read: priced_only.cache_read,
                    cache_write: priced_only.cache_write,
                },
            )]),
            |id| {
                epsilon_llm::catalog::lookup_model(id).map(|info| {
                    epsilon_tui::ui::cost::TokenPrices {
                        input: info.input_cost_micro_per_mtok,
                        output: info.output_cost_micro_per_mtok,
                        cache_read: info.cache_read_cost_micro_per_mtok,
                        cache_write: info.cache_write_cost_micro_per_mtok,
                    }
                })
            },
        )
        .total() as f64
            / 1_000_000.0;
        assert!(
            (cost - priced_cost).abs() < 1e-12,
            "the session's cost must be the priced row alone ({priced_cost}), not a rate \
             applied to the flat total ({cost}): {reply}"
        );
    }

    // ---- M189: get_entries, get_tree, set_session_name ----

    /// An agent with a real session store, so the store-backed ops can be exercised.
    fn agent_with_store(steps: Vec<epsilon_llm::FauxStep>) -> (Arc<Agent>, std::path::PathBuf) {
        // A per-TEST unique suffix, not a per-thread one. The fork tests below each create
        // their own session, and `std::thread::current().id()` is the same for every
        // `#[tokio::test]` on the same worker -- so with a dozen tests in one binary the
        // names collided and one test's `rm_dir_all` deleted another's session mid-run.
        // `new_id` is already a dependency and is unique for exactly this purpose.
        let dir = std::env::temp_dir().join(format!(
            "epsilon-rpc-store-{}-{}",
            std::process::id(),
            epsilon_contracts::new_id("t")
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("session.jsonl");
        let store = epsilon_core::SessionStore::open_in(&file, &dir).expect("session store");
        let agent = agent_with(steps, 0);
        agent.attach_session_store(Arc::new(std::sync::Mutex::new(store)));
        (agent, dir)
    }

    /// `set_session_name` writes what `discover.rs` reads, so the RPC op and `/name` agree.
    ///
    /// The assertion is on the ENTRY the store now holds, not on the reply. A handler that
    /// echoed the name back without writing anything would pass a reply-only test — and the
    /// whole point is that the name survives into the session file.
    #[tokio::test]
    async fn set_session_name_appends_a_name_entry_the_discoverer_can_read() {
        let (agent, dir) = agent_with_store(vec![]);
        let approvals = approvals();

        let reply = parse(
            &dispatch_line(
                r#"{"op":"set_session_name","name":"the audit run"}"#,
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(reply["success"], true, "{reply}");

        // Read it back through the session crate's OWN reader (`list_sessions`, the same
        // call the resume picker makes), not by peeking at the file. That is the point:
        // if the RPC op wrote the entry in a shape the discoverer cannot read, this fails
        // where a hand-rolled parse of the same file would not.
        let found = epsilon_session::list_sessions(&dir)
            .into_iter()
            .find(|s| s.path.ends_with("session.jsonl"))
            .and_then(|s| s.name);
        assert_eq!(
            found.as_deref(),
            Some("the audit run"),
            "the name must be readable by the discoverer, which is what /name writes: {found:?}"
        );
    }

    /// **`get_tree` returns pi's nested forest, and every entry appears exactly once.**
    ///
    /// **This test is the second half of fixing D27.** The first version asserted a flat
    /// array of the current branch, which is why the wrong shape passed 1,446 tests: the
    /// test and the handler were written from the same guess. The assertions below read
    /// the shape off pi's `SessionTreeNode` and its `getTree()` implementation, so they
    /// would now fail against the old handler.
    #[tokio::test]
    async fn get_tree_is_a_nested_forest_of_every_entry() {
        let (agent, _dir) = agent_with_store(vec![
            epsilon_llm::FauxStep::say("one"),
            epsilon_llm::FauxStep::say("two"),
        ]);
        let approvals = approvals();
        let mut events = agent.subscribe();
        let _ = dispatch_line(r#"{"op":"prompt","task":"first"}"#, &agent, &approvals).await;
        wait_for_end(&mut events).await;
        let mut events2 = agent.subscribe();
        let _ = dispatch_line(r#"{"op":"prompt","task":"second"}"#, &agent, &approvals).await;
        wait_for_end(&mut events2).await;

        let tree = parse(&dispatch_line(r#"{"op":"get_tree"}"#, &agent, &approvals).await);
        let all = parse(&dispatch_line(r#"{"op":"get_entries"}"#, &agent, &approvals).await);
        assert_eq!(tree["success"], true, "{tree}");
        let roots = tree["data"]["tree"]
            .as_array()
            .unwrap_or_else(|| panic!("tree must be an array: {tree}"));
        let file = all["data"]["entries"]
            .as_array()
            .unwrap_or_else(|| panic!("entries must be an array: {all}"));

        // **Nesting is the property the old handler lacked.** Every node has a `children`
        // ARRAY, and a node below the root level proves the tree actually nests. A flat
        // array of entries has no `children` key at all, so this is the assertion that
        // would have caught D27.
        // No `depth` parameter: the depth is measured once, afterwards, by `deep()`. A
        // parameter threaded through the recursion purely to be ignored is one clippy
        // rightly refuses, and the m190 gate caught exactly that.
        fn walk(nodes: &[serde_json::Value], seen: &mut Vec<String>, file: &[serde_json::Value]) {
            for n in nodes {
                assert!(
                    n["children"].is_array(),
                    "every node needs a children ARRAY (pi's SessionTreeNode): {n}"
                );
                assert_eq!(
                    n["label"],
                    serde_json::Value::Null,
                    "labels are null, not absent: {n}"
                );
                assert!(
                    n["labelTimestamp"].is_null(),
                    "labelTimestamp is null too: {n}"
                );
                // pi's SessionEntry as RPC sends it: camelCase, and an ISO STRING.
                assert!(n["entry"]["id"].is_string(), "{n}");
                assert!(n["entry"].get("parentId").is_some(), "{n} needs parentId");
                assert!(n["entry"]["type"].is_string(), "{n} needs a type tag");
                let ts = n["entry"]["timestamp"].as_str().unwrap_or_else(|| {
                    panic!("timestamp must be an ISO STRING, not a number: {n}")
                });
                assert!(
                    ts.ends_with('Z') && ts.contains('T') && ts.len() == 24,
                    "timestamp must be ISO-8601 UTC with a Z and millis: {ts}"
                );
                // Every node must correspond to a real entry -- no invented nodes.
                assert!(
                    file.iter().any(|e| e["id"] == n["entry"]["id"]),
                    "the tree invented an entry not in the file: {n}"
                );
                let id = n["entry"]["id"].as_str().unwrap().to_string();
                assert!(!seen.contains(&id), "an entry appeared twice: {id}");
                seen.push(id);
                walk(n["children"].as_array().unwrap(), seen, file);
            }
        }
        let mut seen = Vec::new();
        walk(roots, &mut seen, file);
        assert!(
            seen.len() >= 3,
            "a two-run session has several entries, all of which must be in the tree: {tree}"
        );
        let max_depth = {
            fn deep(nodes: &[serde_json::Value]) -> usize {
                nodes
                    .iter()
                    .map(|n| 1 + deep(n["children"].as_array().unwrap()))
                    .max()
                    .unwrap_or(0)
            }
            deep(roots)
        };
        assert!(
            max_depth >= 2,
            "pi's tree NESTS -- a linear session must still be depth 2 or more: {tree}"
        );
    }

    /// The forest's roots carry the whole file, not just the current branch.
    ///
    /// pi's `getTree()` iterates `getEntries()` -- the WHOLE file -- and orphans become
    /// roots, so an abandoned branch still appears. The old handler returned
    /// `branch_entries()`, so after a fork the off-branch entries simply vanished from the
    /// reply. This is the assertion that distinguishes the two, and the control that
    /// restores `branch_entries()` fails here.
    #[tokio::test]
    async fn get_tree_includes_entries_off_the_current_branch() {
        let (agent, _dir) = agent_with_store(vec![
            epsilon_llm::FauxStep::say("one"),
            epsilon_llm::FauxStep::say("two"),
            epsilon_llm::FauxStep::say("three"),
        ]);
        let approvals = approvals();
        for task in ["first", "second"] {
            let mut e = agent.subscribe();
            let _ = dispatch_line(
                &format!(r#"{{"op":"prompt","task":"{task}"}}"#),
                &agent,
                &approvals,
            )
            .await;
            wait_for_end(&mut e).await;
        }
        // Jump back to a MIDDLE message and run again, so the old branch is stranded.
        // First and last are both wrong: navigating to the first leaves the branch as long
        // as the file, and navigating to the last strands nothing.
        let target = {
            let handle = agent.session_store().expect("store attached");
            let st = handle.lock().expect("lock");
            let msgs: Vec<String> = st
                .branch_entries()
                .expect("branch")
                .into_iter()
                .filter(|e| {
                    matches!(
                        e.kind,
                        epsilon_contracts::session_entry::EntryKind::Message { .. }
                    )
                })
                .map(|e| e.id)
                .collect();
            assert!(msgs.len() >= 3, "need entries on both sides: {msgs:?}");
            msgs[msgs.len() / 2].clone()
        };
        {
            let handle = agent.session_store().expect("store attached");
            let mut st = handle.lock().expect("lock");
            st.navigate_to(&target).expect("navigate");
        }
        let mut e3 = agent.subscribe();
        let _ = dispatch_line(r#"{"op":"prompt","task":"third"}"#, &agent, &approvals).await;
        wait_for_end(&mut e3).await;

        let tree = parse(&dispatch_line(r#"{"op":"get_tree"}"#, &agent, &approvals).await);
        let all = parse(&dispatch_line(r#"{"op":"get_entries"}"#, &agent, &approvals).await);
        let file = all["data"]["entries"]
            .as_array()
            .unwrap_or_else(|| panic!("entries must be an array: {all}"));

        // Count the ids in the forest, recursively.
        fn ids(nodes: &[serde_json::Value], out: &mut Vec<String>) {
            for n in nodes {
                out.push(n["entry"]["id"].as_str().unwrap().to_string());
                ids(n["children"].as_array().unwrap(), out);
            }
        }
        let mut in_tree = Vec::new();
        ids(tree["data"]["tree"].as_array().unwrap(), &mut in_tree);
        for e in file {
            let id = e["id"].as_str().unwrap();
            assert!(
                in_tree.contains(&id.to_string()),
                "entry {id} is in the file but missing from the tree; pi renders the WHOLE \\
                 file and orphans as roots, so a stranded branch must still appear"
            );
        }
        assert!(
            in_tree.len() > tree["data"]["leafId"].as_str().map(|_| 0).unwrap_or(0),
            "sanity: the forest is non-empty"
        );
    }

    /// `get_entries` returns the raw file, and `since` returns only what came after it.
    #[tokio::test]
    async fn get_entries_since_returns_only_the_newer_entries() {
        let (agent, _dir) = agent_with_store(vec![
            epsilon_llm::FauxStep::say("one"),
            epsilon_llm::FauxStep::say("two"),
        ]);
        let approvals = approvals();

        let mut events = agent.subscribe();
        let _ = dispatch_line(r#"{"op":"prompt","task":"first"}"#, &agent, &approvals).await;
        wait_for_end(&mut events).await;

        let all = parse(&dispatch_line(r#"{"op":"get_entries"}"#, &agent, &approvals).await);
        assert_eq!(all["success"], true, "{all}");
        let entries = all["data"]["entries"]
            .as_array()
            .unwrap_or_else(|| panic!("entries must be an array: {all}"));
        assert!(entries.len() >= 2, "{all}");
        let first_id = entries[0]["id"].as_str().expect("an id").to_string();

        // A second run, then ask only for what is newer than the first entry.
        let mut events2 = agent.subscribe();
        let _ = dispatch_line(r#"{"op":"prompt","task":"second"}"#, &agent, &approvals).await;
        wait_for_end(&mut events2).await;

        let since = parse(
            &dispatch_line(
                &format!(r#"{{"op":"get_entries","since":"{first_id}"}}"#),
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(since["success"], true, "{since}");
        let newer = since["data"]["entries"]
            .as_array()
            .unwrap_or_else(|| panic!("entries must be an array: {since}"));
        // Exclusive, and checked against the FILE rather than against a number I
        // predicted: the tail must be exactly the entries whose position is after the
        // named one. Asserting "the same count as before" was my first attempt and it was
        // wrong -- the second run appended more entries, so the tail is legitimately
        // longer. The property is the set difference, not a length.
        let all_now = parse(&dispatch_line(r#"{"op":"get_entries"}"#, &agent, &approvals).await);
        let everything = all_now["data"]["entries"]
            .as_array()
            .unwrap_or_else(|| panic!("entries must be an array: {all_now}"));
        let cut = everything
            .iter()
            .position(|e| e["id"].as_str() == Some(first_id.as_str()))
            .expect("the first entry is still in the file");
        let expected: Vec<_> = everything[cut + 1..].iter().collect();
        let got: Vec<_> = newer.iter().collect();
        assert_eq!(
            got.len(),
            expected.len(),
            "the tail must be everything after the named entry: {since}"
        );
        for (g, e) in got.iter().zip(expected.iter()) {
            assert_eq!(
                g["id"], e["id"],
                "the tail must be the file's own suffix, in order: {since}"
            );
        }
        assert_ne!(
            newer[0]["id"].as_str(),
            Some(first_id.as_str()),
            "`since` is EXCLUSIVE -- the named entry must not be in its own tail: {since}"
        );
    }

    /// A `since` that names nothing returns everything, not nothing.
    ///
    /// An unresolvable `since` means the client has fallen behind. An empty reply would
    /// have it conclude the session is empty, which is unrecoverable; over-sending is not.
    #[tokio::test]
    async fn get_entries_with_an_unresolvable_since_returns_everything() {
        let (agent, _dir) = agent_with_store(vec![epsilon_llm::FauxStep::say("hi")]);
        let approvals = approvals();
        let mut events = agent.subscribe();
        let _ = dispatch_line(r#"{"op":"prompt","task":"hi"}"#, &agent, &approvals).await;
        wait_for_end(&mut events).await;

        let all = parse(&dispatch_line(r#"{"op":"get_entries"}"#, &agent, &approvals).await);
        let bogus = parse(
            &dispatch_line(
                r#"{"op":"get_entries","since":"ent_does_not_exist"}"#,
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(bogus["success"], true, "{bogus}");
        assert_eq!(
            bogus["data"]["entries"], all["data"]["entries"],
            "an unknown `since` must over-send rather than look like an empty session: {bogus}"
        );
    }

    /// The three store-backed ops refuse, by name, when no store is attached.
    ///
    /// An empty `entries: []` would read as "this session has no history" and send a client
    /// hunting a bug that is not there; the real cause is that there is no store.
    #[tokio::test]
    async fn store_backed_ops_say_why_when_there_is_no_store() {
        let agent = agent_with(vec![], 0);
        let approvals = approvals();
        for op in ["get_entries", "get_tree", "set_session_name"] {
            let frame = if op == "set_session_name" {
                r#"{"op":"set_session_name","name":"x"}"#.to_string()
            } else {
                format!(r#"{{"op":"{op}"}}"#)
            };
            let reply = parse(&dispatch_line(&frame, &agent, &approvals).await);
            assert_eq!(reply["success"], false, "{op}: {reply}");
            assert!(
                reply["error"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("no session store"),
                "{op} must name the real cause: {reply}"
            );
        }
    }

    /// The three new ops accept pi's `type` tag.
    #[tokio::test]
    async fn the_m189_ops_accept_pis_type_tag() {
        let (agent, _dir) = agent_with_store(vec![]);
        let approvals = approvals();
        for frame in [
            r#"{"type":"get_entries"}"#,
            r#"{"type":"get_tree"}"#,
            r#"{"type":"set_session_name","name":"tagged"}"#,
        ] {
            let reply = parse(&dispatch_line(frame, &agent, &approvals).await);
            assert_eq!(reply["success"], true, "{frame}: {reply}");
        }
    }

    /// **`epoch_millis_to_iso8601` against the real calendar, not against my arithmetic.**
    ///
    /// The M190 milestone recorded this function as "reasoned, not measured" — the leap-year
    /// cases were things I believed about the algorithm rather than things I had checked. This
    /// closes that, and the three cases below are the ones that catch the classic
    /// days-from-civil mistakes: a 400-year-cycle boundary that IS a leap year (2000), a
    /// century that is NOT (2100), and the epoch itself.
    ///
    /// The expected values are the ISO forms of the instants named in each case, and the
    /// *independent* check is that they parse back to the same millisecond count — a string
    /// in the right format carrying the wrong time would satisfy a format assertion alone.
    #[test]
    fn iso8601_conversion_is_correct_at_the_calendar_boundaries() {
        // 2000-02-29: divisible by 400, so a leap day.
        assert_eq!(
            epoch_millis_to_iso8601(951_782_400_000),
            "2000-02-29T00:00:00.000Z"
        );
        // 2100-01-01: divisible by 100 but NOT by 400, so NOT a leap day. A conversion that
        // treats every century year as a leap year is off by a day from here on.
        assert_eq!(
            epoch_millis_to_iso8601(4_102_444_800_000),
            "2100-01-01T00:00:00.000Z"
        );
        // The epoch, and one second past it -- the floor of the range.
        assert_eq!(epoch_millis_to_iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(epoch_millis_to_iso8601(1_000), "1970-01-01T00:00:01.000Z");
        // Millisecond precision is preserved, and the suffix is UTC -- a timestamp with no
        // offset is a bug waiting for a client in another timezone.
        assert_eq!(
            epoch_millis_to_iso8601(1_790_584_501_907),
            "2026-09-28T08:35:01.907Z"
        );
        // 2026-12-31T23:59:59.999Z, the other end of the year, to catch a year-boundary
        // slip the epoch cases do not reach.
        //
        // The millisecond here was computed from the calendar, not by hand. I wrote
        // 1_798_708_799_999 first and it was off by 14_600_000 ms -- four hours and one
        // second -- because I did the year arithmetic in my head instead of deriving it.
        // The function was right and the EXPECTED VALUE was wrong, twice, including for
        // 2100 above. An expected value you have not derived is a test that will tell you
        // the implementation is broken; deriving it is the whole point of the assertion.
        assert_eq!(
            epoch_millis_to_iso8601(1_798_761_599_999),
            "2026-12-31T23:59:59.999Z"
        );
    }

    /// Every node's `timestamp` must PARSE, not merely look right.
    ///
    /// A format assertion (`len == 24`, `ends_with('Z')`) is satisfied by a string carrying
    /// the wrong instant. Parsing the fields back and checking they describe the same
    /// moment is the property a client depends on: pi's own `getTree()` sorts children with
    /// `new Date(a.entry.timestamp)`, so a string that does not parse is a client-side throw.
    #[test]
    fn every_iso_timestamp_round_trips_through_its_own_fields() {
        for ms in [
            0u64,
            1,
            999,
            1_000,
            951_782_400_000,
            1_798_761_599_999,
            4_102_444_800_000,
        ] {
            let s = epoch_millis_to_iso8601(ms);
            assert_eq!(s.len(), 24, "not a millisecond ISO instant: {s}");
            assert!(s.ends_with('Z'), "must be UTC: {s}");
            // yyyy-mm-ddThh:mm:ss.mmm
            let (date, time) = s.split_once('T').expect("a T separator");
            let (y, rest) = date.split_once('-').expect("a year separator");
            let (mo, d) = rest.split_once('-').expect("a month separator");
            let (h, rest) = time.split_once(':').expect("an hour separator");
            let (mi, sec_ms) = rest.split_once(':').expect("a minute separator");
            // The millisecond field still carries the `Z`; strip it before parsing, and
            // say why rather than letting `parse` fail with an opaque InvalidDigit.
            let (sec, milli_z) = sec_ms.split_once('.').expect("a millisecond separator");
            let milli = milli_z
                .strip_suffix('Z')
                .expect("the assertion above already checked the Z");
            // The decisive check: do these fields describe the instant we started from?
            // Days-from-civil forward, which is the inverse of the function under test.
            let y: i64 = y.parse().unwrap();
            let mo: i64 = mo.parse().unwrap();
            let d: i64 = d.parse().unwrap();
            let (h, mi, sec, milli) = (
                h.parse::<i64>().unwrap(),
                mi.parse::<i64>().unwrap(),
                sec.parse::<i64>().unwrap(),
                milli.parse::<i64>().unwrap(),
            );
            let y_adj = y - i64::from(mo <= 2);
            let era = y_adj.div_euclid(400);
            let yoe = y_adj - era * 400;
            let mp = if mo > 2 { mo - 3 } else { mo + 9 };
            let doy = (153 * mp + 2) / 5 + d - 1;
            let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
            let days = era * 146_097 + doe - 719_468;
            let secs = days * 86_400 + h * 3600 + mi * 60 + sec;
            let back = secs as u64 * 1000 + milli as u64;
            assert_eq!(back, ms, "{s} does not describe {ms}");
        }
    }

    // ---- M192: fork, clone, new_session, switch_session ----

    /// Run a task to completion, so the store has real entries behind it.
    async fn run_task(agent: &Arc<Agent>, approvals: &Arc<ChannelApprovalUi>, task: &str) {
        let mut events = agent.subscribe();
        let _ = dispatch_line(
            &format!(r#"{{"op":"prompt","task":"{task}"}}"#),
            agent,
            approvals,
        )
        .await;
        wait_for_end(&mut events).await;
    }

    /// A store-backed agent with two completed runs, so a fork has something to fork.
    async fn agent_with_two_runs() -> (Arc<Agent>, std::path::PathBuf) {
        let (agent, dir) = agent_with_store(vec![
            epsilon_llm::FauxStep::say("one"),
            epsilon_llm::FauxStep::say("two"),
        ]);
        let approvals = approvals();
        run_task(&agent, &approvals, "first").await;
        run_task(&agent, &approvals, "second").await;
        (agent, dir)
    }

    /// **`fork` writes a new file AND rebinds the agent to it.**
    ///
    /// Both halves are asserted, because the rebind is the half a shape test misses. A
    /// handler that wrote the file and left the agent pointing at the old one would pass a
    /// "did a new file appear" check and be silently wrong: the next prompt would persist
    /// to the file nobody is reading. The third assertion — a run AFTER the fork lands in
    /// the new file — is what actually proves the rebind.
    #[tokio::test]
    async fn fork_writes_a_new_file_and_rebinds_the_agent_to_it() {
        let (agent, _dir) = agent_with_two_runs().await;
        let approvals = approvals();

        let before = {
            let h = agent.session_store().expect("store");
            let g = h.lock().expect("lock");
            (g.path().to_path_buf(), g.read_all().expect("read").len())
        };
        assert!(
            before.1 >= 2,
            "two runs must have written entries: {}",
            before.1
        );

        // Fork at the FIRST entry, so the fork is a strictly shorter conversation.
        let target = {
            let h = agent.session_store().expect("store");
            let g = h.lock().expect("lock");
            g.branch_entries()
                .expect("branch")
                .first()
                .expect("an entry")
                .id
                .clone()
        };
        let reply = parse(
            &dispatch_line(
                &format!(r#"{{"op":"fork","entryId":"{target}"}}"#),
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(reply["success"], true, "{reply}");
        // pi's payload: `{ text, cancelled }`.
        assert_eq!(reply["data"]["cancelled"], false, "{reply}");
        assert!(
            reply["data"]["text"]["id"].as_str() == Some(target.as_str()),
            "the reply must name the entry it forked from: {reply}"
        );

        // Half 1: a DIFFERENT file, and it exists.
        let after_path = {
            let h = agent.session_store().expect("store");
            let g = h.lock().expect("lock");
            g.path().to_path_buf()
        };
        assert_ne!(
            after_path, before.0,
            "the agent must point at a new file: {after_path:?}"
        );
        assert!(after_path.exists(), "{after_path:?} must exist");
        assert_ne!(
            after_path.parent(),
            None,
            "the fork lives beside the original, where list_sessions can find it"
        );

        // Half 2: the conversation was re-derived from the FORK, not carried over. The fork
        // is at the first entry, so the agent's history is much shorter than it was.
        let forked_len = agent.history().len();
        assert!(
            forked_len < before.1 * 2,
            "the agent must now hold the FORK's conversation, not the original's: \
             history={forked_len} entries_in_original_file={}",
            before.1
        );
        // Half 3 — a run after the fork landing in the NEW file — is a separate test,
        // because this agent has no faux steps left to run with. Naming the gap beats
        // leaving a stub that looks like coverage.
    }

    /// A run issued AFTER a fork appends to the forked file, not the original.
    ///
    /// This is the assertion the "did a new file appear" check cannot make, and it is the
    /// one that would catch a handler that wrote the fork and forgot to rebind — the exact
    /// failure `rebind_to`'s two-step order exists to prevent.
    #[tokio::test]
    async fn a_run_after_a_fork_persists_to_the_forked_file() {
        let (agent, _dir) = agent_with_store(vec![
            epsilon_llm::FauxStep::say("one"),
            epsilon_llm::FauxStep::say("two"),
            epsilon_llm::FauxStep::say("three"),
        ]);
        let approvals = approvals();
        run_task(&agent, &approvals, "first").await;
        run_task(&agent, &approvals, "second").await;

        let target = {
            let h = agent.session_store().expect("store");
            let g = h.lock().expect("lock");
            g.branch_entries()
                .expect("branch")
                .first()
                .expect("an entry")
                .id
                .clone()
        };
        let reply = parse(
            &dispatch_line(
                &format!(r#"{{"op":"fork","entryId":"{target}"}}"#),
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(reply["success"], true, "{reply}");
        let forked_path = {
            let h = agent.session_store().expect("store");
            let g = h.lock().expect("lock");
            g.path().to_path_buf()
        };
        let forked_entries = {
            let h = agent.session_store().expect("store");
            let g = h.lock().expect("lock");
            g.read_all().expect("read").len()
        };

        // One more run, with a step still queued.
        run_task(&agent, &approvals, "third").await;

        let now = {
            let h = agent.session_store().expect("store");
            let g = h.lock().expect("lock");
            (g.path().to_path_buf(), g.read_all().expect("read").len())
        };
        assert_eq!(now.0, forked_path, "the agent must still be on the fork");
        assert!(
            now.1 > forked_entries,
            "the post-fork run must append to the FORK: {forked_entries} -> {}",
            now.1
        );
    }

    /// `fork` names an entry that is not there, by name.
    #[tokio::test]
    async fn fork_refuses_an_entry_id_that_does_not_exist() {
        let (agent, _dir) = agent_with_two_runs().await;
        let approvals = approvals();
        let reply = parse(
            &dispatch_line(r#"{"op":"fork","entryId":"ent_nope"}"#, &agent, &approvals).await,
        );
        assert_eq!(reply["success"], false, "{reply}");
        assert!(
            !reply["error"].as_str().unwrap_or_default().is_empty(),
            "the error must say something: {reply}"
        );
    }

    /// `clone` duplicates the current leaf, and pi's refusal when there is no leaf.
    #[tokio::test]
    async fn clone_duplicates_the_current_leaf() {
        let (agent, _dir) = agent_with_two_runs().await;
        let approvals = approvals();
        let before = {
            let h = agent.session_store().expect("store");
            let g = h.lock().expect("lock");
            g.path().to_path_buf()
        };
        let reply = parse(&dispatch_line(r#"{"op":"clone"}"#, &agent, &approvals).await);
        assert_eq!(reply["success"], true, "{reply}");
        assert_eq!(reply["data"]["cancelled"], false, "{reply}");
        let after = {
            let h = agent.session_store().expect("store");
            let g = h.lock().expect("lock");
            g.path().to_path_buf()
        };
        assert_ne!(after, before, "clone must produce a new file");
        assert!(after.exists(), "{after:?}");
    }

    /// `clone` on a session with nothing in it refuses, as pi does.
    ///
    /// pi: `if (!leafId) return error(id, "clone", "Cannot clone session: no current entry
    /// selected")`. A clone that produced an empty session and reported success would be a
    /// client told "duplicated" about a session with no history.
    #[tokio::test]
    async fn clone_refuses_when_there_is_no_leaf() {
        let (agent, _dir) = agent_with_store(vec![]);
        let approvals = approvals();
        let reply = parse(&dispatch_line(r#"{"op":"clone"}"#, &agent, &approvals).await);
        assert_eq!(reply["success"], false, "{reply}");
        assert!(
            reply["error"]
                .as_str()
                .unwrap_or_default()
                .contains("no current entry"),
            "the error must name the real cause, as pi's does: {reply}"
        );
    }

    /// `new_session` starts empty, and `switch_session` goes back to the old file.
    ///
    /// The round trip is the point: a client that can only start a new session but never
    /// return to the previous one has a one-way door. Both files must survive, and the
    /// second half of the test switches back and proves the original conversation is
    /// intact — which also exercises `reproject_current_branch` in the restore direction.
    #[tokio::test]
    async fn new_session_then_switch_session_round_trips() {
        let (agent, _dir) = agent_with_two_runs().await;
        let approvals = approvals();
        let original = {
            let h = agent.session_store().expect("store");
            let g = h.lock().expect("lock");
            (g.path().to_path_buf(), g.read_all().expect("read").len())
        };

        let made = parse(&dispatch_line(r#"{"op":"new_session"}"#, &agent, &approvals).await);
        assert_eq!(made["success"], true, "{made}");
        let fresh = made["data"]["sessionFile"]
            .as_str()
            .expect("sessionFile")
            .to_string();
        assert!(std::path::Path::new(&fresh).exists(), "{fresh} must exist");

        // The new session is EMPTY — that is what "new" means, and a test that only checked
        // the file exists would pass a handler that copied the transcript across.
        let empty = {
            let h = agent.session_store().expect("store");
            let g = h.lock().expect("lock");
            g.read_all().expect("read").len()
        };
        assert_eq!(empty, 0, "a new session must start empty: {made}");
        assert_eq!(
            agent.history().len(),
            0,
            "and the agent must see it as empty"
        );

        // Go back.
        let back = parse(
            &dispatch_line(
                // Built as a VALUE and serialized, never interpolated into a string
                // literal: a Windows path is full of backslashes, and `"C:\Users\…"`
                // inside a JSON string is an escape error. A test that hand-writes the path
                // fails on Windows and passes on Linux -- the wrong way round for a harness
                // whose primary platform is Windows.
                &serde_json::json!({
                    "op": "switch_session",
                    "sessionPath": original.0.to_string_lossy(),
                })
                .to_string(),
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(back["success"], true, "{back}");
        let restored = {
            let h = agent.session_store().expect("store");
            let g = h.lock().expect("lock");
            (g.path().to_path_buf(), g.read_all().expect("read").len())
        };
        assert_eq!(restored.0, original.0, "must be back on the original file");
        assert_eq!(restored.1, original.1, "with its entries intact");
        assert!(
            agent.history().len() >= 2,
            "and the agent's conversation must be re-derived from it: {}",
            agent.history().len()
        );
    }

    /// `switch_session` to a path that is not there fails by name.
    #[tokio::test]
    async fn switch_session_refuses_a_missing_file() {
        let (agent, _dir) = agent_with_two_runs().await;
        let approvals = approvals();
        let reply = parse(
            &dispatch_line(
                r#"{"op":"switch_session","sessionPath":"C:/definitely/not/here.jsonl"}"#,
                &agent,
                &approvals,
            )
            .await,
        );
        assert_eq!(reply["success"], false, "{reply}");
        assert!(
            reply["error"]
                .as_str()
                .unwrap_or_default()
                .contains("no session file"),
            "the error must name the real cause: {reply}"
        );
    }

    /// The four new ops accept pi's camelCase field names, over the wire.
    ///
    /// `entryId`, `parentSession`, `sessionPath` are the spellings in `rpc-types.ts`, and a
    /// snake_case field would parse as absent rather than as wrong — `fork` would then
    /// refuse for want of an entry id while reporting a problem the client cannot see.
    #[tokio::test]
    async fn the_m192_ops_accept_pis_camel_case_fields() {
        let (agent, _dir) = agent_with_two_runs().await;
        let approvals = approvals();
        let target = {
            let h = agent.session_store().expect("store");
            let g = h.lock().expect("lock");
            g.branch_entries()
                .expect("branch")
                .first()
                .expect("an entry")
                .id
                .clone()
        };
        for frame in [
            format!(r#"{{"op":"fork","entryId":"{target}"}}"#),
            r#"{"op":"new_session","parentSession":null}"#.to_string(),
        ] {
            let reply = parse(&dispatch_line(&frame, &agent, &approvals).await);
            assert_eq!(reply["success"], true, "{frame}: {reply}");
        }
    }

    /// **`new_session` must empty the agent's CONVERSATION, not just its file.**
    ///
    /// This is the assertion Control H demanded. That control swapped the mirror and left
    /// the conversation alone, and all 59 tests passed — because the round-trip test only
    /// checked file paths and entry counts. The defect it hides is the worst kind: the
    /// session file says "empty" while the agent carries the previous session's messages
    /// into the next provider call, so the user starts a fresh conversation and the model
    /// answers as if the old one were still in context.
    ///
    /// Two independent facts are asserted, because either alone is satisfiable by accident:
    /// the store holds no entries, and the agent's own history is empty.
    #[tokio::test]
    async fn new_session_empties_the_conversation_not_just_the_file() {
        let (agent, _dir) = agent_with_two_runs().await;
        let approvals = approvals();
        // The precondition, as an assertion. A session that never ran makes "empty" true
        // for the wrong reason -- this is the M188/M189 precondition lesson again.
        assert!(
            agent.history().len() >= 2,
            "two runs must be in the conversation before we empty it: {}",
            agent.history().len()
        );

        let made = parse(&dispatch_line(r#"{"op":"new_session"}"#, &agent, &approvals).await);
        assert_eq!(made["success"], true, "{made}");

        assert_eq!(
            agent.history().len(),
            0,
            "the agent must carry NO messages from the previous session into the new one: \
             {} left over",
            agent.history().len()
        );
        // And the file agrees, so the two cannot drift.
        let file_entries = {
            let h = agent.session_store().expect("store");
            let g = h.lock().expect("lock");
            g.read_all().expect("read").len()
        };
        assert_eq!(file_entries, 0, "the new file must be empty too: {made}");

        // The decisive third fact: a run in the new session must produce a conversation
        // containing ONLY that run. A stale carry-over shows up here even if `history()`
        // were somehow lazily re-read.
        let (fresh_agent, _) = agent_with_store(vec![epsilon_llm::FauxStep::say("only me")]);
        let made2 =
            parse(&dispatch_line(r#"{"op":"new_session"}"#, &fresh_agent, &approvals).await);
        assert_eq!(made2["success"], true, "{made2}");
        let mut events = fresh_agent.subscribe();
        let _ = dispatch_line(
            r#"{"op":"prompt","task":"hello"}"#,
            &fresh_agent,
            &approvals,
        )
        .await;
        wait_for_end(&mut events).await;
        let texts: Vec<String> = fresh_agent.history().iter().map(|m| m.text()).collect();
        assert!(
            texts.iter().all(|t| t != "one" && t != "two"),
            "no message from an earlier session may survive: {texts:?}"
        );
        // `!t.is_empty()` would be the tempting shorthand here and it is vacuous: it is
        // true of every message that exists, so it asserts only that the run produced
        // *something*. The check has to name the content that should be there.
        assert!(
            texts.iter().any(|t| t.contains("only me")),
            "the new session's own reply must be present: {texts:?}"
        );
    }

    /// **`get_entries` and `get_tree` must emit the SAME entry shape.**
    ///
    /// This is D29, and it is D27's twin: `get_tree` returned pi's
    /// `{type, id, parentId, timestamp}` while `get_entries` returned epsilon's internal
    /// `{id, kind, parent_id, seq, timestamp_ms}`. Two ops, one reference type, two
    /// shapes — and **the M190 in-process tests passed**, because they asserted
    /// `get_entries` against itself. Found only by driving the installed binary over real
    /// stdio and reading the raw JSON.
    ///
    /// The assertion is deliberately a *comparison* rather than a fixed literal. A literal
    /// would be my guess again, and a guess is what produced both defects. What cannot be
    /// wrong is that one reference type cannot be served two ways: whichever fields each op
    /// emits, they must be the same fields with the same names and the same value types.
    /// That is checkable without knowing the right answer, which is the point.
    #[tokio::test]
    async fn get_entries_and_get_tree_agree_on_every_entries_shape() {
        let (agent, _dir) = agent_with_two_runs().await;
        let approvals = approvals();

        let entries_reply =
            parse(&dispatch_line(r#"{"op":"get_entries"}"#, &agent, &approvals).await);
        let tree_reply = parse(&dispatch_line(r#"{"op":"get_tree"}"#, &agent, &approvals).await);
        assert_eq!(entries_reply["success"], true, "{entries_reply}");
        assert_eq!(tree_reply["success"], true, "{tree_reply}");

        let entries = entries_reply["data"]["entries"]
            .as_array()
            .unwrap_or_else(|| panic!("entries must be an array: {entries_reply}"));
        assert!(!entries.is_empty(), "the precondition: entries must exist");

        // Every tree node's entry, flattened out of the forest.
        fn flatten(nodes: &[serde_json::Value], out: &mut Vec<serde_json::Value>) {
            for n in nodes {
                out.push(n["entry"].clone());
                if let Some(kids) = n["children"].as_array() {
                    flatten(kids, out);
                }
            }
        }
        let mut tree_entries = Vec::new();
        flatten(
            tree_reply["data"]["tree"].as_array().expect("a tree array"),
            &mut tree_entries,
        );
        assert_eq!(
            tree_entries.len(),
            entries.len(),
            "both ops must cover the SAME entries: {} vs {}",
            tree_entries.len(),
            entries.len()
        );

        // The comparison. Same key set, same order-insensitive, same JSON value types.
        for (from_list, from_tree) in entries.iter().zip(tree_entries.iter()) {
            let mut a: Vec<String> = from_list
                .as_object()
                .expect("an object")
                .keys()
                .cloned()
                .collect();
            let mut b: Vec<String> = from_tree
                .as_object()
                .expect("an object")
                .keys()
                .cloned()
                .collect();
            a.sort();
            b.sort();
            assert_eq!(
                a, b,
                "get_entries and get_tree disagree on the entry's fields: {from_list} vs {from_tree}"
            );
            for k in &a {
                assert_eq!(
                    type_name(&from_list[k]),
                    type_name(&from_tree[k]),
                    "field `{k}` has a different JSON type between the two ops: \
                     {} vs {}",
                    from_list[k],
                    from_tree[k]
                );
            }
            // And the identifying value must be the identical STRING, not merely a
            // string-shaped thing: the same entry id in both.
            assert_eq!(
                from_list["id"].as_str(),
                from_tree["id"].as_str(),
                "the same entry must carry the same id in both ops"
            );
        }

        /// The JSON type name, so a number-vs-string difference is caught rather than
        /// compared loosely.
        fn type_name(v: &serde_json::Value) -> &'static str {
            match v {
                serde_json::Value::Null => "null",
                serde_json::Value::Bool(_) => "bool",
                serde_json::Value::Number(_) => "number",
                serde_json::Value::String(_) => "string",
                serde_json::Value::Array(_) => "array",
                serde_json::Value::Object(_) => "object",
            }
        }
    }

    /// The emitted fields are pi's four, and `timestamp` is an ISO string.
    ///
    /// The comparison test above cannot tell a *consistent* wrong shape from the right
    /// one — two ops agreeing on `{id, kind, seq}` would satisfy it. This test pins the
    /// four names and the timestamp's type against `session-manager.ts`'s
    /// `SessionEntryBase`, which is the reference line rather than my reading of it.
    #[tokio::test]
    async fn every_get_entries_entry_has_pis_four_fields() {
        let (agent, _dir) = agent_with_two_runs().await;
        let approvals = approvals();
        let reply = parse(&dispatch_line(r#"{"op":"get_entries"}"#, &agent, &approvals).await);
        let entries = reply["data"]["entries"]
            .as_array()
            .unwrap_or_else(|| panic!("entries must be an array: {reply}"));
        assert!(!entries.is_empty(), "{reply}");
        for e in entries {
            // pi: `SessionEntryBase { type; id; parentId; timestamp }` -- and `type` is a
            // plain string, not the nested `kind` OBJECT epsilon serialized internally.
            let obj = e.as_object().expect("an object");
            let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
            keys.sort();
            assert_eq!(
                keys,
                vec!["id", "parentId", "timestamp", "type"],
                "an entry must carry exactly pi's four fields: {e}"
            );
            assert!(
                e["type"].is_string(),
                "`type` must be a string, not epsilon's nested kind object: {e}"
            );
            assert!(
                e["timestamp"].is_string(),
                "`timestamp` must be an ISO-8601 STRING, not epoch millis: {e}"
            );
            let ts = e["timestamp"].as_str().expect("a string");
            assert_eq!(ts.len(), 24, "millisecond ISO instant: {ts}");
            assert!(ts.ends_with('Z'), "must be UTC: {ts}");
            // And the four internal names must be GONE -- the defect was their presence,
            // so asserting their absence is what pins the fix.
            for gone in ["parent_id", "seq", "timestamp_ms", "kind"] {
                assert!(
                    !obj.contains_key(gone),
                    "`{gone}` is epsilon's internal name and must not reach the wire: {e}"
                );
            }
        }
    }
}
