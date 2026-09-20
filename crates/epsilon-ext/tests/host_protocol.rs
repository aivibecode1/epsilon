//! Host protocol hardening: what an untrusted or merely broken extension child can do to
//! the host by choosing what to put on stdout.
//!
//! The extension child is third-party code. Every line it writes is attacker-controlled
//! text, and the frames the host accepts become `ContentBlock::text` in the model's
//! transcript, so the interesting cases are not the happy path (already covered by
//! `host.rs`) but the four failure categories the error enum names: `Timeout`, `Crashed`,
//! `Protocol` and `ToolFailed`.
//!
//! Every wait here is bounded. A misbehaving child must fail a test, never wedge the
//! suite, so no test trusts the host to return — each one caps the wall clock and says
//! what it saw.

mod support;

use std::time::Duration;

use epsilon_ext::manifest::ExtManifest;
use epsilon_ext::{ExtError, ExtHost};
use support::{MANIFEST_TIMEOUT, awake};

fn fixture(mode: &str) -> ExtManifest {
    ExtManifest {
        name: "fixture".into(),
        version: "0.1.0".into(),
        bin: support::fixture_exe(),
        args: vec![mode.to_string()],
        events: vec![],
        tools: vec![],
    }
}

/// Spawn bounded: a child that misbehaves during the handshake must fail, not hang.
async fn spawn(dir: &std::path::Path, mode: &str) -> Result<std::sync::Arc<ExtHost>, ExtError> {
    let manifest = fixture(mode);
    match tokio::time::timeout(MANIFEST_TIMEOUT, ExtHost::spawn(dir, &manifest)).await {
        Ok(r) => r,
        Err(_) => panic!(
            "spawn({mode}) did not settle within {MANIFEST_TIMEOUT:?}; the host's own handshake \
             deadline is far shorter, so reaching this bound means the child never started or \
             never went away"
        ),
    }
}

/// The error half of a failed spawn. `ExtHost` plausibly has no `Debug`, so a failed
/// `spawn` is unwrapped here where the failure message can name the mode.
async fn spawn_err(dir: &std::path::Path, mode: &str) -> ExtError {
    match spawn(dir, mode).await {
        Ok(_) => panic!("spawn({mode}) was expected to fail"),
        Err(e) => e,
    }
}

/// Spawn a known-good host, or fail the test rather than propagate.
async fn spawn_ok(dir: &std::path::Path, mode: &str) -> std::sync::Arc<ExtHost> {
    match spawn(dir, mode).await {
        Ok(h) => h,
        Err(e) => panic!("spawn({mode}) failed: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Lines that are not JSON, and JSON of the wrong shape.
// ---------------------------------------------------------------------------

/// A result line that is not JSON at all must be reported as a protocol violation, and
/// the offending text must NOT be echoed back: that text would otherwise land in the
/// model's transcript behind a `[harness]` prefix while carrying the child's own words.
#[tokio::test]
async fn non_json_result_line_is_a_protocol_error_that_is_not_echoed() {
    let dir = tempfile::tempdir().unwrap();
    let host = spawn_ok(dir.path(), "--garbage").await;

    let err = awake(
        host.invoke("ext_echo", "tc_1", serde_json::json!({})),
        Duration::from_secs(5),
        "invoke against a child that answers with plain text",
    )
    .await
    .expect_err("a non-JSON line is not a result");

    let msg = err.to_string();
    assert!(
        matches!(err, ExtError::Protocol(_)),
        "expected Protocol, got {err:?}"
    );
    assert!(
        !msg.contains(support::GARBAGE_MARKER),
        "the child's raw line was echoed into the error text (and so into the transcript): {msg}"
    );
}

/// A JSON line that is a valid document but not a result frame is chatter: the host keeps
/// reading rather than mistaking it for an answer.
#[tokio::test]
async fn wrong_shaped_json_lines_are_skipped_until_the_real_result() {
    let dir = tempfile::tempdir().unwrap();
    let host = spawn_ok(dir.path(), "--shape-noise").await;

    let (text, _) = awake(
        host.invoke("ext_echo", "tc_1", serde_json::json!({"text": "ok"})),
        Duration::from_secs(5),
        "invoke against a child that emits valid JSON of the wrong shape",
    )
    .await
    .expect("wrong-shaped frames are noise, not the answer");

    assert_eq!(text, "OK");
}

/// The id is the only thing tying a frame to a pending request. A frame carrying some
/// other id must be ignored even when it looks exactly like a result, otherwise a child
/// could answer a question nobody asked and have it attributed to the live call.
#[tokio::test]
async fn a_result_for_another_id_is_not_accepted_as_this_one() {
    let dir = tempfile::tempdir().unwrap();
    let host = spawn_ok(dir.path(), "--wrong-id").await;

    let (text, _) = awake(
        host.invoke("ext_echo", "tc_1", serde_json::json!({"text": "real"})),
        Duration::from_secs(5),
        "invoke while the child sends a foreign id first",
    )
    .await
    .expect("the correctly-id'd frame must still be accepted");

    assert_eq!(
        text, "REAL",
        "the frame for an unknown id was accepted as this invoke's answer"
    );
}

/// A `result` frame missing the fields the protocol defines must not be promoted to a
/// successful tool call with empty content: an empty success is indistinguishable from a
/// tool that legitimately returned nothing, and the model would be told the tool ran.
#[tokio::test]
async fn a_result_frame_missing_its_fields_is_a_protocol_error() {
    let dir = tempfile::tempdir().unwrap();
    let host = spawn_ok(dir.path(), "--bad-shape").await;

    let err = awake(
        host.invoke("ext_echo", "tc_1", serde_json::json!({})),
        Duration::from_secs(5),
        "invoke against a child that sends a result with no text/details",
    )
    .await
    .expect_err("a malformed result frame must not read as a successful empty result");

    assert!(
        matches!(err, ExtError::Protocol(_)),
        "expected Protocol, got {err:?}"
    );
    assert!(
        err.to_string().contains("text"),
        "the error should name the first field that was missing: {err}"
    );
}

/// A frame that carries text but no `details` is refused too. `details` is not optional
/// decoration: the tool wrapper writes the extension's name into it.
#[tokio::test]
async fn a_result_frame_without_details_is_a_protocol_error() {
    let dir = tempfile::tempdir().unwrap();
    let host = spawn_ok(dir.path(), "--no-details").await;

    let err = awake(
        host.invoke("ext_echo", "tc_1", serde_json::json!({})),
        Duration::from_secs(5),
        "invoke against a child whose result frame omits details",
    )
    .await
    .expect_err("a result with no details must not be accepted as an unattributed success");

    assert!(
        matches!(err, ExtError::Protocol(_)),
        "expected Protocol, got {err:?}"
    );
    assert!(
        err.to_string().contains("details"),
        "the error should name the missing field: {err}"
    );
}

/// Fields of the wrong type are rejected the same way. This is the case that decides
/// whether a broken child can quietly strip the extension's own attribution: the tool
/// wrapper inserts its name into `details`, and a `details` that is not an object has
/// nowhere for that to go.
#[tokio::test]
async fn a_result_frame_with_wrong_field_types_is_a_protocol_error() {
    let dir = tempfile::tempdir().unwrap();
    let host = spawn_ok(dir.path(), "--wrong-types").await;

    let err = awake(
        host.invoke("ext_echo", "tc_1", serde_json::json!({})),
        Duration::from_secs(5),
        "invoke against a child that sends a result with wrong field types",
    )
    .await
    .expect_err("a result whose fields have the wrong types must not be accepted");

    let msg = err.to_string();
    assert!(matches!(err, ExtError::Protocol(_)), "got {err:?}");
    assert!(
        msg.contains("text"),
        "the error should name the first field that was wrong: {msg}"
    );
    assert!(
        msg.contains("number"),
        "the error should say what it saw instead: {msg}"
    );
}

/// The extension's own failure channel: a well-formed result with `is_error: true` is
/// `ToolFailed` carrying the child's message, not a protocol violation and not a crash.
/// The message is the point — it is what the model is told when a tool fails, so it must
/// survive the trip unchanged.
#[tokio::test]
async fn a_well_formed_failure_result_becomes_a_tool_failure() {
    let dir = tempfile::tempdir().unwrap();
    let host = spawn_ok(dir.path(), "--tool-error").await;

    let err = awake(
        host.invoke("ext_echo", "tc_1", serde_json::json!({})),
        Duration::from_secs(5),
        "invoke against a child that reports a tool error",
    )
    .await
    .expect_err("an error result is not a successful call");

    match err {
        ExtError::ToolFailed { message } => assert_eq!(
            message,
            support::TOOL_ERROR,
            "the extension's message must reach the model verbatim"
        ),
        other => panic!("expected ToolFailed, got {other:?}"),
    }
    assert!(
        !host.is_disabled(),
        "reporting a tool failure is not a crash and must not spend the budget"
    );
}

/// A protocol violation is not a crash: it must not spend the crash budget, or a child
/// that merely emits one stray log line would burn through the budget and get disabled.
#[tokio::test]
async fn protocol_violations_do_not_spend_the_crash_budget() {
    let dir = tempfile::tempdir().unwrap();
    let host = spawn_ok(dir.path(), "--garbage").await;

    for i in 0..3 {
        let err = awake(
            host.invoke("ext_echo", &format!("tc_{i}"), serde_json::json!({})),
            Duration::from_secs(5),
            "repeated protocol violations",
        )
        .await
        .expect_err("garbage is never a result");
        assert!(matches!(err, ExtError::Protocol(_)), "got {err:?}");
    }
    assert!(
        !host.is_disabled(),
        "protocol violations were charged to the crash budget"
    );
}

// ---------------------------------------------------------------------------
// Timeouts: a child that never answers, answers late, or dribbles lines.
// ---------------------------------------------------------------------------

/// Never answering hits the invoke timeout. The child is left blocked on its own stdin
/// read, so this also pins the timeout as a *host* property rather than a child one.
#[tokio::test]
async fn a_child_that_never_replies_times_out() {
    let dir = tempfile::tempdir().unwrap();
    let host = spawn_ok(dir.path(), "--sleep").await;

    let err = awake(
        host.invoke("ext_echo", "tc_1", serde_json::json!({})),
        support::INVOKE_CAP,
        "invoke against a child that stalls past the timeout",
    )
    .await
    .expect_err("a stalled invoke must not succeed");

    assert!(
        matches!(err, ExtError::Timeout),
        "expected Timeout, got {err:?}"
    );
}

/// The timeout is per *invoke*, not per line read. The child writes a small frame every
/// 20 ms and never answers, so every individual read succeeds: a per-read timer never
/// fires and the call is held open while the documented contract promises it ends at ten
/// seconds. The trickle is ~200 KB per invoke, far below the byte budget, so the deadline
/// is the only defence that can end this call — the two are tested apart, not raced.
#[tokio::test]
async fn a_chatty_child_cannot_extend_the_invoke_timeout_indefinitely() {
    let dir = tempfile::tempdir().unwrap();
    let host = spawn_ok(dir.path(), "--pace").await;

    let started = std::time::Instant::now();
    let err = awake(
        host.invoke("ext_echo", "tc_1", serde_json::json!({})),
        support::INVOKE_CAP,
        "invoke against a child that never answers but keeps writing frames",
    )
    .await
    .expect_err("a child that answers nothing must not keep the invoke alive");

    assert!(
        matches!(err, ExtError::Timeout),
        "expected Timeout, got {err:?}"
    );
    assert!(
        started.elapsed() < support::INVOKE_CAP,
        "invoke ran {:?}, past the {:?} the timeout promises",
        started.elapsed(),
        support::INVOKE_CAP
    );
}

/// A reply that arrives after the timeout is stale: it must not be handed to the next
/// invoke as that call's answer, and the next invoke must still work.
#[tokio::test]
async fn a_reply_that_arrives_after_the_timeout_is_discarded() {
    let dir = tempfile::tempdir().unwrap();
    let host = spawn_ok(dir.path(), "--late").await;

    let first = awake(
        host.invoke("ext_echo", "tc_1", serde_json::json!({"text": "late"})),
        support::INVOKE_CAP,
        "first invoke against a child that answers late",
    )
    .await
    .expect_err("a late reply is still a timeout");
    assert!(matches!(first, ExtError::Timeout), "got {first:?}");

    // Give the stale frame time to land in the pipe, then ask a second question. The
    // stale answer belongs to `i_1`; accepting it here would return LATE.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let (text, _) = awake(
        host.invoke("ext_echo", "tc_2", serde_json::json!({"text": "second"})),
        support::INVOKE_CAP,
        "second invoke after a stale reply landed",
    )
    .await
    .expect("the second invoke must still be answerable");

    assert_eq!(
        text, "SECOND",
        "an answer to the previous, timed-out invoke was served to this one"
    );
}

// ---------------------------------------------------------------------------
// Volume: one enormous line, and a great many lines.
// ---------------------------------------------------------------------------

/// A child can write an arbitrarily long single line. The host must refuse it without
/// buffering it: the child picks the length, so an unbounded read hands it a memory
/// primitive inside the harness process. `--huge` writes 4 MiB with no newline; the host
/// rejects it on its own byte cap, long before the line ends.
#[tokio::test]
async fn an_enormous_line_is_refused_rather_than_buffered() {
    let dir = tempfile::tempdir().unwrap();
    let host = spawn_ok(dir.path(), "--huge").await;

    let err = awake(
        host.invoke("ext_echo", "tc_1", serde_json::json!({})),
        Duration::from_secs(20),
        "invoke against a child that writes one oversized line",
    )
    .await
    .expect_err("an oversized frame is not a result");

    let msg = err.to_string();
    assert!(
        matches!(err, ExtError::Protocol(_)),
        "expected Protocol for an oversized line, got {err:?}"
    );
    assert!(
        msg.contains("exceeds"),
        "the refusal should say the frame was too large: {msg}"
    );
}

/// The per-invoke byte budget: a child that streams frames forever without answering must
/// be cut off by volume, not merely by the clock, or the reader stays busy for the whole
/// timeout window on a stream it can never finish.
#[tokio::test]
async fn a_child_that_never_stops_sending_lines_is_cut_off_by_volume() {
    let dir = tempfile::tempdir().unwrap();
    // 200k receipts of 100 bytes each is ~20 MB, past the 16 MiB per-invoke budget.
    let host = spawn_ok(dir.path(), "--flood 200000 100").await;

    let err = awake(
        host.invoke("ext_echo", "tc_1", serde_json::json!({})),
        Duration::from_secs(20),
        "invoke against a child that floods without ever answering",
    )
    .await
    .expect_err("a child that never answers cannot satisfy an invoke");

    let msg = err.to_string();
    assert!(
        matches!(err, ExtError::Protocol(_)),
        "expected Protocol once the byte budget is spent, got {err:?}"
    );
    assert!(
        msg.contains("bytes"),
        "the refusal should name the budget it spent: {msg}"
    );
}

/// Line count is not a memory leak: the log channel is drained, capped, and the real
/// answer behind the flood is still read.
#[tokio::test]
async fn a_flood_of_lines_is_drained_and_the_log_channel_stays_capped() {
    let dir = tempfile::tempdir().unwrap();
    let host = spawn_ok(dir.path(), "--lines 50000").await;

    let (text, _) = awake(
        host.invoke("ext_echo", "tc_1", serde_json::json!({"text": "flood"})),
        Duration::from_secs(20),
        "invoke against a child that emits 50k log lines first",
    )
    .await
    .expect("the result behind the flood must still be read");

    assert_eq!(text, "FLOOD");
    let logs = host.logs().await;
    assert_eq!(
        logs.len(),
        1000,
        "log receipts must stay capped at 1000, got {}",
        logs.len()
    );
    assert!(logs[0].contains("line #0"), "first receipt: {}", logs[0]);
    assert!(
        logs[999].contains("line #999"),
        "last retained receipt: {}",
        logs[999]
    );
}

// ---------------------------------------------------------------------------
// Death: exit mid-invoke, exit at handshake, and the crash budget.
// ---------------------------------------------------------------------------

/// A child that exits while an invoke is pending is a crash, not a panic and not a hang.
#[tokio::test]
async fn a_child_that_exits_mid_invoke_reports_a_crash() {
    let dir = tempfile::tempdir().unwrap();
    let host = spawn_ok(dir.path(), "--exit-mid").await;

    let err = awake(
        host.invoke("ext_echo", "tc_1", serde_json::json!({})),
        Duration::from_secs(5),
        "invoke against a child that exits while the call is pending",
    )
    .await
    .expect_err("a dead child cannot answer");

    assert!(
        matches!(err, ExtError::Crashed(_)),
        "expected Crashed, got {err:?}"
    );
}

/// Exit before hello fails the spawn: an extension that dies at handshake must be
/// reported at startup, not attached as a live host whose every call hangs.
#[tokio::test]
async fn a_child_that_exits_at_handshake_fails_to_spawn() {
    let dir = tempfile::tempdir().unwrap();
    let err = spawn_err(dir.path(), "--no-hello").await;

    assert!(
        matches!(err, ExtError::Crashed(_)),
        "expected Crashed for an exit before hello, got {err:?}"
    );
}

/// A child that is alive but silent is neither a crash nor a protocol violation: the
/// handshake has its own timeout, and `attach_extensions` reports it as a failed start.
///
/// The child never speaks, so nothing it does can end this handshake — only the host's
/// deadline can. That matters because the *test's* bound must never be the thing that
/// fires: when the fixture's silence was finite (30 s) and the bound was 20 s, a loaded
/// machine lost the race and the test failed against a host that was behaving perfectly
/// (measured on this box: 1.7 s to start one small process, 23.4 s for a spawn plus
/// teardown of a silent child, and 10.03 s for the host's own timeout to fire).
#[tokio::test]
async fn a_child_that_never_says_hello_times_out_at_spawn() {
    let dir = tempfile::tempdir().unwrap();
    let err = spawn_err(dir.path(), "--slow-hello").await;

    assert!(
        matches!(err, ExtError::Timeout),
        "expected Timeout for a silent handshake, got {err:?}"
    );
}

/// A first line that is not a hello is a protocol violation at spawn, distinct from a
/// missing line (which is the timeout) and from an exit (which is a crash).
#[tokio::test]
async fn a_non_json_hello_is_a_protocol_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = spawn_err(dir.path(), "--garbage-hello").await;

    assert!(
        matches!(err, ExtError::Protocol(_)),
        "expected Protocol, got {err:?}"
    );
}

/// A binary that is not there fails at spawn with a crash, not a panic.
#[tokio::test]
async fn a_missing_binary_fails_to_spawn() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = ExtManifest {
        name: "fixture".into(),
        version: "0.1.0".into(),
        bin: dir.path().join("no-such-extension-binary"),
        args: vec![],
        events: vec![],
        tools: vec![],
    };

    let err = match ExtHost::spawn(dir.path(), &manifest).await {
        Ok(_) => panic!("a missing binary cannot spawn"),
        Err(e) => e,
    };

    assert!(
        matches!(err, ExtError::Crashed(_)),
        "expected Crashed, got {err:?}"
    );
}

/// The crash budget is per host and actually disables the extension. The old test proved
/// the counter by spawning two healthy hosts; this drives one host until the budget is
/// spent and asserts the next call is refused *without* a round trip to the child.
#[tokio::test]
async fn crashes_accumulate_on_one_host_until_it_is_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let host = spawn_ok(dir.path(), "--die").await;

    for i in 0..2 {
        let err = awake(
            host.invoke("ext_echo", &format!("tc_{i}"), serde_json::json!({})),
            Duration::from_secs(5),
            "invoke against a child that dies immediately",
        )
        .await
        .expect_err("a dying child cannot answer");
        assert!(
            matches!(err, ExtError::Crashed(_)),
            "crash {i} should be Crashed, got {err:?}"
        );
    }
    assert!(
        host.is_disabled(),
        "two crashes must exhaust the budget of a single host"
    );

    let err = awake(
        host.invoke("ext_echo", "tc_after", serde_json::json!({})),
        Duration::from_secs(5),
        "invoke against a disabled host",
    )
    .await
    .expect_err("a disabled extension must refuse");
    assert!(
        err.to_string().contains("disabled"),
        "the refusal must say the extension is disabled, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Event filtering at the host (the manifest-level test covers only the predicate).
// ---------------------------------------------------------------------------

/// `send_event` is the only path from an agent event into a child, and it was previously
/// reachable only through `spawn_event_forwarder` — the old forwarding test bypasses the
/// host entirely with a hand-rolled child. Filtering at the host is what plan 007
/// restored, so an empty list must deliver nothing and `["*"]` must deliver everything.
#[tokio::test]
async fn an_empty_event_list_delivers_nothing_and_a_star_delivers_everything() {
    let dir = tempfile::tempdir().unwrap();

    let silent = spawn_with_events(dir.path(), vec![]).await;
    deliver(&silent, "turn_end", 1).await;
    assert!(
        silent.logs().await.is_empty(),
        "an extension with an empty events list received {:?}",
        silent.logs().await
    );

    let everything = spawn_with_events(dir.path(), vec!["*".into()]).await;
    deliver(&everything, "turn_end", 1).await;
    deliver(&everything, "tool_execution_end", 2).await;
    let logs = everything.logs().await;
    assert_eq!(
        logs.len(),
        2,
        "[\"*\"] must receive every event; got {logs:?}"
    );
    assert!(logs[0].contains("turn_end"), "got {logs:?}");
    assert!(logs[1].contains("tool_execution_end"), "got {logs:?}");
}

/// A named filter delivers the named type and withholds the rest. `turn_end` arrives in
/// both cases, so the second assertion is about what the child saw, not about whether the
/// child was alive.
#[tokio::test]
async fn a_named_event_filter_is_honored_at_the_host() {
    let dir = tempfile::tempdir().unwrap();
    let host = spawn_with_events(dir.path(), vec!["turn_end".into()]).await;

    deliver(&host, "tool_execution_end", 1).await;
    deliver(&host, "turn_end", 2).await;

    let logs = host.logs().await;
    assert_eq!(logs.len(), 1, "exactly one event was wanted; got {logs:?}");
    assert!(
        logs[0].contains("turn_end"),
        "the filtered-out type reached the child: {logs:?}"
    );
}

async fn spawn_with_events(dir: &std::path::Path, events: Vec<String>) -> std::sync::Arc<ExtHost> {
    let manifest = ExtManifest {
        name: "fixture".into(),
        version: "0.1.0".into(),
        bin: support::fixture_exe(),
        args: vec![],
        events,
        tools: vec![],
    };
    match tokio::time::timeout(MANIFEST_TIMEOUT, ExtHost::spawn(dir, &manifest)).await {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => panic!("spawn failed: {e}"),
        Err(_) => panic!("spawn did not return within {MANIFEST_TIMEOUT:?}"),
    }
}

/// Send one event and fold the child's receipt back through the host. The fixture turns
/// every event it sees into a `{"type":"log"}` frame, which the host only drains while it
/// is reading for a result — so the probe invoke is what makes the receipt observable,
/// and its reply text doubles as proof the child was still healthy.
async fn deliver(host: &ExtHost, event_type: &str, seq: u64) {
    host.send_event(&serde_json::json!({"type": event_type, "seq": seq}))
        .await;
    let (text, _) = awake(
        host.invoke(
            "ext_echo",
            &format!("tc_{event_type}_{seq}"),
            serde_json::json!({}),
        ),
        Duration::from_secs(5),
        "probe invoke that folds event receipts back through the host",
    )
    .await
    .expect("the fixture answers invokes in every mode");
    assert_eq!(text, "", "the probe carried no text");
}
