//! The terminal runtime's own tests: state, keys, imports and overlays.
//!
//! Lives beside `app.rs` rather than inside it so the runtime file reads as the
//! code it is. Rust still compiles these as part of the `app` module, so
//! `super::*` reaches every private item — a test forced through a public API
//! would be testing a different thing.

use super::*;
use crate::app::commands::{timeline_kind_tag, timeline_snip};
// The key handlers are one module down (they were moved out of `app.rs`),
// so the tests that drive them name the module rather than a re-export.
use super::keys::{begin_auth_key_prompt, handle_key};
// Frame helpers and the wired app are shared with `golden.rs`.
use super::tests_support::{render_rows, test_app, test_app_with_bash};
use crossterm::event::KeyEventState;
use epsilon_contracts::agent_event::{PermissionDecision, PermissionRequest};
use epsilon_contracts::message::UserMessage;
use epsilon_safety::{ApprovalUi, ChannelApprovalUi};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

/// Type a string one char at a time, then Enter — the real key path.
fn key_mod(modifiers: KeyModifiers, code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn type_and_enter(app: &mut App, agent: &Arc<Agent>, text: &str) -> bool {
    for c in text.chars() {
        handle_key(app, agent, key(KeyCode::Char(c)));
    }
    handle_key(app, agent, key(KeyCode::Enter))
}

#[tokio::test]
async fn search_command_now_searches_the_transcript_and_not_the_session_log() {
    // M157 replaced this test rather than deleting it, because the behaviour it pinned is
    // the one that was wrong. It seeded a real session log and asserted the hits came back as
    // transcript notices — which passed, and passed for a feature that could not move the
    // view, highlight anything, or work at all without a session attached.
    //
    // What is asserted now is the contract that actually matters: `/search` finds text in
    // **what is on screen**, and a miss and a blank query are two different answers.
    let (mut app, agent, (_f, _n)) = test_app();
    for text in [
        "first thing",
        "the launch codes are under the keyboard",
        "third thing",
    ] {
        app.ui.push_notice(text);
    }

    // Real key path: /search keyboard
    assert!(!type_and_enter(&mut app, &agent, "/search keyboard"));
    let search = app.search.as_ref().expect("a hit opens the search");
    assert_eq!(search.query, "keyboard");
    assert_eq!(
        search.matches,
        vec![1],
        "the seeded line, by transcript index"
    );
    assert!(
        app.search.is_some() && app.ui.lines()[1].text.contains("keyboard"),
        "the match must be a real line on screen, not a session-log entry: {:?}",
        app.ui
            .lines()
            .iter()
            .map(|l| l.text.clone())
            .collect::<Vec<_>>()
    );

    // Close first, and the reason is the assertion: while a search is open it owns the
    // keyboard (as the approval dialog does), so typing goes nowhere and the transcript
    // below shows a mangled "/search q..." swallowed by the overlay. A user would press Esc;
    // this test has to as well, and the fact that it *had* to is worth knowing.
    app.search_close();

    // Miss case: a different message from a blank query, and nothing left open.
    assert!(!type_and_enter(&mut app, &agent, "/search quantum-unicorn"));
    assert!(app.search.is_none(), "a miss must not leave a search open");
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("no match for 'quantum-unicorn'")),
        "expected the miss notice, got {texts:?}"
    );

    // Usage hint case
    assert!(!type_and_enter(&mut app, &agent, "/search"));
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t.contains("usage: /search")),
        "expected the usage hint, got {texts:?}"
    );
}

#[tokio::test]
async fn approval_preempts_palette_and_picker_without_changing_filter() {
    for palette in [true, false] {
        let (mut app, agent, _receivers) = test_app();
        app.palette_open = palette;
        app.picker_open = !palette;
        app.input = "models".into();
        app.cursor_chars = 6;
        app.picker_index = 1;
        let approvals = ChannelApprovalUi::new();
        let requester = approvals.clone();
        let response = tokio::spawn(async move {
            requester
                .ask(PermissionRequest {
                    request_id: "focus-test".into(),
                    capability: "read".into(),
                    pattern: "notes.txt".into(),
                    detail: serde_json::json!({}),
                })
                .await
        });
        app.pending_ask = approvals.next_ask().await;
        // M94: visibility is an identity fact now — paint one real frame so
        // `draw` records THIS ask as the one the dialog showed.
        let _ = render_rows(&mut app, 80, 24);
        assert!(!handle_key(
            &mut app,
            &agent,
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)
        ));
        assert!(
            app.pending_ask.is_none(),
            "visible approval did not receive deny"
        );
        assert!(matches!(response.await.unwrap(), PermissionDecision::Deny));
        assert_eq!(app.input, "models");
        assert_eq!(app.cursor_chars, 6);
        assert_eq!(app.picker_index, 1);
        assert_eq!(app.palette_open, palette);
        assert_eq!(app.picker_open, !palette);
    }
}

/// Park one ask on the shared queue and hand it to the app, exactly like the
/// run loop does; the response future stays live so a test can assert which
/// decision the parked ask received (or that it never received one).
async fn park_ask(
    app: &mut App,
    approvals: &ChannelApprovalUi,
    id: &str,
) -> tokio::task::JoinHandle<PermissionDecision> {
    let requester = approvals.clone();
    let id = id.to_string();
    let response = tokio::spawn(async move {
        requester
            .ask(PermissionRequest {
                request_id: id,
                capability: "bash".into(),
                pattern: "rm -rf build".into(),
                detail: serde_json::json!({ "command": "rm -rf build" }),
            })
            .await
    });
    app.pending_ask = approvals.next_ask().await;
    response
}

/// M94 review (safety): below 60×12 the fallback paints instead of the
/// dialog, but the approval keys used to route to the pending ask anyway —
/// 'y' authorized work the user could not see, 'a' made it permanent for
/// the session, and Esc silently denied it. They must all refuse now, say
/// why, and leave the ask pending.
#[tokio::test]
async fn approval_keys_do_not_answer_an_invisible_approval() {
    for code in [
        KeyCode::Char('y'),
        KeyCode::Char('a'),
        KeyCode::Char('n'),
        KeyCode::Esc,
    ] {
        let (mut app, agent, _receivers) = test_app();
        let approvals = ChannelApprovalUi::new();
        let response = park_ask(&mut app, &approvals, "invisible-test").await;
        // Undersized frame: the fallback renders, so the dialog did not.
        let _ = render_rows(&mut app, 40, 8);
        assert!(
            app.approval_painted_id.is_none(),
            "the fallback cannot paint the dialog"
        );
        let notices_before = app.ui.lines().len();

        assert!(!handle_key(&mut app, &agent, key(code)));
        assert!(
            app.pending_ask.is_some(),
            "{code:?} answered an approval that was never drawn"
        );
        assert!(
            app.ui
                .lines()
                .iter()
                .any(|l| l.text.contains("approval is not visible")),
            "{code:?} pushed no refusal notice: {:?}",
            app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
        );
        assert!(
            app.ui.lines().len() > notices_before,
            "{code:?}: notice, not silence"
        );

        // The ask was never answered: dropping it fails closed to Deny.
        drop(app.pending_ask.take());
        assert!(matches!(response.await.unwrap(), PermissionDecision::Deny));
    }
}

/// M94 review: at sizes below the 60×12 floor the fallback must still name a
/// pending approval and the keys that can act on it — a bare size banner
/// hid a live permission request.
#[tokio::test]
async fn an_undersized_terminal_names_a_pending_approval() {
    let (mut app, _agent, _receivers) = test_app();
    let approvals = ChannelApprovalUi::new();
    let _response = park_ask(&mut app, &approvals, "undersized-test").await;

    // Below the layout floor, where no pane can be drawn at all.
    let rows = render_rows(&mut app, 43, 10);
    let screen = rows.join("\n");
    assert!(screen.contains("too small"), "{screen}");
    assert!(
        screen.contains("approval pending: bash · rm -rf build"),
        "the fallback did not name the ask: {screen}"
    );
    assert!(
        screen.contains("widen the window"),
        "the fallback did not say how to proceed: {screen}"
    );
}

/// The refusal notice names the floor the layout actually enforces.
///
/// It used to say "enlarge the terminal (need 60×12)" while
/// `Regions::too_small` refuses below `MIN_WIDTH`×`MIN_HEIGHT` (44×10). A user
/// who obeyed the sentence could resize to 60×12, still see no dialog, and have
/// no way to tell whether the harness was broken or the number was wrong. The
/// message is formatted from the constants now, so the two cannot drift again.
#[tokio::test]
async fn the_approval_refusal_names_the_layouts_real_floor() {
    let (mut app, agent, _receivers) = test_app();
    let approvals = ChannelApprovalUi::new();
    let _response = park_ask(&mut app, &approvals, "floor-test").await;
    // No frame has painted the ask, which is the state the refusal is for.
    assert!(app.approval_painted_id.is_none());
    assert!(!handle_key(&mut app, &agent, key(KeyCode::Char('y'))));

    let notice = app
        .ui
        .lines()
        .iter()
        .map(|l| l.text.clone())
        .find(|t| t.contains("approval is not visible"))
        .unwrap_or_else(|| panic!("no refusal notice: {:?}", app.ui.lines()));
    let floor = format!(
        "{}×{}",
        crate::ui::layout::MIN_WIDTH,
        crate::ui::layout::MIN_HEIGHT
    );
    assert!(
        notice.contains(&floor),
        "the notice must name the floor the layout enforces ({floor}): {notice:?}"
    );
    assert!(
        !notice.contains("60×12"),
        "the notice still names a size the layout does not want: {notice:?}"
    );
    drop(app.pending_ask.take());
}

/// M94 review: an ask that arrives while `/timeline` is open must be visible.
/// The timeline widget clears its own centre, so a dialog painted before it
/// was wiped out while y/a/n/Esc still routed to it (Esc silently denied).
#[tokio::test]
async fn a_pending_approval_is_drawn_over_the_timeline() {
    let (mut app, agent, _receivers) = test_app();
    app.timeline_open = true;
    app.timeline_entries = vec![crate::app::TimelineRow {
        seq: 7,
        id: "e7".into(),
        kind: "user".into(),
        snippet: "TIMELINE-ROW-MARKER".into(),
        depth: 0,
        on_active: true,
        parent: None,
        label: None,
    }];
    let approvals = ChannelApprovalUi::new();
    let response = park_ask(&mut app, &approvals, "over-timeline").await;

    let rows = render_rows(&mut app, 80, 24);
    let screen = rows.join("\n");
    assert!(
        rows.iter().any(|r| r.contains("TIMELINE-ROW-MARKER")),
        "timeline not drawn: {screen}"
    );
    assert!(
        screen.contains("permission required"),
        "the timeline covered the approval dialog: {screen}"
    );
    assert!(
        rows.iter().any(|r| r.contains("cap") && r.contains("bash")),
        "the dialog body is buried: {screen}"
    );

    // Visible now, so the keys answer it (the precedence is unchanged).
    assert!(!handle_key(&mut app, &agent, key(KeyCode::Char('n'))));
    assert!(app.pending_ask.is_none(), "visible approval got no deny");
    assert!(matches!(response.await.unwrap(), PermissionDecision::Deny));
}

/// M94 re-review (safety): `draw` wrote a per-frame `approval_visible` flag, so
/// the credit survived the frame that set it. Answer ask A, seat ask B inside
/// the same <=33 ms frame window (the channel is FIFO; parallel tool calls park
/// several) and a key handled before the next draw answered B — a decision the
/// user never saw, and `a` would have made it permanent for the session. The
/// guard is identity-based now: `draw` records the ask it painted and any other
/// ask is refused until a frame paints it.
#[tokio::test]
async fn a_second_parked_ask_is_not_answerable_before_a_frame_paints_it() {
    let (mut app, agent, _receivers) = test_app();
    let approvals = ChannelApprovalUi::new();

    // Ask A: parked, painted by a real frame, answered through the key path.
    let response_a = park_ask(&mut app, &approvals, "ask-a").await;
    let _ = render_rows(&mut app, 80, 24);
    assert!(!handle_key(&mut app, &agent, key(KeyCode::Char('y'))));
    assert!(
        app.pending_ask.is_none(),
        "'y' must answer the painted ask A"
    );
    assert!(matches!(
        response_a.await.unwrap(),
        PermissionDecision::AllowOnce
    ));

    // Ask B parks in the frame window — nothing draws between A and B.
    let _response_b = park_ask(&mut app, &approvals, "ask-b").await;
    assert_eq!(
        app.pending_ask
            .as_ref()
            .map(|a| a.request.request_id.as_str()),
        Some("ask-b"),
        "the freshly seated ask is the pending one"
    );
    for code in [KeyCode::Char('y'), KeyCode::Char('a')] {
        let noticed = app.ui.lines().len();
        assert!(!handle_key(&mut app, &agent, key(code)));
        assert_eq!(
            app.pending_ask
                .as_ref()
                .map(|a| a.request.request_id.as_str()),
            Some("ask-b"),
            "{code:?} answered ask B before any frame painted it"
        );
        assert!(
            app.ui.lines().len() > noticed,
            "{code:?} refused silently — the user must be told the ask is not on screen yet"
        );
    }
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.text.contains("approval is not visible")),
        "expected the refusal notice, got {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );

    // The guard is a gate, not a lockout: the next frame paints B and the
    // very same key answers it.
    let _ = render_rows(&mut app, 80, 24);
    assert!(!handle_key(&mut app, &agent, key(KeyCode::Char('y'))));
    assert!(app.pending_ask.is_none(), "the painted ask B is answerable");
    assert!(matches!(
        _response_b.await.unwrap(),
        PermissionDecision::AllowOnce
    ));

    // And a never-painted ask fails closed: a blind 'a' (AlwaysSession) is
    // refused, and dropping the ask resolves to Deny.
    let response_c = park_ask(&mut app, &approvals, "ask-c").await;
    assert!(!handle_key(&mut app, &agent, key(KeyCode::Char('a'))));
    assert!(
        app.pending_ask.is_some(),
        "'a' made a never-painted ask permanent for the session"
    );
    drop(app.pending_ask.take());
    assert!(matches!(
        response_c.await.unwrap(),
        PermissionDecision::Deny
    ));
}

/// M94 re-review (coherence): the dialog is painted over the masked `/login`
/// key prompt, but the auth branches handled keys first — y/a/n/Esc (and
/// Enter, which saves to `auth.json`) went into a buffer the user could not
/// see while the user was answering the dialog. With an ask pending the
/// approval keys route first and the auth state is left exactly as it was, so
/// the login flow resumes untouched.
#[tokio::test]
async fn a_pending_approval_preempts_the_auth_key_prompt() {
    let (mut app, agent, _receivers) = test_app();
    begin_auth_key_prompt(&mut app, "openai".into());
    app.auth_key = "sk-typed-so-far".into();
    app.auth_key_cursor = app.auth_key.chars().count();
    let approvals = ChannelApprovalUi::new();
    let response = park_ask(&mut app, &approvals, "over-auth").await;
    let _ = render_rows(&mut app, 80, 24); // the dialog paints over the prompt

    let before = (
        app.auth_prompt_open,
        app.auth_provider.clone(),
        app.auth_key.clone(),
        app.auth_key_cursor,
    );
    assert!(!handle_key(&mut app, &agent, key(KeyCode::Char('y'))));
    assert!(
        app.pending_ask.is_none(),
        "'y' must answer the approval, not the auth buffer"
    );
    assert!(matches!(
        response.await.unwrap(),
        PermissionDecision::AllowOnce
    ));
    assert_eq!(
        app.auth_prompt_open, before.0,
        "the login prompt stays open"
    );
    assert_eq!(app.auth_provider, before.1, "its provider is untouched");
    assert_eq!(
        app.auth_key, before.2,
        "'y' must not reach the masked buffer"
    );
    assert_eq!(app.auth_key_cursor, before.3, "nor move its caret");
}

/// M75: `/session` says nothing about repairs when the session opened clean — the listing
/// must not imply damage that did not happen. (The repaired case is covered where the repair
/// happens: `epsilon-session`'s own tests.)
#[tokio::test]
async fn session_repair_lines_are_empty_for_a_clean_session() {
    let (_app, agent, (_f, _n)) = test_app();
    assert!(agent.repair_report().is_none());
    assert!(repair_lines(&agent).is_empty());
}

/// M74: a submitted prompt with an `@` reference carries the file's content, through the same
/// constructor the other frontends use. The user's own words are untouched.
#[tokio::test]
async fn submit_text_attaches_at_references() {
    let (mut app, _agent, (_f, _n)) = test_app();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), "the answer is 42").unwrap();
    app.workspace_root = dir.path().to_path_buf();

    let msg = app.submit_text("what does @notes.txt say?".into());
    let Message::User(u) = msg else {
        panic!("a user message")
    };
    let texts: Vec<String> = u
        .content
        .iter()
        .filter_map(|c| match c {
            UserContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts[0], "what does @notes.txt say?", "verbatim");
    assert!(texts.len() >= 2, "the attachment rides with it: {texts:?}");
    assert!(texts[1].contains("the answer is 42"), "{texts:?}");

    // A prompt with no references is exactly what it always was.
    let plain = app.submit_text("no references here".into());
    let Message::User(p) = plain else {
        panic!("a user message")
    };
    assert_eq!(p.content.len(), 1, "{:?}", p.content);
}

#[tokio::test]
async fn slash_models_opens_picker_via_real_key_path() {
    let (mut app, agent, (_f, _n)) = test_app();
    for c in "/models".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        if app.picker_open {
            panic!("picker opened early at char {c:?} — some arm intercepts typing");
        }
    }
    assert_eq!(app.input, "/models", "typing must land in the editor");
    let quit_now = handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert!(!quit_now);
    assert!(app.picker_open, "/models must open the picker");
    assert!(app.input.is_empty(), "editor becomes the filter box");

    assert!(!handle_key(&mut app, &agent, key(KeyCode::Esc)));
    assert!(!app.picker_open);
}

/// The `LoginUi` adapter is the only thing the OAuth worker thread can say through.
///
/// Found by a control, not by reading: making `TuiLoginUi::notify` a no-op left the whole
/// suite green, because nothing constructed the adapter. `OauthUiEvent` has tests; the type
/// that produces events and notices had none, and it compiles only under `--features real`,
/// so an ordinary `cargo test` never even builds it.
///
/// These two are the adapter's entire observable surface: a notice that arrives, and a
/// device-code line that reaches the user with all three values in it.
#[cfg(feature = "real")]
#[test]
fn the_login_adapter_carries_notices_and_the_device_code_to_the_transcript() {
    use epsilon_llm::oauth_login::{DevicePrompt, LoginUi};

    let (notices_tx, mut notices_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let (events_tx, _events_rx) = tokio::sync::mpsc::unbounded_channel::<OauthUiEvent>();
    let (code_tx, code_rx) = std::sync::mpsc::channel::<String>();
    let _keep_code_sender = code_tx;
    let mut ui = TuiLoginUi {
        notices: notices_tx,
        events: events_tx,
        code_rx,
        provider: "anthropic".to_string(),
    };

    ui.notify("opening your browser");
    ui.device_code(&DevicePrompt {
        verification_uri: "https://example.invalid/device".to_string(),
        user_code: "WXYZ-1234".to_string(),
        expires_in_seconds: 900,
    });

    assert_eq!(
        notices_rx
            .try_recv()
            .expect("notify must reach the transcript"),
        "opening your browser"
    );
    let device = notices_rx
        .try_recv()
        .expect("device_code must reach the transcript");
    assert!(
        device.contains("https://example.invalid/device"),
        "{device}"
    );
    assert!(device.contains("WXYZ-1234"), "{device}");
    assert!(device.contains("900"), "{device}");
}

#[tokio::test]
async fn oauth_need_code_opens_the_overlay_and_done_closes_it() {
    let (mut app, agent, (_f, _n)) = test_app();
    // start_oauth_login installs the sender; NeedCode only opens the overlay.
    let (tx, _rx) = std::sync::mpsc::channel::<String>();
    app.oauth_code_tx = Some(tx);
    app.apply_oauth_event(OauthUiEvent::NeedCode {
        provider: "anthropic".into(),
    });
    assert!(app.auth_prompt_open, "overlay opens for the paste");
    assert_eq!(app.auth_provider, "anthropic");
    assert!(
        app.oauth_code_tx.is_some(),
        "the waiting login still owns it"
    );
    // Typing routes into the masked buffer, never the transcript editor.
    for c in "http://localhost:53692/callback?code=abc".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(app.input.is_empty(), "editor stays out of it");
    assert!(app.auth_key.contains("code=abc"));
    // Completion closes the overlay however the flow ended.
    app.apply_oauth_event(OauthUiEvent::Done("done".into()));
    assert!(!app.auth_prompt_open);
    assert!(app.oauth_code_tx.is_none());
}

#[tokio::test]
async fn oauth_enter_hands_the_code_to_the_waiting_login_and_esc_cancels_it() {
    let (mut app, agent, (_f, _n)) = test_app();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    app.oauth_code_tx = Some(tx);
    app.apply_oauth_event(OauthUiEvent::NeedCode {
        provider: "anthropic".into(),
    });
    for c in "code-123#state-1".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert_eq!(
        rx.try_recv().expect("code reached the flow"),
        "code-123#state-1"
    );
    assert!(!app.auth_prompt_open);
    assert!(app.oauth_code_tx.is_none());

    // Esc drops the sender, which is how the worker sees a cancellation.
    let (tx2, rx2) = std::sync::mpsc::channel::<String>();
    app.oauth_code_tx = Some(tx2);
    app.auth_prompt_open = true;
    app.auth_provider = "anthropic".into();
    handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert!(app.oauth_code_tx.is_none());
    assert!(matches!(
        rx2.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Disconnected)
    ));
}

#[tokio::test]
async fn ctrl_o_in_the_key_prompt_switches_to_the_oauth_ceremony() {
    let (mut app, agent, (_f, _n)) = test_app();
    begin_auth_key_prompt(&mut app, "anthropic".into());
    assert!(app.auth_prompt_open);
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts.join("\n").contains("Ctrl+O"),
        "the prompt advertises the subscription route"
    );
    handle_key(
        &mut app,
        &agent,
        key_mod(KeyModifiers::CONTROL, KeyCode::Char('o')),
    );
    assert!(!app.auth_prompt_open, "the key prompt gives way");
    // A provider without a ceremony gets guidance, not a silent no-op.
    begin_auth_key_prompt(&mut app, "deepseek".into());
    handle_key(
        &mut app,
        &agent,
        key_mod(KeyModifiers::CONTROL, KeyCode::Char('o')),
    );
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts
            .join("\n")
            .contains("no OAuth login for that provider")
    );
}

/// Write a real session file (header + one user message) for browser tests.
fn write_session_fixture(path: &std::path::Path, text: &str) {
    let mut store =
        epsilon_session::SessionStore::open_in(path, &std::env::temp_dir()).expect("session store");
    let user = epsilon_contracts::message::UserMessage {
        id: "m1".to_string(),
        timestamp_ms: 0,
        content: vec![epsilon_contracts::content::UserContent::Text {
            text: text.to_string(),
        }],
    };
    store
        .append(epsilon_contracts::session_entry::EntryKind::Message {
            message: epsilon_contracts::message::Message::User(user),
            api_request_sha256: None,
        })
        .expect("append");
}

#[test]
fn editor_command_prefers_config_then_visual_then_editor() {
    // pi `getExternalEditorCommand` order, with blanks treated as unset.
    assert_eq!(
        editor_command_from(Some("code --wait"), Some("vim"), Some("nano")).unwrap(),
        "code --wait"
    );
    assert_eq!(
        editor_command_from(None, Some("vim"), Some("nano")).unwrap(),
        "vim"
    );
    assert_eq!(
        editor_command_from(None, None, Some("nano")).unwrap(),
        "nano"
    );
    assert_eq!(
        editor_command_from(Some("  "), None, Some("  nano  ")).unwrap(),
        "nano"
    );
    let err = editor_command_from(None, None, None).expect_err("unconfigured");
    assert!(err.contains("$EDITOR"), "{err}");
    assert!(
        err.contains("[editor]"),
        "the error shows the config shape: {err}"
    );
}

#[test]
fn external_editor_round_trip_writes_hands_over_and_reads_back() {
    // The spawner seam stands in for the user's editor: it appends a line
    // (and a trailing newline the round trip must trim).
    let edited = edit_with("draft line\n", &|path: &std::path::Path| {
        let mut text = std::fs::read_to_string(path).expect("draft written");
        text.push_str("second line\n");
        std::fs::write(path, text).expect("edit");
        Ok(0)
    })
    .expect("round trip");
    assert_eq!(edited, "draft line\nsecond line");
}

#[test]
fn external_editor_keeps_the_draft_when_the_editor_fails() {
    let err =
        edit_with("important draft", &|_p: &std::path::Path| Ok(3)).expect_err("non-zero exit");
    assert!(err.contains("status 3"), "{err}");
    assert!(err.contains("draft kept"), "{err}");
}

#[test]
fn editor_request_refuses_during_a_run_and_reports_a_missing_editor() {
    let (mut app, _agent, (_f, _n)) = test_app();
    app.request_external_editor();
    // No editor configured in the test environment unless the suite host has
    // one — either way the request never crashes and either arms the flag or
    // explains what to configure.
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    let joined = texts.join("\n");
    if app.external_editor_requested {
        assert!(joined.contains("opening"), "{joined}");
        app.external_editor_requested = false;
    } else {
        assert!(joined.contains("editor:"), "{joined}");
    }
}

#[test]
fn editor_keybinding_and_slash_arm_the_same_request() {
    let (mut app, agent, (_f, _n)) = test_app();
    // Alt+E (and Ctrl+O) reach the same request path as `/editor`.
    app.keybindings = crate::keybindings::KeybindingsManager::create();
    let alt_e = KeyEvent::new(KeyCode::Char('e'), KeyModifiers::ALT);
    assert!(
        app.keybindings.matches("app.editor.external", &alt_e),
        "alt+e is a default for app.editor.external"
    );
    app.kill_ring = crate::kill_ring::KillRing::new();
    let _ = handle_key(&mut app, &agent, alt_e);
    let opened_by_key = app.external_editor_requested;
    app.external_editor_requested = false;
    type_and_enter(&mut app, &agent, "/editor");
    assert_eq!(
        app.external_editor_requested, opened_by_key,
        "the slash command and the key take the same path"
    );
}

#[tokio::test]
async fn sessions_command_browses_and_enter_switches_the_session() {
    // M57: the browser lists the workspace's session directory, filters with
    // the input box, and Enter runs the same switch `/resume <path>` does.
    let (mut app, agent, (_f, _n)) = test_app();
    let tmp = tempfile::tempdir().unwrap();
    let sessions = tmp.path().join(".epsilon").join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let target = sessions.join("other.jsonl");
    write_session_fixture(&target, "the other session's first request");
    write_session_fixture(&sessions.join("zzz-unrelated.jsonl"), "unrelated work");
    app.workspace_root = tmp.path().to_path_buf();

    // Typing must land in the editor until Enter runs the command.
    for c in "/sessions".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(app.session_browser.is_none(), "opens only on Enter");
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    let browser = app.session_browser.as_ref().expect("browser open");
    assert_eq!(browser.rows.len(), 2, "both session files listed");
    assert!(app.input.is_empty(), "editor becomes the filter box");

    // Esc closes without switching.
    handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert!(app.session_browser.is_none());

    // Reopen (Esc cleared the editor, so retype), filter to one row, Enter.
    type_and_enter(&mut app, &agent, "/sessions");
    assert!(app.session_browser.is_some());
    for c in "other".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    let rows = app.session_browser.as_ref().unwrap().rows.clone();
    assert_eq!(rows.len(), 1, "filter narrows to the fixture: {rows:?}");
    assert_eq!(rows[0].path, target);
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert!(app.session_browser.is_none(), "closes on pick");
    let attached = agent
        .store_handle_for_tests()
        .lock()
        .unwrap()
        .path()
        .to_path_buf();
    assert_eq!(attached, target, "the app switched onto the picked file");
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(texts.join("\n").contains("resumed 1 messages"), "{texts:?}");
}

#[tokio::test]
async fn sessions_command_says_when_there_is_nothing_to_browse() {
    let (mut app, agent, (_f, _n)) = test_app();
    let tmp = tempfile::tempdir().unwrap();
    app.workspace_root = tmp.path().to_path_buf();
    type_and_enter(&mut app, &agent, "/sessions");
    assert!(app.session_browser.is_none(), "no directory, no browser");
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts.join("\n").contains("no session directory"),
        "{texts:?}"
    );

    // An empty directory (no session files yet) is explained too.
    std::fs::create_dir_all(tmp.path().join(".epsilon").join("sessions")).unwrap();
    type_and_enter(&mut app, &agent, "/sessions");
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(texts.join("\n").contains("nothing to resume"), "{texts:?}");
}

#[tokio::test]
async fn session_command_reports_cache_attribution_beside_the_totals() {
    // M69: `/session` already prints the token totals; the cache lines say whether the prompt
    // cache was paying off, and why not when it was not. Before any provider call there is
    // nothing to attribute, and the notice says exactly that instead of inventing a rate.
    let (mut app, agent, (_f, _n)) = test_app();
    type_and_enter(&mut app, &agent, "/session");
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    let joined = texts.join("\n");
    assert!(joined.contains("session:"), "{texts:?}");
    assert!(
        joined.contains("no durable session file attached"),
        "an agent without a store is described, not crashed on: {texts:?}"
    );
    assert!(
        joined.contains("cache: no provider call yet"),
        "an untouched session has no cache history: {texts:?}"
    );
}

#[tokio::test]
async fn session_command_shows_where_the_money_went_not_just_how_much() {
    // M143. `CostBreakdown`'s four per-component accessors had **no callers anywhere** —
    // tested, documented, and unreachable, which is the worst kind of dead code: it reads
    // like a feature. pi's `SessionStats` carries a `cost`; this is the same idea, made
    // legible, because a single `$0.31` cannot tell you whether the cache is working.
    let (mut app, agent, (_f, _n)) = test_app();
    type_and_enter(&mut app, &agent, "/session");
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    let joined = texts.join("\n");

    // Before any provider call there is nothing billed, so the line says so rather than
    // printing `$0.00` — which would be a precise-looking number about nothing.
    assert!(
        joined.contains("cost: nothing billed yet"),
        "an untouched session must not claim a cost: {texts:?}"
    );
}

#[tokio::test]
async fn tasks_command_lists_the_registry_and_says_when_it_is_empty() {
    let (mut app, agent, (_f, _n)) = test_app();
    type_and_enter(&mut app, &agent, "/tasks");
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts.join("\n").contains("no subagent tasks"),
        "empty registry is explained, not silent"
    );

    // Start a background child through the loop's host: the registry view
    // must show it as running with its description.
    let host: std::sync::Arc<dyn epsilon_contracts::tool::TaskHost> =
        agent.clone() as std::sync::Arc<dyn epsilon_contracts::tool::TaskHost>;
    host.run_task(epsilon_contracts::tool::TaskRequest {
        prompt: "look around".into(),
        description: "repo recon".into(),
        subagent_type: "default".into(),
        background: true,
        resume: None,
        // M56: the view must also name the model the child runs on.
        model: Some("deepseek-flash".into()),
    })
    .await
    .expect("background start");
    type_and_enter(&mut app, &agent, "/tasks");
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    let joined = texts.join("\n");
    assert!(joined.contains("repo recon"), "{joined}");
    assert!(
        ["running", "completed", "failed"]
            .iter()
            .any(|state| joined.contains(state)),
        "the state is shown: {joined}"
    );
    assert!(joined.contains("task(s)"), "{joined}");
    assert!(
        joined.contains("@deepseek-flash"),
        "the child's model is part of the view: {joined}"
    );
}

#[tokio::test]
async fn login_oauth_never_falls_through_to_the_key_store() {
    let (mut app, agent, (_f, _n)) = test_app();
    type_and_enter(&mut app, &agent, "/login xai oauth");
    assert!(
        !app.auth_prompt_open,
        "`oauth` must not be read as a pasted key"
    );
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    let joined = texts.join("\n");
    // The guidance depends on how this crate was compiled: a build without
    // the `real` feature says so honestly, a real build either starts the
    // ceremony or reports the failure. Assert the invariant that matters —
    // the word `oauth` was never mistaken for a pasted API key — and let the
    // message differ per build (feature unification with epsilon-cli means
    // `real` may well be on during a workspace test run).
    assert!(
        joined.contains("OAuth login") || joined.contains("oauth login"),
        "the user is told what happened: {joined}"
    );
    assert!(
        !epsilon_llm::auth::load_credentials().contains_key("xai"),
        "nothing is stored for a ceremony that could not run"
    );
}

#[tokio::test]
async fn slash_model_direct_applies_instantly() {
    let (mut app, agent, (_f, _n)) = test_app();
    let before = app.ui.lines().len();
    assert!(!type_and_enter(&mut app, &agent, "/model deepseek-chat"));
    assert_eq!(
        app.model_id, "deepseek-chat",
        "direct /model applies instantly"
    );
    let line = &app.ui.lines()[before];
    assert!(line.text.contains("deepseek-chat"), "{:?}", line.text);
    // Recents refresh in-memory through apply_model (the persisted side is
    // covered by config-layer round-trip tests; asserting the real global
    // file here races every other TUI test's writes — lesson 16b again).
    assert!(
        app.recents.first().map(String::as_str) == Some("deepseek-chat"),
        "in-memory recents updated with the applied model"
    );
}

#[tokio::test]
async fn unknown_slash_word_errors_instead_of_prompting_the_model() {
    let (mut app, agent, (_f, _n)) = test_app();
    let before = app.ui.lines().len();
    assert!(!type_and_enter(&mut app, &agent, "/bogus"));
    assert!(!app.ui.running(), "must NOT start a run");
    assert_eq!(app.ui.lines().len(), before + 1, "one system notice");
    assert!(app.ui.lines()[before].text.contains("/bogus"));
}

/// G1: the binding `/hotkeys` prints must actually do something.
///
/// `app.clear` was in the defaults table, `/hotkeys` printed it as
/// "Clear editor", and **nothing read it** — while `app.exit` matched the same
/// chord first. So Ctrl+C quit the process with the draft still in the buffer, and
/// the one place a user goes to learn the keys was advertising a binding that did
/// nothing. This is written through `handle_key` rather than by calling
/// `clear_editor` directly, because a direct call cannot see whether anything
/// calls it — the exact defect class this test exists to catch.
/// G9: every palette entry must name a command the parser can actually run.
///
/// The "Toggle tool output expand" entry carried a `\u{1}expand` pseudo-command.
/// `parse_command` requires a leading `/`, so it returned `None` — the palette
/// closed, the composer cleared, nothing ran, and no notice explained why. A
/// menu entry that silently does nothing is worse than an absent one, because it
/// tells the user the feature exists.
///
/// Asserted over **every** entry rather than this one: the defect is a class, and
/// the next entry added with a made-up command would fail the same way.
/// G10: a key the TUI does not act on must say so.
///
/// The invariant is the one this whole investigation turned up: **"nothing
/// happened" must never be a state the UI can be in.** A silently swallowed
/// keystroke is indistinguishable from a frozen program — the exact shape of the
/// report this project received, and the reason an unexplained freeze could not be
/// told apart from a key that was simply not handled.
///
/// The notice travels on the app's own channel, so this asserts on what the user
/// would actually see rather than on an internal flag.
/// G2: `/fork` in the TUI must produce a real fork, not a copy of the file.
///
/// The TUI's fork was `std::fs::copy` — the whole file, every abandoned branch
/// included, under the *same* session id in the header, so the two files claimed
/// one identity. The correct primitive (`SessionStore::fork_branch_at`, which
/// writes a fresh header and copies only the active branch) already existed and
/// was wired **only** from the RPC surface, so the transport a human uses had the
/// worse implementation.
///
/// Asserted on the bytes the command produced, because "it called a different
/// function" is not the property that matters — a distinct session is.
#[tokio::test]
async fn fork_creates_a_distinct_session_and_not_a_file_copy() {
    let (mut app, agent, _rx) = test_app();
    let dir = seed_branch(&agent, &["first", "second"]);

    app.run_command(
        &agent,
        crate::picker::parse_command("/fork").expect("/fork parses"),
    );

    let source = dir.path().join("session.jsonl");
    let siblings: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read_dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();

    let forked: Vec<_> = siblings.iter().filter(|p| **p != source).cloned().collect();
    assert_eq!(
        forked.len(),
        1,
        "exactly one new session file must appear; found {forked:?}"
    );

    // A different session id in the header — this is what a file copy cannot give,
    // since it duplicates the header verbatim.
    let source_id = header_id(&std::fs::read_to_string(&source).expect("read source"));
    let fork_id = header_id(&std::fs::read_to_string(&forked[0]).expect("read fork"));
    assert_ne!(
        source_id, fork_id,
        "a fork is a new session: it must carry its own id, not the source's"
    );
}

/// Read the `id` field out of a session file's header line.
fn header_id(first_line: &str) -> String {
    first_line
        .lines()
        .next()
        .and_then(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .and_then(|v| v.get("id").and_then(|i| i.as_str()).map(str::to_string))
        .unwrap_or_default()
}

#[tokio::test]
async fn an_unbound_key_produces_a_visible_response() {
    let (mut app, agent, (_ftx_rx, mut notice_rx)) = test_app();

    // F1, not ctrl+]: a `Char` with CONTROL reaches `insert_char_tracked` — the
    // `Char` arm comes before the catch-all — so it is a *handled* key and proves
    // nothing about the fallback. F1 is in no default binding, and is not a `Char`
    // at all, so the only arm it can reach is the catch-all.
    let quit = handle_key(&mut app, &agent, key(KeyCode::F(1)));

    assert!(!quit, "an unbound key must not quit the app");
    let said = notice_rx.try_recv().expect(
        "a key the TUI does not act on must produce feedback — a silent no-op is \
         indistinguishable from a frozen program",
    );
    assert!(
        said.contains("not bound"),
        "the notice must say the key is unbound, not merely that something happened; got: {said:?}"
    );
    assert!(
        said.contains("/hotkeys"),
        "the notice should point at the place that lists what IS bound; got: {said:?}"
    );
}

/// A key that IS bound must not be reported as unbound.
///
/// The other direction of the same property: feedback that fires on working keys
/// is its own lie, training the user to ignore the notice.
#[tokio::test]
async fn a_bound_key_does_not_produce_a_not_bound_notice() {
    let (mut app, agent, (_ftx_rx, mut notice_rx)) = test_app();
    // ctrl+x is `app.tools.expand` — a real binding that reports its own outcome.
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('x')));
    let notices: Vec<String> = std::iter::from_fn(|| notice_rx.try_recv().ok()).collect();
    assert!(
        !notices.iter().any(|n| n.contains("not bound")),
        "a bound key must not be reported as unbound; got: {notices:?}"
    );
}

#[tokio::test]
async fn every_palette_entry_names_a_command_that_parses() {
    for entry in crate::picker::palette_entries() {
        assert!(
            crate::picker::parse_command(&entry.command).is_some(),
            "palette entry {:?} carries {:?}, which the parser cannot read — \
             choosing it would close the palette and do nothing",
            entry.title,
            entry.command
        );
    }
}

/// An unrouted `Command` must be *visible*, not merely detected.
///
/// `run_workspace_command` is the dispatcher's catch-all, so a new `Command`
/// variant that nobody gave an arm lands in its `other` arm. That arm is
/// unavoidable — the executor receives every variant, including the 29 its six
/// siblings own by name, so removing it would mean re-enumerating all of them
/// here. What must not be avoidable is the arm's *silence*.
///
/// It used to be silent in exactly the build that matters: `#[allow]` silenced
/// the warning, and `debug_assert!` fires only in a debug build, so a release
/// binary with an unrouted variant returned `false` and printed nothing. The
/// user typed `/thing` and the screen did not change — indistinguishable from a
/// hang, which `AGENTS.md`'s TUI invariant calls the one thing the UI must never
/// be.
///
/// Asserted on the source, because the branch is statically unreachable in a
/// correct build and a runtime test could not reach it to observe anything.
#[test]
fn the_catch_alls_other_arm_tells_the_user_instead_of_failing_silently() {
    let source = include_str!("commands.rs");
    let body = source
        .split_once("fn run_workspace_command(")
        .expect("run_workspace_command is in this file")
        .1;
    let body = body.split_once("\n    fn ").map_or(body, |(head, _)| head);
    let arm = body
        .split_once("other => {")
        .map_or("", |(_, tail)| tail)
        .split_once("}\n")
        .map_or("", |(head, _)| head);
    assert!(
        !arm.is_empty(),
        "run_workspace_command has no `other` arm. That is expected only if it \
         became exhaustive over the whole enum — check that it was not made \
         exhaustive by re-listing the variants the sibling executors own."
    );
    assert!(
        arm.contains("push_notice"),
        "the catch-all's `other` arm fails silently again. A Command variant \
         with no arm of its own must say so in the transcript, or a user who \
         typed it sees an unchanged screen. Found: {arm:?}"
    );
}

/// The advertised key hint must be the chord that is actually bound.
///
/// The same entry advertised a bare `x`, which is the binding the editor
/// deliberately removed because it made the letter untypable. A hint is only
/// useful if pressing it does what the hint says.
#[tokio::test]
async fn a_palette_entry_that_advertises_a_key_advertises_the_bound_one() {
    for entry in crate::picker::palette_entries() {
        if let Some(key) = entry.binding {
            let normalized = key.trim_start_matches("Ctrl+").to_lowercase();
            assert!(
                normalized != "x",
                "entry {:?} advertises a bare `x`; the real chord is ctrl+x and a \
                 bare x is the binding that was removed to keep the letter typable",
                entry.title
            );
        }
    }
}

#[tokio::test]
async fn ctrl_c_clears_the_draft_instead_of_quitting() {
    let (mut app, agent, _rx) = test_app();
    for c in "a draft worth keeping".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(
        !app.input.is_empty(),
        "the draft is in the composer before the chord is pressed"
    );

    let quit = handle_key(&mut app, &agent, ctrl(KeyCode::Char('c')));

    assert!(
        !quit,
        "ctrl+c is 'Clear editor' — it must not terminate the process with a \
         draft still in the composer"
    );
    assert!(
        app.input.is_empty(),
        "the advertised behaviour is that ctrl+c empties the composer"
    );
    assert_eq!(
        app.cursor_chars, 0,
        "the cursor moves with the text, or the next character lands somewhere \
         the user did not type"
    );

    // And the way out still works, so "clear does not quit" is not "quit is gone".
    assert!(
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('q'))),
        "ctrl+q is the documented exit and must still quit"
    );
}

/// The advertised table and the handlers must not disagree about a chord.
///
/// Two ids sharing one chord is not a remap — it is a coin toss decided by
/// handler order, and the user cannot see which way it lands. Asserted over the
/// *effective* table so a user's own `keybindings.json` is covered too.
#[tokio::test]
async fn no_chord_is_bound_to_two_ids_by_default() {
    let (app, _agent, _rx) = test_app();
    let mut seen: std::collections::BTreeMap<String, String> = Default::default();
    let mut clashes: Vec<String> = Vec::new();
    for (id, keys) in app.keybindings.effective().into_iter() {
        for k in keys {
            match seen.get(&k) {
                Some(other) => clashes.push(format!("{k}: {other} and {id}")),
                None => {
                    seen.insert(k.clone(), id.clone());
                }
            }
        }
    }
    assert!(
        clashes.is_empty(),
        "each chord must name one action; found: {}",
        clashes.join("; ")
    );
}

#[tokio::test]
async fn ctrl_m_opens_picker_without_typing_slash() {
    let (mut app, agent, (_f, _n)) = test_app();
    assert!(!handle_key(&mut app, &agent, ctrl(KeyCode::Char('m'))));
    assert!(app.picker_open);
}

#[test]
fn remapped_quit_via_keybindings_manager() {
    // G6: app.exit remapped to ctrl+q via manager still quits; ctrl+c no longer does.
    let (mut app, agent, _rx) = test_app();
    let mut map = std::collections::HashMap::new();
    map.insert("app.exit".into(), vec!["ctrl+q".into()]);
    app.keybindings = crate::keybindings::KeybindingsManager::from_map(map);
    assert!(handle_key(
        &mut app,
        &agent,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)
    ));
    let (mut app2, agent2, _rx2) = test_app();
    let mut map2 = std::collections::HashMap::new();
    map2.insert("app.exit".into(), vec!["ctrl+q".into()]);
    app2.keybindings = crate::keybindings::KeybindingsManager::from_map(map2);
    // ctrl+c is NOT exit when remapped to only ctrl+q (verify the negative).
    assert!(!handle_key(
        &mut app2,
        &agent2,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
    ));
}

/// The two halves of the per-card convention must answer the same question.
///
/// `RenderState::is_open` decides what is **drawn**; `App::card_is_open` decides
/// what a keypress **records**. The whole per-card feature lives in their
/// agreement — press `ctrl+x` twice and the card the first press opened has to
/// still be open after the second, which only holds while both read one rule.
/// They were two copies of that rule, identical on the day they were written and
/// free to drift apart afterwards, so this walks every input the rule depends on
/// and asserts the two answers are the same answer.
#[tokio::test]
async fn the_drawn_and_recorded_answers_to_an_open_card_are_the_same_answer() {
    let (mut app, _agent, (_f, _n)) = test_app();
    // Bodies, so `has_body` is exercised both ways: this is the input a card's
    // openness is allowed to consult first.
    app.ui.apply(&crate::state::agent_event_edit_result(
        "@@ -1,1 +1,1 @@\n-old one\n+new one\n",
        false,
    ));
    app.ui.push_notice("and a row that is not a card at all");
    let ids: Vec<u64> = app.ui.lines().iter().map(|l| l.id).collect();
    assert!(
        ids.len() >= 2,
        "the fixture must hold more than the card: {ids:?}"
    );
    let bodyness: Vec<(u64, bool)> = ids
        .iter()
        .map(|id| {
            (
                *id,
                app.ui
                    .lines()
                    .iter()
                    .find(|l| l.id == *id)
                    .and_then(|l| l.card.as_ref())
                    .is_some_and(|c| c.has_body()),
            )
        })
        .collect();
    assert!(
        bodyness.iter().any(|(_, has)| *has) && bodyness.iter().any(|(_, has)| !*has),
        "the fixture must hold a card with a body and a row without one: {bodyness:?}"
    );

    let card_open: Vec<(u64, bool)> = vec![(ids[0], true), (ids[1], false)];
    let mut checked = 0usize;
    for expand_all in [false, true] {
        for entries in [Vec::new(), card_open.clone(), vec![(ids[0], false)]] {
            app.prefs.expand_outputs = expand_all;
            app.card_open = entries.clone();
            // The exact construction the draw path uses, minus the key text —
            // which no branch of the rule reads.
            let drawn = crate::render_cache::RenderState {
                expand_all: app.prefs.expand_outputs,
                expanded: &app.card_open,
                more_key: None,
            };
            for (id, has_body) in &bodyness {
                assert_eq!(
                    app.card_is_open(*id, *has_body),
                    drawn.is_open(*id, *has_body),
                    "expand_all={expand_all} entries={entries:?} card={id} \
                     has_body={has_body}: a keypress and a frame disagree about the same card"
                );
                checked += 1;
            }
        }
    }
    assert!(
        checked >= 12,
        "the sweep stopped covering: {checked} checks"
    );
}

/// The concatenated source of the `app` module and its children.
///
/// The module is being split up, and a test that greps the source must not care which file a
/// line ended up in. This list is hand-maintained and *will* fall behind the next split;
/// the rule that keeps it honest is M166's — a new file under `app/` is added here in the
/// same change that creates it.
/// and a test that greps the source must not care which file a line ended up in.
#[cfg(test)]
const APP_MODULE_SOURCE: &str = concat!(
    include_str!("../app.rs"),
    include_str!("../app/commands.rs"),
    include_str!("../app/panels.rs"),
    include_str!("../app/view.rs"),
    include_str!("../app/session.rs"),
    include_str!("../app/editing.rs"),
    include_str!("../app/completion.rs"),
    include_str!("../app/editor.rs"),
    include_str!("../app/login.rs"),
);

/// The rule is stated in one place, and this is what keeps it that way.
///
/// The agreement test below passes for a second copy of the rule too — which is
/// exactly why the copy was invisible for a whole slice: two implementations that
/// agree on the day they are written cannot be told apart by running them. The
/// thing that can be checked is the thing that was wrong: whether the *rule* is
/// stated here or borrowed. So this reads the function and asks it to delegate,
/// and it fails on the day someone writes the expression out again.
#[test]
fn the_card_open_rule_is_borrowed_and_not_restated() {
    // Every file the `app` module is split across, not just `app.rs`. A source-grep
    // test that names one file is a test that breaks the first time the code moves,
    // and it breaks with a message about the *rule* rather than about the move.
    let source = APP_MODULE_SOURCE;
    const GONE: &str = "`card_is_open` is gone; the draw path and the key path now decide this \
                        independently and nothing in the tree says the two agree";
    let after = source
        .split_once("fn card_is_open(&self, id: u64, has_body: bool) -> bool {")
        .unwrap_or_else(|| panic!("{GONE}"))
        .1;
    // Up to the end of the item: the first line that closes it at column zero.
    let body: String = after
        .lines()
        .take_while(|line| *line != "    }" && *line != "}")
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        body.contains(".is_open(id, has_body)"),
        "`card_is_open` must call the renderer's rule, not state its own: {body}"
    );
    // The delegation unavoidably *names* the two inputs — that is what building a
    // `RenderState` is — so this does not forbid them. It forbids the shape of a
    // second implementation: a lookup that decides for itself, which is the
    // `find(..).map(..).unwrap_or(..)` chain both copies used to be.
    assert!(
        !body.contains("unwrap_or"),
        "the rule's own default has been restated here instead of borrowed: {body}"
    );
}

#[tokio::test]
async fn x_toggles_expand_and_quit_requests_exit() {
    let (mut app, agent, (_f, _n)) = test_app();
    // M29: bare `x` is a LETTER now; only Ctrl+X toggles output expansion.
    assert!(!handle_key(&mut app, &agent, key(KeyCode::Char('x'))));
    assert!(!app.prefs.expand_outputs, "plain x types, not toggles");
    assert!(app.input.ends_with('x'), "x lands in the editor");
    app.backspace();

    assert!(!handle_key(&mut app, &agent, ctrl(KeyCode::Char('x'))));
    assert!(app.prefs.expand_outputs);

    assert!(!type_and_enter(&mut app, &agent, "/clear"));
    let last = app.ui.lines().last().unwrap();
    assert!(last.text.contains("cleared"), "{:?}", last.text);

    // /quit must request exit through the same Enter path users press.
    assert!(type_and_enter(&mut app, &agent, "/quit"), "/quit exits");
}

#[tokio::test]
async fn ctrl_w_then_ctrl_y_round_trips_text_through_the_kill_ring() {
    let (mut app, agent, (_f, _n)) = test_app();
    for c in "hello world".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    // Ctrl+W kills "world".
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('w')));
    assert_eq!(app.input, "hello ");
    // Ctrl+Y yanks it back.
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('y')));
    assert_eq!(app.input, "hello world", "yank restores the kill");
}

#[tokio::test]
async fn ctrl_j_inserts_newline_and_enter_still_submits() {
    let (mut app, agent, (_f, _n)) = test_app();
    for c in "line one".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('j')));
    for c in "line two".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert_eq!(app.input, "line one\nline two", "Ctrl+J is a newline");

    // Enter submits the WHOLE multi-line buffer (and persists history).
    let before = app.ui.lines().len();
    assert!(!handle_key(&mut app, &agent, key(KeyCode::Enter)));
    assert_eq!(app.ui.lines().len(), before + 1);
    assert_eq!(
        app.history.back().map(String::as_str),
        Some("line one\nline two")
    );
}

#[tokio::test]
async fn consecutive_ctrl_w_accumulates_into_one_yank_entry() {
    let (mut app, agent, (_f, _n)) = test_app();
    for c in "alpha beta gamma".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('w'))); // kills "gamma"
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('w'))); // kills "beta "
    assert_eq!(app.input, "alpha ");
    // One yank restores BOTH words (accumulated entry).
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('y')));
    assert_eq!(
        app.input,
        "alpha beta gamma",
        "{:?} ring={}",
        app.input,
        app.kill_ring.len()
    );
}

#[tokio::test]
async fn paste_marker_expands_to_payload_on_submit() {
    let (mut app, agent, (_f, _n)) = test_app();
    // Simulate a bracketed paste of 3 lines.
    let pasted = "l1\nl2\nl3";
    app.paste_payloads.insert(7, pasted.to_string());
    app.next_paste_id = 8;
    for c in "summarize [paste #7 +3 lines]".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    let before = app.ui.lines().len();
    assert!(!handle_key(&mut app, &agent, key(KeyCode::Enter)));
    // The transcript echo must show the EXPANDED payload.
    let echoed = app.ui.lines()[before].text.clone();
    assert!(echoed.contains("l1\nl2\nl3"), "{echoed:?}");
}

#[tokio::test]
async fn ctrl_p_opens_palette_runs_command_and_restores_draft() {
    let (mut app, agent, (_f, _n)) = test_app();
    for c in "draft in progress".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    // Ctrl+P opens the palette and stashes the draft.
    assert!(!handle_key(&mut app, &agent, ctrl(KeyCode::Char('p'))));
    assert!(app.palette_open);
    assert_eq!(app.input, "", "editor becomes the filter box");
    assert_eq!(
        app.stash.as_ref().map(|(t, _)| t.as_str()),
        Some("draft in progress")
    );

    // Enter runs the first entry (/models) → opens the picker instead.
    assert!(!handle_key(&mut app, &agent, key(KeyCode::Enter)));
    assert!(!app.palette_open);
    assert!(app.picker_open);

    // Esc closes the picker AND restores the stashed draft.
    assert!(!handle_key(&mut app, &agent, key(KeyCode::Esc)));
    assert!(!app.picker_open);
    assert_eq!(app.input, "draft in progress");
}

#[tokio::test]
async fn ctrl_z_undoes_backspace_and_kills() {
    let (mut app, agent, (_f, _n)) = test_app();
    for c in "undo me".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    // Backspace deletes "e".
    handle_key(&mut app, &agent, key(KeyCode::Backspace));
    assert_eq!(app.input, "undo m");
    // Ctrl+Z restores it.
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('z')));
    assert_eq!(app.input, "undo me");

    // Ctrl+W kill is undoable too.
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('w')));
    assert_eq!(app.input, "undo ");
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('z')));
    assert_eq!(app.input, "undo me", "kill undone");
}

#[tokio::test]
async fn two_stage_picker_descends_and_applies_model() {
    let (mut app, agent, (_f, _n)) = test_app();
    // Open the picker via Ctrl+M (stage 1: providers).
    assert!(!handle_key(&mut app, &agent, ctrl(KeyCode::Char('m'))));
    assert!(app.picker_open);
    assert!(app.picker_provider.is_none(), "starts at stage 1");

    // Enter on the first provider descends to stage 2.
    assert!(!handle_key(&mut app, &agent, key(KeyCode::Enter)));
    assert!(app.picker_provider.is_some(), "descended into provider");

    // Esc goes back to stage 1 (picker still open).
    assert!(!handle_key(&mut app, &agent, key(KeyCode::Esc)));
    assert!(app.picker_open && app.picker_provider.is_none());

    // Esc again closes entirely and unstashes (nothing stashed here).
    assert!(!handle_key(&mut app, &agent, key(KeyCode::Esc)));
    assert!(!app.picker_open);
}

// ---- M21: thinking level surface -------------------------------------------------

#[tokio::test]
async fn think_command_sets_levels_and_reports_unknown() {
    let (mut app, agent, (_f, _n)) = test_app();
    assert!(!type_and_enter(&mut app, &agent, "/think high"));
    assert_eq!(
        agent.thinking_level(),
        epsilon_contracts::ThinkingLevel::High
    );
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(texts.iter().any(|t| t.contains("thinking level: high")));

    // Bare /think reports the current level without changing it.
    assert!(!type_and_enter(&mut app, &agent, "/think"));
    assert_eq!(
        agent.thinking_level(),
        epsilon_contracts::ThinkingLevel::High
    );

    // Unknown level is corrected, not sent to the model.
    assert!(!type_and_enter(&mut app, &agent, "/think banana"));
    assert_eq!(
        agent.thinking_level(),
        epsilon_contracts::ThinkingLevel::High
    );
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(texts.iter().any(|t| t.contains("unknown level 'banana'")));
}

#[tokio::test]
async fn ctrl_t_cycles_the_thinking_ladder() {
    let (mut app, agent, (_f, _n)) = test_app();
    assert_eq!(
        agent.thinking_level(),
        epsilon_contracts::ThinkingLevel::Off
    );
    for _ in 0..3 {
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('t')));
    }
    // off -> minimal -> low -> medium
    assert_eq!(
        agent.thinking_level(),
        epsilon_contracts::ThinkingLevel::Medium
    );
    // M25: the footer mirror must equal what the agent actually carries
    // (displayed state == executed state, never a stale estimate).
    assert_eq!(app.thinking, agent.thinking_level());
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(texts.iter().any(|t| t.contains("thinking level: medium")));
}

// ---- M22: credential store + trust review ----------------------------------------

#[tokio::test]
async fn trust_review_holds_submits_until_answered() {
    let (mut app, agent, (_f, _n)) = test_app();
    // Simulate the startup banner having flagged a changed config.
    app.trust_review_pending = true;

    // Any normal text is bounced with guidance, never sent to the model.
    assert!(!type_and_enter(&mut app, &agent, "run my tests"));
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(texts.iter().any(|t| t.contains("config review pending")));
    assert!(!app.ui.running(), "nothing was submitted");

    // `trust` accepts and unblocks.
    assert!(!type_and_enter(&mut app, &agent, "trust"));
    assert!(!app.trust_review_pending);
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(texts.iter().any(|t| t.contains("project config accepted")));

    // After acceptance, submits flow normally again (bare /think reports).
    assert!(!type_and_enter(&mut app, &agent, "/think"));
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(texts.iter().any(|t| t.contains("thinking level:")));
}

#[tokio::test]
async fn trust_no_quits_instead_of_running_against_unreviewed_config() {
    let (mut app, agent, (_f, _n)) = test_app();
    app.trust_review_pending = true;
    assert!(
        type_and_enter(&mut app, &agent, "trust no"),
        "declining exits the session"
    );
}

#[tokio::test]
async fn trust_gate_blocks_palette_and_stages_initial_task() {
    let (mut app, agent, (_f, _n)) = test_app();
    app.trust_review_pending = true;

    // Ctrl+P must NOT open the palette while review is pending.
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('p')));
    assert!(!app.palette_open, "palette stays closed under the gate");

    // After accepting, the palette opens normally again.
    assert!(!type_and_enter(&mut app, &agent, "trust"));
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('p')));
    assert!(app.palette_open, "palette works after review");
}

#[tokio::test]
async fn trust_gate_blocks_f2_favorite_cycling() {
    // M27 audit (lesson 14: every input path gets a gate test): F2 cycles
    // models AND mutates the thinking level (M26), persisting config — it
    // must not act while the project-config review is pending.
    let (mut app, agent, (_f, _n)) = test_app();
    app.favorites = vec!["faux-1".into(), "deepseek-chat".into()];
    app.favorite_thinking = vec![("deepseek-chat".into(), "high".into())];
    app.trust_review_pending = true;

    handle_key(&mut app, &agent, key(KeyCode::F(2)));
    assert_eq!(app.model_id, "faux-1", "model unchanged under the gate");
    assert_eq!(
        agent.thinking_level(),
        epsilon_contracts::ThinkingLevel::Off,
        "thinking level unchanged under the gate"
    );

    // After accepting, cycling applies model + remembered level as usual.
    assert!(!type_and_enter(&mut app, &agent, "trust"));
    handle_key(&mut app, &agent, key(KeyCode::F(2)));
    assert_eq!(app.model_id, "deepseek-chat");
    assert_eq!(
        agent.thinking_level(),
        epsilon_contracts::ThinkingLevel::High
    );
}

#[tokio::test]
async fn compact_already_running_refused_and_second_compact_guards() {
    let (mut app, agent, _rx) = test_app();
    app.compact_running = true;
    assert!(!type_and_enter(&mut app, &agent, "/compact"));
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t.contains("already running")),
        "must refuse second /compact: {texts:?}"
    );
    // non-compact path unchanged: /timeline still works even while compact flag set
    // (compact only guards /compact).
}

#[tokio::test]
async fn timeline_picker_opens_and_enter_jumps() {
    let (mut app, agent, _rx) = test_app();
    let tmp = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(std::sync::Mutex::new(
        epsilon_core::SessionStore::open(tmp.path().join("s.jsonl")).unwrap(),
    ));
    agent.attach_session_store(std::sync::Arc::clone(&store));
    {
        let mut st = store.lock().unwrap();
        use epsilon_contracts::content::UserContent;
        use epsilon_contracts::message::UserMessage;
        use epsilon_contracts::session_entry::EntryKind;
        let _ = st
            .append(EntryKind::Message {
                message: epsilon_contracts::message::Message::User(UserMessage {
                    id: epsilon_contracts::new_id("msg"),
                    timestamp_ms: 0,
                    content: vec![UserContent::Text {
                        text: "hello".into(),
                    }],
                }),
                api_request_sha256: None,
            })
            .unwrap();
        let _ = st
            .append(EntryKind::Message {
                message: epsilon_contracts::message::Message::User(UserMessage {
                    id: epsilon_contracts::new_id("msg"),
                    timestamp_ms: 1,
                    content: vec![UserContent::Text {
                        text: "world".into(),
                    }],
                }),
                api_request_sha256: None,
            })
            .unwrap();
    }
    assert!(!type_and_enter(&mut app, &agent, "/timeline"));
    assert!(
        app.timeline_open,
        "timeline picker opens instead of notice-dumping"
    );
    assert!(app.timeline_entries.len() >= 2);
    app.timeline_index = 0;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::empty());
    let ate = handle_key(&mut app, &agent, enter);
    assert!(!ate);
    assert!(
        !app.timeline_open,
        "Enter dismisses the overlay after jumping"
    );
    assert!(!type_and_enter(&mut app, &agent, "/timeline"));
    assert!(app.timeline_open);
    let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::empty());
    handle_key(&mut app, &agent, esc);
    assert!(!app.timeline_open, "Esc dismisses without navigating");
}

// ---- M23: favorites ----------------------------------------------------------------

#[tokio::test]
async fn favorites_section_orders_above_recents_and_marks_stars() {
    let rows = crate::picker::build_rows_with(
        "",
        "faux-1",
        &["deepseek-chat".to_string()],
        &["kimi-k2.6".to_string()],
        &[],
        &[],
    );
    // Active model, then favorite, then recent (favorite NOT repeated in
    // recents), then catalog.
    assert_eq!(rows[0].id, "faux-1");
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    let fav_pos = ids.iter().position(|i| *i == "kimi-k2.6").unwrap();
    let rec_pos = ids.iter().position(|i| *i == "deepseek-chat").unwrap();
    assert!(fav_pos < rec_pos, "favorite must precede recent: {ids:?}");
    assert_eq!(rows[fav_pos].provider, "favorite");
    assert!(rows[fav_pos].is_favorite);
    // The star also shows on the same id inside the catalog section.
    assert!(
        rows.iter()
            .all(|r| if r.id == "kimi-k2.6" && r.provider != "favorite" {
                r.is_favorite
            } else {
                true
            })
    );
}

#[tokio::test]
async fn f2_cycles_favorites_and_guides_when_empty() {
    let (mut app, agent, (_f, _n)) = test_app();
    // Empty list: guidance, no crash, model unchanged.
    handle_key(&mut app, &agent, key(KeyCode::F(2)));
    assert_eq!(app.model_id, "faux-1");

    app.favorites = vec!["faux-1".into(), "deepseek-chat".into()];
    handle_key(&mut app, &agent, key(KeyCode::F(2)));
    assert_eq!(app.model_id, "deepseek-chat", "forward cycle");
    handle_key(&mut app, &agent, key(KeyCode::F(2)));
    assert_eq!(app.model_id, "faux-1", "wraps around");

    handle_key(
        &mut app,
        &agent,
        KeyEvent::new(KeyCode::F(2), KeyModifiers::SHIFT),
    );
    assert_eq!(app.model_id, "deepseek-chat", "shift = backward");

    // Persistence side effects: cycling routes through apply_model, so the
    // saved model id must reflect the last switch. The recents list also
    // persists, but asserting the real global config here races every other
    // TUI test's writes (lesson 16b) — the in-memory mirror is what F2 owns.
    // The persisted model id is covered by config-layer round-trip tests;
    // asserting the real global file here races every other TUI test's
    // writes (lesson 16b). What F2 owns: session state + in-memory mirrors.
    assert_eq!(
        app.model_id, "deepseek-chat",
        "cycle switched the session model"
    );
    assert!(
        app.recents.first().map(String::as_str) == Some("deepseek-chat"),
        "in-memory recents updated with the cycled-to model"
    );
}

#[tokio::test]
async fn f2_applies_the_favorite_remembered_thinking_level() {
    // M26 (pi _cycleScopedModel): a remembered level overrides the session
    // level on cycle; no memory inherits it. Both paths end with the footer
    // mirror equal to the agent's actual level. The memory is injected via
    // the App field (the same thing a real star toggle populates) so this
    // test never touches the user's real config.
    let (mut app, agent, (_f, _n)) = test_app();
    app.favorites = vec!["faux-1".into(), "deepseek-chat".into()];
    app.favorite_thinking = vec![("deepseek-chat".into(), "high".into())];

    handle_key(&mut app, &agent, key(KeyCode::F(2)));
    assert_eq!(app.model_id, "deepseek-chat");
    assert_eq!(
        agent.thinking_level(),
        epsilon_contracts::ThinkingLevel::High,
        "remembered level applied on cycle"
    );
    assert_eq!(app.thinking, agent.thinking_level(), "mirror stays exact");

    // Cycle back to faux-1, which has NO memory: session level inherited
    // (stays High), never reset by surprise.
    handle_key(&mut app, &agent, key(KeyCode::F(2)));
    assert_eq!(app.model_id, "faux-1");
    assert_eq!(
        agent.thinking_level(),
        epsilon_contracts::ThinkingLevel::High,
        "unremembered favorite inherits the session level"
    );
}

#[tokio::test]
async fn slash_popup_opens_and_arrows_move_selection() {
    let (mut app, agent, (_f, _n)) = test_app();
    // Typing "/" opens the popup with all 6 candidates.
    for c in "/".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(app.slash_open, "popup opens on slash");
    assert_eq!(app.slash_index, 0);

    // Down moves the highlight; Up at top stays clamped.
    handle_key(&mut app, &agent, key(KeyCode::Down));
    assert_eq!(app.slash_index, 1);
    handle_key(&mut app, &agent, key(KeyCode::Up));
    handle_key(&mut app, &agent, key(KeyCode::Up));
    assert_eq!(app.slash_index, 0, "clamped at top");

    // Arrows must NOT touch history while the popup is open.
    assert_eq!(app.input, "/");
}

#[tokio::test]
async fn enter_applies_highlighted_suggestion_then_runs_it() {
    let (mut app, agent, (_f, _n)) = test_app();
    for c in "/m".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(app.slash_open);
    let candidates = crate::picker::slash_items("/m");
    assert!(candidates.len() >= 2, "/models and /model both match");

    // Highlight the second candidate (/model — argument form) then Enter.
    // pi semantics: apply THEN submit, so "/model " opens the picker with
    // the filter prefilled (parse_command maps bare /model to Models).
    handle_key(&mut app, &agent, key(KeyCode::Down));
    assert!(!handle_key(&mut app, &agent, key(KeyCode::Enter)));
    assert!(app.picker_open, "applied /model ran and opened the picker");
    assert!(!app.slash_open, "popup closed after apply");
}

#[tokio::test]
async fn enter_with_popup_on_exact_text_still_edits_before_running() {
    let (mut app, agent, (_f, _n)) = test_app();
    // "/hotk" → fuzzy-unique to /hotkeys (no other slash item contains h-o-t-k in order).
    for c in "/hotk".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(app.slash_open);
    assert!(
        crate::picker::slash_items("/hotk")
            .iter()
            .any(|it| it.value == "/hotkeys")
    );
    assert!(!handle_key(&mut app, &agent, key(KeyCode::Enter)));
    assert!(
        app.ui.lines().iter().any(|l| l.text.contains("hotkeys")),
        "/hotkeys executed via Enter-apply: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );
    assert!(!app.slash_open);
}

#[tokio::test]
async fn esc_dismisses_popup_before_quitting_or_aborting() {
    let (mut app, agent, (_f, _n)) = test_app();
    for c in "/h".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(app.slash_open);
    // Idle Esc would quit — but the popup eats it first.
    assert!(!handle_key(&mut app, &agent, key(KeyCode::Esc)));
    assert!(!app.slash_open);
    assert_eq!(app.input, "/h", "text survives dismissal");
    // Next Esc quits (idle).
    assert!(handle_key(&mut app, &agent, key(KeyCode::Esc)));
}

#[tokio::test]
async fn tab_completes_highlighted_candidate() {
    let (mut app, agent, (_f, _n)) = test_app();
    // "/hotk" is fuzzy-unique to /hotkeys (stable even after fuzzy slash_items).
    for c in "/hotk".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(app.slash_open);
    assert!(!handle_key(&mut app, &agent, key(KeyCode::Tab)));
    assert_eq!(app.input, "/hotkeys", "Tab completed the fuzzy /hotk match");
    assert!(!app.slash_open);
}

#[tokio::test]
async fn popup_tracks_edits_and_closes_when_not_a_command() {
    let (mut app, agent, (_f, _n)) = test_app();
    for c in "/help".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(app.slash_open);
    // A space makes it an argument line → popup closes.
    handle_key(&mut app, &agent, key(KeyCode::Char(' ')));
    assert!(!app.slash_open, "space closes the command-name popup");
}

// ---- M17b: new editor keys -------------------------------------------------------

#[tokio::test]
async fn ctrl_a_ctrl_e_home_end_line_semantics() {
    let (mut app, agent, (_f, _n)) = test_app();
    for c in "hello world".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('a')));
    assert_eq!(app.cursor_chars, 0, "Ctrl+A = line home");
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('e')));
    assert_eq!(
        app.cursor_chars,
        app.input.chars().count(),
        "Ctrl+E = line end"
    );
}

#[tokio::test]
async fn ctrl_k_kills_to_line_end_into_kill_ring() {
    let (mut app, agent, (_f, _n)) = test_app();
    for c in "hello world".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('a'))); // home
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('e'))); // end
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('b'))); // back one
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('k'))); // kill "d"
    assert_eq!(app.input, "hello worl");
    // Ctrl+Y yanks the killed text back.
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('y')));
    assert_eq!(app.input, "hello world");
}

#[tokio::test]
async fn ctrl_d_forward_deletes_and_alt_backspace_kills_word() {
    let (mut app, agent, (_f, _n)) = test_app();
    for c in "ab cd".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('a')));
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('d')));
    assert_eq!(app.input, "b cd", "Ctrl+D deleted 'a'");

    handle_key(&mut app, &agent, key(KeyCode::End));
    let alt_bs = KeyEvent {
        code: KeyCode::Backspace,
        modifiers: KeyModifiers::ALT,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    };
    handle_key(&mut app, &agent, alt_bs);
    assert_eq!(app.input, "b ", "Alt+Backspace killed 'cd'");
}

#[tokio::test]
async fn ctrl_dot_redoes_what_ctrl_z_undid() {
    let (mut app, agent, (_f, _n)) = test_app();
    for c in "keep me".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    handle_key(&mut app, &agent, key(KeyCode::Backspace));
    assert_eq!(app.input, "keep m");
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('z')));
    assert_eq!(app.input, "keep me", "undo restored");
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('.')));
    assert_eq!(app.input, "keep m", "redo re-applied the deletion");
}

#[tokio::test]
async fn ctrl_b_ctrl_f_move_cursor_by_char() {
    let (mut app, agent, (_f, _n)) = test_app();
    for c in "abc".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert_eq!(app.cursor_chars, 3);
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('b')));
    assert_eq!(app.cursor_chars, 2);
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('f')));
    assert_eq!(app.cursor_chars, 3);
}

// ---- the context figure the header prints ------------------------------------
//
// `Metrics::context_used` used to be `latest_prompt_tokens` — the prompt size of
// the last call — and it was rendered as a bare `42%/200k`, which reads as window
// occupancy. On turn 1 those agree; afterwards the last prompt is only a prefix of
// the conversation, so the row under-reported while looking authoritative. The
// estimate now comes from `Agent::estimated_context_tokens` (the number the
// compaction trigger decides on) and the fragment names its own subject.

/// Run one turn through the real agent and fold every event the loop would fold.
///
/// Returns the events folded, so a test can assert the turn actually happened
/// rather than discovering an empty transcript in a confusing way.
async fn run_one_turn(app: &mut App, agent: &Arc<Agent>, text: &str) {
    let mut events = agent.subscribe();
    let message = Message::User(UserMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: 0,
        content: vec![epsilon_contracts::content::UserContent::Text {
            text: text.to_string(),
        }],
    });
    agent.run(message).await.expect("the faux run must succeed");
    while let Ok(ev) = events.try_recv() {
        apply_with_scroll(app, &ev);
    }
}

/// The frame-boundary mirror the event loop performs, without a terminal.
///
/// `drive` owns a `DefaultTerminal`, so this is the one line of it a test can
/// re-state: everything else in this test is the production path.
fn mirror_at_frame_boundary(app: &mut App, agent: &Agent) {
    app.mirror_context_size(agent);
}

/// A catalog model with a known window, so a printed share has a denominator.
///
/// Read from the catalog rather than written into the test: the window is the
/// model registry's business, and a test that hardcoded `200_000` would pass
/// against a stale copy of it.
fn model_with_a_window() -> (String, u64) {
    let id = "glm-4.7-flash".to_string();
    let info = epsilon_llm::catalog::lookup_model(&id)
        .unwrap_or_else(|| panic!("{id} is not in the catalog; pick another pinned model"));
    assert!(info.context_window > 0, "{id} has no window to report");
    (id, info.context_window)
}

/// The share the header should print for a context size, in the gauge's own terms.
///
/// Restates `Metrics::context_gauge`'s rule rather than calling it, so the test
/// is about what the user sees: a number that rounds to nothing prints `<1%`,
/// and anything larger prints whole percent.
fn expected_share(tokens: u64, window: u64) -> String {
    let percent = tokens as f64 * 100.0 / window as f64;
    if percent > 0.0 && percent < 0.5 {
        "<1%".to_string()
    } else {
        format!("{}%", percent.round() as u64)
    }
}

#[test]
fn the_context_figure_is_the_session_and_not_the_last_prompt() {
    // The defect, on the production draw path. The last provider call reported a
    // 100-token prompt while the session holds 82% of the window, so the two
    // candidate numbers are three orders of magnitude apart and the rendered row
    // can only match one of them. The old gauge printed `0%/200k` here — a true
    // statement about one request, presented as the state of the session.
    //
    // The figures are placed through the state API rather than by running a
    // turn, deliberately: the *magnitude* of the session estimate is the core's
    // business (it is anchored to whatever a provider billed), and a test that
    // needed a provider to report a particular number would be testing the
    // provider. The wiring that carries the real number is asserted below.
    let (model, window) = model_with_a_window();
    let mut app = App::over_ui(UiState::new(), model);
    app.ui.context_now = window * 82 / 100;

    let rows = render_rows(&mut app, 120, 30);
    let header = &rows[0];
    let gauge = format!("ctx 82%/{}", crate::ui::format::format_tokens(window));
    assert!(
        header.contains(&gauge),
        "the header must read the mirrored session context, expected {gauge:?} \
         and never the last prompt's 0%: {header:?}"
    );
    assert!(
        header.contains("ctx "),
        "a bare percentage reads as a different measurement: {header:?}"
    );
}

#[test]
fn a_context_that_is_measured_but_tiny_does_not_read_as_empty() {
    // 150 tokens of a 200k window is 0.075%, which rounds to `0%` — the exact
    // string the gauge avoids when nothing is known at all. A reader cannot tell
    // an empty context from an early one, and on a wide window the first several
    // turns all land in this range.
    let (model, window) = model_with_a_window();
    let mut app = App::over_ui(UiState::new(), model.clone());
    app.ui.context_now = 150;
    let rows = render_rows(&mut app, 120, 30);
    assert!(
        rows[0].contains("ctx <1%"),
        "a small measured context must not print as 0% ({window} window): {:?}",
        rows[0]
    );

    // With nothing measured at all it still says unknown, never zero.
    let mut fresh = App::over_ui(UiState::new(), model);
    let rows = render_rows(&mut fresh, 120, 30);
    assert!(
        rows[0].contains("?/"),
        "an unmeasured context must read as unknown: {:?}",
        rows[0]
    );
}

#[tokio::test]
async fn a_real_frame_draws_the_context_the_agent_reports() {
    // The other half of the property: the frame's number really is the agent's,
    // carried across the seam by `mirror_context_size`. This is the wiring a
    // widget test cannot see — a correct gauge fed a stale number renders a
    // perfect frame that says the wrong thing.
    let (mut app, agent, _receivers) = test_app();
    let (model, window) = model_with_a_window();
    app.model_id = model;

    run_one_turn(&mut app, &agent, "explain the layout module").await;
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.kind == crate::state::LineKind::Assistant),
        "the turn produced no assistant row, so the frame proves nothing: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );
    mirror_at_frame_boundary(&mut app, &agent);

    let session = agent.estimated_context_tokens() as u64;
    assert_eq!(
        app.ui.context_now, session,
        "the frame's input must be the agent's own estimate"
    );
    assert!(
        session > 0,
        "a turn that produced usage must leave a measurable context"
    );
    let rows = render_rows(&mut app, 120, 30);
    // The exact denominator text comes from the module's own formatter, so this
    // is about which *number* is drawn rather than about how a window prints.
    let gauge = format!(
        "ctx {}/{}",
        expected_share(session, window),
        crate::ui::format::format_tokens(window)
    );
    assert!(
        rows[0].contains(&gauge),
        "the header must print what the agent reports ({session} of {window}), \
         expected {gauge:?}: {:?}",
        rows[0]
    );
}

// ---- M30: sticky scroll (pi followEnd semantics) ----------------------------------

#[tokio::test]
async fn scrolling_up_freezes_the_reading_window_while_lines_append() {
    let mut app = App::over_ui(UiState::new(), "faux-1".into());
    app.ui.push_notice("one");
    app.ui.push_notice("two");
    // User pages up 10 rows from the bottom.
    app.scroll_from_bottom = 10;
    // A run start and end append nothing to the transcript any more — the
    // header carries that state — so a frozen reading window must not move
    // while runs come and go around it. This is the property that matters:
    // the reader stays where they were looking.
    for _ in 0..3 {
        apply_with_scroll(&mut app, &AgentEvent::AgentStart);
        apply_with_scroll(&mut app, &AgentEvent::AgentEnd { messages: vec![] });
    }
    assert_eq!(
        app.scroll_from_bottom, 10,
        "lifecycle events add no rows, so a frozen window must not scroll"
    );

    // A pinned view (offset 0) keeps following the tail.
    app.scroll_from_bottom = 0;
    apply_with_scroll(&mut app, &AgentEvent::AgentStart);
    assert_eq!(app.scroll_from_bottom, 0, "pinned view follows new rows");
}
#[test]
fn at_word_before_detects_fragment_and_boundary() {
    assert_eq!(
        crate::editor_ops::at_word_before("fix @src/ap", 11),
        Some((4, "src/ap".into()))
    );
    assert_eq!(
        crate::editor_ops::at_word_before("a@b", 3),
        None,
        "@ without word boundary must not trigger"
    );
    assert_eq!(
        crate::editor_ops::at_word_before("hello @", 7),
        Some((6, "".into())),
        "bare @ triggers with empty fragment"
    );
    assert_eq!(
        crate::editor_ops::at_word_before("fix @src/ap ", 12),
        None,
        "space after fragment ends the @-word"
    );
}

#[test]
fn at_insert_replaces_fragment() {
    let (s, c) = crate::editor_ops::at_insert("fix @src/ap", 11, "src/app.rs");
    assert_eq!(s, "fix src/app.rs");
    assert_eq!(c, s.chars().count());
    // no @-word: unchanged
    let (s2, c2) = crate::editor_ops::at_insert("hello world", 5, "x");
    assert_eq!(s2, "hello world");
    assert_eq!(c2, 5);
}

/// M194: typing after an `@` must not re-walk the workspace on every character.
///
/// The `@`-popup re-derives whenever the fragment changes, so every keystroke after
/// an `@` reached `at_file_rows`. M110 cached the ignore *rules* and assumed the
/// traversal was cheap; measured, one walk is 4.2 ms here and up to 9.2 ms against a
/// large tree, paid per keystroke to rebuild a list that cannot have changed.
///
/// The assertion is on the cache being **reused**, not on a duration: a timing
/// assertion would be a flaky test that fails on a busy machine and passes on a
/// warm page cache, which is the opposite of evidence. The list identity is what
/// the property actually is.
#[tokio::test]
async fn the_at_popup_walks_the_workspace_once_not_once_per_keystroke() {
    let ws = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(ws.path().join("src")).unwrap();
    std::fs::write(ws.path().join("src/app.rs"), "").unwrap();
    let (mut app, _agent) = {
        let (mut a, ag, _) = test_app();
        a.workspace_root = ws.path().to_path_buf();
        (a, ag)
    };

    // First character after the `@` populates the cache.
    app.insert_char('@');
    app.sync_at_state();
    let first = app
        .at_ignore
        .as_ref()
        .expect("the first @-fragment must populate the cache")
        .2
        .clone();
    assert!(!first.is_empty(), "the walk found the fixture file");

    // The rest of the word must reuse that list rather than walk again.
    //
    // **Asserted on a file that appears mid-word, not on list contents.** The first
    // version of this test compared the list before and after and passed against a
    // control that re-walked on every keystroke — the contents are identical either
    // way, so the assertion could not see the traversal at all. A control that passes
    // is not evidence; this one did, and it is why the property is now stated as an
    // *observable effect* instead.
    //
    // Creating a file after the cache is filled is the discriminator: a walk that
    // runs again finds it, and a walk that is cached does not — until the TTL
    // expires, which is the documented bargain.
    // Named so it MATCHES the fragment typed below: `fuzzy_filter` splits the query
    // on `/` and requires every token, so a file that does not match would be absent
    // from the popup whether or not the walk re-ran — and the assertion would be
    // vacuous in both directions. That is not a hypothetical: the first version of
    // this test used `created_while_typing.rs` against the query `src/app` and
    // passed against a re-walking control for exactly that reason.
    std::fs::write(ws.path().join("src/app_created.rs"), "").unwrap();
    for c in "src/app".chars() {
        app.insert_char(c);
        app.sync_at_state();
    }
    let after_typing = app
        .at_ignore
        .as_ref()
        .expect("the cache is still valid inside the TTL")
        .2
        .clone();

    assert_eq!(first, after_typing, "the list itself must be untouched");
    assert!(
        !after_typing.iter().any(|p| p == "src/app_created.rs"),
        "the cached list must not gain the new file"
    );
    // **The popup's own rows** are the observable the control can move. The cache
    // field is unreadable from here in the sense that matters: a re-walking
    // implementation leaves it untouched too, so asserting on it proved nothing
    // (this test passed against that control once already). What differs is what the
    // user is shown.
    let shown = app
        .at_rows
        .as_ref()
        .expect("rows are derived on every fragment change");
    assert!(
        !shown.iter().any(|p| p == "src/app_created.rs"),
        "inside the TTL the popup must serve the CACHED list, so a file created \
         after it was filled is not offered. A re-walking implementation would \
         offer it, and that is the difference this test exists to see. shown={shown:?}"
    );

    // And the popup still works, because caching must not cost correctness.
    assert!(app.at_open, "the popup is still open after typing the path");
    assert!(
        app.at_rows
            .as_ref()
            .expect("rows are derived from the cached list")
            .iter()
            .any(|p| p == "src/app.rs"),
        "the cached list still answers the fragment: {rows:?}",
        rows = app.at_rows
    );
}

#[tokio::test]
async fn at_file_popup_opens_and_tab_inserts_path() {
    let ws = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(ws.path().join("src")).unwrap();
    std::fs::write(ws.path().join("src/app.rs"), "").unwrap();
    std::fs::write(ws.path().join("src/lib.rs"), "").unwrap();
    let (mut app, agent) = {
        let (mut a, ag, _) = test_app();
        a.workspace_root = ws.path().to_path_buf();
        (a, ag)
    };
    // Type "fix @src/a" -> at popup should open filtered to src/app.rs
    for c in "fix @src/a".chars() {
        app.insert_char(c);
        app.sync_at_state();
    }
    assert!(app.at_open, "at popup opens on @word");
    assert!(
        app.at_rows
            .as_ref()
            .unwrap()
            .iter()
            .any(|p| p == "src/app.rs")
    );
    let before = app.at_index;
    // Tab inserts without space
    app.apply_at_selection(false);
    assert!(!app.at_open, "popup closes after insert");
    assert!(
        app.input.contains("src/app.rs"),
        "path inserted: {}",
        app.input
    );
    assert!(!app.input.ends_with(' '), "Tab must not add trailing space");
    let _ = before;
    let _ = agent;
}

#[tokio::test]
async fn export_writes_html_and_json() {
    let ws = tempfile::tempdir().unwrap();
    let dest_html = ws.path().join("out.html");
    let dest_json = ws.path().join("out.json");
    let (mut app, agent, _) = test_app();
    app.workspace_root = ws.path().to_path_buf();
    let backing = std::sync::Arc::new(std::sync::Mutex::new(
        epsilon_core::SessionStore::open(ws.path().join("t.jsonl")).unwrap(),
    ));
    agent.attach_session_store(std::sync::Arc::clone(&backing));
    {
        let mut guard = backing.lock().unwrap();
        guard
            .append(epsilon_contracts::session_entry::EntryKind::Message {
                message: epsilon_contracts::message::Message::User(
                    epsilon_contracts::message::UserMessage {
                        id: "m1".into(),
                        timestamp_ms: 1,
                        content: vec![epsilon_contracts::content::UserContent::Text {
                            text: "hello".into(),
                        }],
                    },
                ),
                api_request_sha256: None,
            })
            .unwrap();
    }
    let n = write_export_html(&agent, &dest_html, ws.path()).unwrap();
    assert_eq!(n, 1);
    let html = std::fs::read_to_string(&dest_html).unwrap();
    assert!(html.contains("<html"), "html export must contain html tag");
    assert!(html.contains("hello"), "html must contain message text");
    let n2 = write_export_json(&agent, &dest_json).unwrap();
    assert_eq!(n2, 1);
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&dest_json).unwrap()).unwrap();
    assert!(j.is_array(), "json export must be an array");
    let _ = app;
}

// ---- OSC 52: `/copy` over a terminal that has no local clipboard ------------
//
// The defect: `/copy` called `arboard` and printed "copied N chars to clipboard"
// unconditionally. Over SSH that is a success notice for a copy that did not
// happen — the harness wrote to the *remote* machine's clipboard, or to nothing
// at all, while the user pressed Ctrl+V locally and got whatever was there
// before. The fix has two halves and they are tested separately, because only
// one of them is reachable on any given machine: `clipboard::tests` owns the
// path decision (with the native result injected, so every branch is reachable
// everywhere), and these two own the wiring — that `/copy` asks, and that what
// it parked is written at the frame boundary.

/// Type `/copy` and press Enter: the real key path into the command.
fn run_copy(app: &mut App, agent: &Arc<Agent>) {
    type_and_enter(app, agent, "/copy");
}

/// The text of the last transcript row, whatever its kind.
fn last_line_text(app: &App) -> String {
    app.ui
        .lines()
        .last()
        .expect("a notice must have been pushed")
        .text
        .clone()
}

/// The assistant body `/copy` is expected to have taken, from the state itself.
fn last_assistant(app: &App) -> String {
    app.ui
        .lines()
        .iter()
        .rev()
        .find(|l| l.kind == crate::state::LineKind::Assistant)
        .map(|l| l.full.as_deref().unwrap_or(l.text.as_str()).to_string())
        .expect("the fixture has an assistant message")
}

#[tokio::test]
async fn copy_parks_a_sequence_that_carries_the_message_and_the_notice_names_the_path() {
    // Both halves of the contract, in one test, because they are the same claim:
    // whatever the notice says happened is what the parked sequence does. The
    // native half is machine-dependent (a dev box has a clipboard, CI may not),
    // so the assertions branch on whether a sequence was parked — and neither
    // branch can pass by accident, because the other's evidence is checked to be
    // absent.
    let (mut app, agent, _receivers) = test_app();
    // A real assistant row, produced by a real (faux) turn rather than pushed
    // into the transcript behind `/copy`'s back.
    run_one_turn(&mut app, &agent, "explain the layout module").await;
    let expected = last_assistant(&app);

    run_copy(&mut app, &agent);

    let notice = last_line_text(&app);
    match app.osc_pending.clone() {
        Some(sequence) => {
            // The terminal was asked, so the notice must say "sent" and must not
            // claim the text is on a clipboard this process wrote to.
            assert_eq!(
                notice,
                format!(
                    "copied {} chars — sent via the terminal (OSC 52)",
                    expected.chars().count()
                ),
                "the notice must name the path that ran"
            );
            assert!(!notice.contains("to the clipboard"), "{notice}");
            // The payload is the real message, base64, under the real framing.
            let payload = sequence
                .trim_start_matches("\u{1b}Ptmux;\u{1b}")
                .trim_start_matches("\u{1b}P")
                .trim_start_matches("\u{1b}]52;c;")
                .trim_end_matches("\u{1b}\\")
                .trim_end_matches('\u{7}');
            use base64::engine::Engine as _;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(payload)
                .expect("the parked payload must be base64");
            assert_eq!(
                String::from_utf8(decoded).expect("utf-8"),
                expected,
                "the sequence must carry the message, not a summary of it"
            );
        }
        None => {
            // The native path landed and the terminal was correctly left alone.
            assert_eq!(
                notice,
                format!("copied {} chars to the clipboard", expected.chars().count()),
                "a local copy must not borrow the terminal's wording"
            );
            assert!(!notice.contains("OSC 52"), "{notice}");
        }
    }
}

#[test]
fn copy_with_nothing_to_copy_parks_no_sequence_and_says_so() {
    // The negative control for the test above, and the "not written when there
    // is nothing to copy" case: an empty payload is not a no-op on the wire —
    // `ESC ] 52 ; c ; BEL` asks the terminal to *clear* the clipboard, so a
    // `/copy` with nothing to copy must not emit anything at all.
    let (mut app, agent, _receivers) = test_app();
    run_copy(&mut app, &agent);

    assert_eq!(app.osc_pending, None, "nothing to copy, nothing to send");
    assert_eq!(last_line_text(&app), "copy: no assistant message to copy");

    let mut out: Vec<u8> = Vec::new();
    assert_eq!(
        flush_osc52(&mut app, &mut out),
        None,
        "the frame boundary must have nothing to write"
    );
    assert!(out.is_empty(), "bytes reached the terminal: {out:?}");
}

#[tokio::test]
async fn the_sequence_is_written_at_the_frame_boundary_and_only_once() {
    // "Not into the rendered buffer": the sequence is not a row, a span or a
    // style — it is bytes on the terminal's own stream, written after the frame
    // the terminal is being handed. So the test drives the seam the event loop
    // drives, with a writer instead of stdout, and checks that the drain is a
    // one-shot: a sequence re-sent every frame is a clipboard rewritten sixty
    // times a second.
    let (mut app, agent, _receivers) = test_app();
    run_one_turn(&mut app, &agent, "explain the layout module").await;
    run_copy(&mut app, &agent);

    let mut out: Vec<u8> = Vec::new();
    let written = flush_osc52(&mut app, &mut out);
    if let Some(sequence) = written {
        assert_eq!(
            String::from_utf8(out.clone()).expect("the sequence is ascii"),
            sequence,
            "the writer must receive exactly the sequence that was parked"
        );
        assert!(
            out.starts_with(b"\x1b]52;c;") || out.starts_with(b"\x1bP"),
            "not an OSC 52 sequence: {out:?}"
        );
    } else {
        // Local clipboard took the text; nothing belongs on the wire.
        assert!(out.is_empty(), "unexpected bytes: {out:?}");
    }

    // The second boundary has nothing left to say.
    let mut again: Vec<u8> = Vec::new();
    assert_eq!(flush_osc52(&mut app, &mut again), None);
    assert!(
        again.is_empty(),
        "the sequence was written twice: {again:?}"
    );
}

/// M160: `/theme` switches the palette while the TUI is running, and says so.
///
/// The palette could only ever be chosen before the process started \u{2014} `EPSILON_THEME` or
/// `[tui] theme` \u{2014} so a user who guessed wrong had to edit a file and restart to find out.
/// `ThemeMode::flipped()` had carried the doc comment "for the runtime toggle" for a long time
/// with no caller.
#[tokio::test]
async fn theme_command_toggles_and_names_the_palette() {
    let (mut app, agent, (_f, _n)) = test_app();
    let start = app.theme.palette();

    // Bare `/theme` toggles, and the notice names both ends so the user can see it happened.
    assert!(!type_and_enter(&mut app, &agent, "/theme"));
    assert_eq!(app.theme.palette(), start.flipped(), "bare /theme toggles");
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t.contains(&format!(
            "palette: {} \u{2192} {}",
            start.name(),
            start.flipped().name()
        ))),
        "the switch must be reported, not silent: {texts:?}"
    );

    // A named palette sets rather than toggles, and twice is idempotent \u{2014} the second
    // `/theme dark` must not bounce back to light.
    assert!(!type_and_enter(&mut app, &agent, "/theme dark"));
    assert_eq!(app.theme.palette(), crate::ui::theme::ThemeMode::Dark);
    assert!(!type_and_enter(&mut app, &agent, "/theme dark"));
    assert_eq!(
        app.theme.palette(),
        crate::ui::theme::ThemeMode::Dark,
        "a named palette sets; it does not toggle"
    );

    // A typo is refused, with the two valid names, and changes nothing.
    let before = app.theme.palette();
    assert!(!type_and_enter(&mut app, &agent, "/theme solarized"));
    assert_eq!(
        app.theme.palette(),
        before,
        "a bad name must not change the palette"
    );
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("unknown palette 'solarized'")),
        "and it must say what was wrong: {texts:?}"
    );
}
/// M168: `/tree` and `/timeline` used to be two 96-line copies of the same picker. These
/// exercise the one body they now share, through both of its entry points, and through the
/// pure helpers that decide what a row says.
fn seed_branch(agent: &std::sync::Arc<epsilon_core::Agent>, texts: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tmp");
    let store =
        epsilon_session::SessionStore::open_in(dir.path().join("session.jsonl"), dir.path())
            .expect("session store");
    agent.attach_session_store(std::sync::Arc::new(std::sync::Mutex::new(store)));
    let guard = agent.store_handle_for_tests();
    let mut st = guard.lock().expect("store lock");
    for (i, t) in texts.iter().enumerate() {
        let user = epsilon_contracts::message::UserMessage {
            id: format!("seed-{i}"),
            timestamp_ms: 0,
            content: vec![epsilon_contracts::content::UserContent::Text {
                text: (*t).to_string(),
            }],
        };
        st.append(epsilon_contracts::session_entry::EntryKind::Message {
            message: epsilon_contracts::message::Message::User(user),
            api_request_sha256: None,
        })
        .expect("append");
    }
    dir
}
#[tokio::test]
async fn tree_and_timeline_open_one_picker_and_say_which_one_ran() {
    let (mut app, agent, (_f, _n)) = test_app();
    let _dir = seed_branch(&agent, &["first thing", "second thing"]);

    // Both entry points reach the same picker, populated from the branch tree.
    for cmd in ["/tree", "/timeline"] {
        app.timeline_open = false;
        app.timeline_entries.clear();
        type_and_enter(&mut app, &agent, cmd);
        assert!(app.timeline_open, "{cmd} did not open the picker");
        assert_eq!(
            app.timeline_entries.len(),
            2,
            "{cmd} should list both branch entries, got {:?}",
            app.timeline_entries
        );
        let kinds: Vec<&str> = app
            .timeline_entries
            .iter()
            .map(|r| r.kind.as_str())
            .collect();
        assert_eq!(kinds, vec!["user", "user"], "{cmd}: {kinds:?}");
        let snippets: Vec<&str> = app
            .timeline_entries
            .iter()
            .map(|r| r.snippet.as_str())
            .collect();
        assert_eq!(snippets, vec!["first thing", "second thing"], "{cmd}");
    }
}

#[tokio::test]
async fn an_empty_session_says_which_picker_was_asked_for() {
    // The notice is the only thing that differs between the two commands, so it is the
    // only thing that can catch a label that stopped being passed through.
    let (mut app, agent, (_f, _n)) = test_app();
    let _dir = seed_branch(&agent, &[]);
    type_and_enter(&mut app, &agent, "/tree");
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts.join("\n").contains("tree: session is empty"),
        "the /tree notice must name tree, got {texts:?}"
    );

    let (mut app2, agent2, (_f2, _n2)) = test_app();
    let _dir2 = seed_branch(&agent2, &[]);
    type_and_enter(&mut app2, &agent2, "/timeline");
    let texts2: Vec<String> = app2.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts2.join("\n").contains("timeline: session is empty"),
        "the /timeline notice must name timeline, got {texts2:?}"
    );
}

#[test]
fn a_long_entry_is_cut_and_marked_rather_than_pushed_off_screen() {
    let long = "x".repeat(500);
    let entry = epsilon_contracts::session_entry::SessionEntry {
        id: "e1".into(),
        seq: 0,
        parent_id: None,
        timestamp_ms: 0,
        kind: epsilon_contracts::session_entry::EntryKind::Message {
            message: epsilon_contracts::message::Message::User(
                epsilon_contracts::message::UserMessage {
                    id: "m1".into(),
                    timestamp_ms: 0,
                    content: vec![epsilon_contracts::content::UserContent::Text { text: long }],
                },
            ),
            api_request_sha256: None,
        },
    };
    let snip = timeline_snip(&entry);
    assert_eq!(
        snip.chars().count(),
        101,
        "100 characters plus the ellipsis"
    );
    assert!(
        snip.ends_with('\u{2026}'),
        "a cut preview must say it was cut: {snip:?}"
    );
}

#[test]
fn a_whitespace_only_entry_reads_as_empty_rather_than_blank() {
    // A blank row is indistinguishable from a row scrolled out of view; `(empty)` is not.
    let entry = epsilon_contracts::session_entry::SessionEntry {
        id: "e1".into(),
        seq: 0,
        parent_id: None,
        timestamp_ms: 0,
        kind: epsilon_contracts::session_entry::EntryKind::Message {
            message: epsilon_contracts::message::Message::User(
                epsilon_contracts::message::UserMessage {
                    id: "m1".into(),
                    timestamp_ms: 0,
                    content: vec![epsilon_contracts::content::UserContent::Text {
                        text: "   \n  ".into(),
                    }],
                },
            ),
            api_request_sha256: None,
        },
    };
    assert_eq!(timeline_snip(&entry), "(empty)");
}
/// M168: the regression test for the bug the coverage gap was hiding. An agent with no
/// durable session store is a normal state - a plain `--tui` run, a child before its store
/// exists - and `Agent::store_handle_for_tests` panics on exactly that. Nine production
/// call sites used it, so `/tree`, `/timeline`, `/fork`, `/clone`, `/name`, `/export`,
/// `/share` and the Enter-to-follow-up key all crashed the TUI instead of saying so.
#[tokio::test]
async fn a_command_needing_the_session_store_says_so_instead_of_crashing() {
    let (mut app, agent, (_f, _n)) = test_app();
    assert!(
        agent.session_store().is_none(),
        "test_app builds an agent with no store; if that changed, this test is vacuous"
    );
    for cmd in ["/tree", "/timeline", "/fork", "/clone", "/name", "/export"] {
        type_and_enter(&mut app, &agent, cmd);
        let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        let joined = texts.join("\n");
        assert!(
            joined.contains("no session store attached"),
            "`{cmd}` must report the missing store, not panic; got {joined:?}"
        );
    }
}
#[test]
fn each_entry_kind_gets_the_tag_the_picker_shows() {
    // The narrow column is how a user tells a user turn from a tool result at a glance.
    // An entry kind added later must fall into `other` rather than break the column.
    let mk = |kind| epsilon_contracts::session_entry::SessionEntry {
        id: "e1".into(),
        seq: 0,
        parent_id: None,
        timestamp_ms: 0,
        kind,
    };
    use epsilon_contracts::session_entry::EntryKind as K;
    let user = K::Message {
        message: epsilon_contracts::message::Message::User(
            epsilon_contracts::message::UserMessage {
                id: "m".into(),
                timestamp_ms: 0,
                content: vec![],
            },
        ),
        api_request_sha256: None,
    };
    assert_eq!(timeline_kind_tag(&mk(user)), "user");
    assert_eq!(
        timeline_kind_tag(&mk(K::Compaction {
            summary: "s".into(),
            tokens_before: 0,
            retained_tail: vec![],
            details: None,
            model_id: None,
            usage: None,
        })),
        "compact"
    );
    // A turn end is not a message, and saying `msg` for it would be a lie about the shape.
    let turn = K::TurnEnd {
        reason: epsilon_contracts::agent_event::TurnEndReason::Completed,
    };
    assert_eq!(timeline_kind_tag(&mk(turn)), "turn");
}
/// M171: every slash command the user can type, driven once, asserting none of them panics.
///
/// M167 measured that 18 of the 30 user-typable commands were never executed by any test,
/// and M168 found a real defect in the gap - nine of them called an accessor that panics.
/// `console-probe.py --submit` can now check them for real, but only one serialised run
/// each, at roughly 40 minutes of wall clock per batch, and the evidence does not survive
/// the next change. This does.
///
/// The assertion is deliberately weak - "did not panic" - because that is the failure mode
/// the gap was hiding. A command that reports a missing session store, an empty session, or
/// a usage string has done its job; a command that panics has not. Stronger per-command
/// assertions belong beside each command, where the expectation is knowable.
#[tokio::test]
async fn no_slash_command_panics_when_typed() {
    // Taken from the same `picker` mapping the parser uses, so a command added without a
    // test here is a gap in this list rather than a silent omission.
    // M172: read the parser's own list rather than keeping a second copy here. M173
    // removed the `exit` filter that M172 added: `type_and_enter` returns the "the app
    // should quit" signal instead of quitting, so the one command whose job is to end the
    // session can be executed and asserted like every other. M172 recorded leaving
    // `Command::Quit` uncovered as an acceptable trade. It was not one - it was a gap I
    // had just finished measuring the cost of, and then introduced on purpose.
    let commands: Vec<String> = super::super::picker::COMMAND_HEADS
        .iter()
        .map(|h| h.to_string())
        .collect();
    for cmd in &commands {
        // A fresh app per command: several of these open pickers or start state machines
        // that would otherwise bleed into the next iteration.
        let (mut app, agent, (_f, _n)) = test_app();
        let before = app.ui.lines().len();
        let should_quit = type_and_enter(&mut app, &agent, &format!("/{cmd}"));
        if cmd == "exit" {
            assert!(should_quit, "`/exit` did not report the quit signal");
            continue;
        }
        assert!(
            !should_quit,
            "`/{cmd}` reported the quit signal, which would end the sweep's own session"
        );
        let after: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
        // A command can answer in the transcript, or by opening something. Both count, and
        // the second list is every `*open` flag on `App` plus the trust review - missing one
        // is how the first version of this test failed on `/model`, which opens the model
        // picker rather than printing.
        let opened_something = app.timeline_open
            || app.session_browser.is_some()
            || app.picker_open
            || app.auth_pick_open
            || app.auth_prompt_open
            || app.palette_open
            || app.trust_review_pending;
        assert!(
            after.len() > before || opened_something,
            "`/{cmd}` produced no notice and opened no overlay - it may have done nothing \
             at all, which is how the M168 defect hid: after.len()={} before.len()={}",
            after.len(),
            before
        );
    }
}

#[tokio::test]
async fn a_prompt_template_expands_before_the_command_parser_sees_it() {
    let (mut app, _agent, (_f, _n)) = test_app();
    let dir = tempfile::tempdir().unwrap();
    app.workspace_root = dir.path().to_path_buf();
    std::fs::create_dir_all(dir.path().join(".epsilon").join("prompts")).unwrap();
    std::fs::write(
        dir.path()
            .join(".epsilon")
            .join("prompts")
            .join("review.md"),
        "---\ndescription: Review a file\n---\nReview $1 for correctness and say what is wrong.\n",
    )
    .unwrap();

    assert_eq!(
        app.discover_prompt_templates(),
        1,
        "one template discovered"
    );

    // The whole point: the template's BODY is what goes to the model, not `/review`.
    let expanded = app
        .expand_template("/review src/parser.rs")
        .expect("/review names a template");
    // No trailing newline: the frontmatter parser rejoins body lines with `\n`, so a
    // file's final newline does not survive. Asserted explicitly here because the
    // contracts test already established the behaviour and this is the second consumer.
    assert_eq!(
        expanded,
        "Review src/parser.rs for correctness and say what is wrong."
    );

    // And a text naming no template is returned untouched, so every existing path holds.
    assert_eq!(app.expand_template("just a question"), None);
    assert_eq!(app.expand_template("/no-such-template"), None);
}

/// Expansion happens BEFORE command parsing, so a template may reuse a built-in's name.
///
/// If it happened after, `/help` would be permanently shadowed and the user could never
/// reach the built-in one. This test is the reason for the ordering.
#[tokio::test]
async fn a_template_may_shadow_a_built_in_command_name() {
    let (mut app, _agent, (_f, _n)) = test_app();
    let dir = tempfile::tempdir().unwrap();
    app.workspace_root = dir.path().to_path_buf();
    std::fs::create_dir_all(dir.path().join(".epsilon").join("prompts")).unwrap();
    std::fs::write(
        dir.path().join(".epsilon").join("prompts").join("help.md"),
        "Custom help for this repository.",
    )
    .unwrap();
    app.discover_prompt_templates();

    let expanded = app
        .expand_template("/help")
        .expect("a template named help is found");
    assert_eq!(expanded, "Custom help for this repository.");
    // ...and with no such template, `/help` is still the built-in command.
    let (mut bare, _a, (_f2, _n2)) = test_app();
    bare.workspace_root = dir.path().join("does-not-exist").to_path_buf();
    bare.discover_prompt_templates();
    assert!(
        bare.expand_template("/help").is_none(),
        "with no template, /help must remain the built-in command"
    );
}

/// A project template overrides a user one of the same name.
///
/// The precedence is the reason a repository can ship its own `review` without the user
/// editing their personal config, and it is a claim worth pinning rather than assuming.
#[tokio::test]
async fn a_project_template_overrides_a_user_template() {
    let (mut app, _agent, (_f, _n)) = test_app();
    let dir = tempfile::tempdir().unwrap();
    app.workspace_root = dir.path().to_path_buf();
    let prompts = dir.path().join(".epsilon").join("prompts");
    std::fs::create_dir_all(&prompts).unwrap();
    std::fs::write(prompts.join("greet.md"), "project greeting").unwrap();

    app.discover_prompt_templates();
    let found: Vec<_> = app
        .prompt_templates
        .iter()
        .filter(|t| t.name == "greet")
        .map(|t| t.content.trim().to_string())
        .collect();
    assert_eq!(
        found,
        vec!["project greeting"],
        "exactly one greet, and it is the project's"
    );
}

/// No `prompts/` directory is the common case, not an error.
#[tokio::test]
async fn a_workspace_with_no_prompts_directory_discovers_nothing() {
    let (mut app, _agent, (_f, _n)) = test_app();
    let dir = tempfile::tempdir().unwrap();
    app.workspace_root = dir.path().to_path_buf();
    assert_eq!(app.discover_prompt_templates(), 0);
    assert!(app.prompt_templates.is_empty());
    // And submitting still works -- expansion is a no-op, not a failure.
    assert_eq!(app.expand_template("/compact"), None);
}

/// The wiring test: a template's BODY reaches the model through a real submit.
///
/// The three tests above call `expand_template` directly, and that is a limitation worth
/// naming rather than hiding — **they all pass when the submit path is disconnected.** A
/// control that removes the `expand_template` call from `submit_editor` leaves them green,
/// because the engine is reachable from a test and from nowhere else. That is precisely the
/// "tested but never wired" defect class this repository has already been bitten by twice
/// (`ToolHooks`, `TurnHooks`), so the fix is a test that goes through the only path a user
/// takes: `type_and_enter` -> `submit_editor` -> the agent.
#[tokio::test]
async fn a_typed_template_reaches_the_agent_as_its_expanded_body() {
    let (mut app, agent, _receivers) = test_app();
    let dir = tempfile::tempdir().unwrap();
    app.workspace_root = dir.path().to_path_buf();
    let prompts = dir.path().join(".epsilon").join("prompts");
    std::fs::create_dir_all(&prompts).unwrap();
    std::fs::write(
        prompts.join("audit.md"),
        "Audit $1 for secrets and report anything found.",
    )
    .unwrap();
    app.discover_prompt_templates();

    // Type it the way a user would, and submit.
    let quit = type_and_enter(&mut app, &agent, "/audit .env");
    assert!(!quit, "submitting a prompt is not quitting");

    // The expanded body is in the transcript, and the `/audit .env` text is not: the
    // template REPLACED the input rather than being appended to it.
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    let joined = texts.join("\n");
    assert!(
        joined.contains("Audit .env for secrets"),
        "the expanded body must reach the transcript: {joined:?}"
    );
    assert!(
        !joined.contains("/audit"),
        "the raw slash text must not also be sent: {joined:?}"
    );
}

/// `/templates` lists names WITH descriptions, because a name alone does not help a user
/// who half-remembers writing a template and cannot recall what it did.
#[tokio::test]
async fn templates_command_lists_names_descriptions_and_hints() {
    let (mut app, agent, _receivers) = test_app();
    let dir = tempfile::tempdir().unwrap();
    app.workspace_root = dir.path().to_path_buf();
    let prompts = dir.path().join(".epsilon").join("prompts");
    std::fs::create_dir_all(&prompts).unwrap();
    std::fs::write(
        prompts.join("commit.md"),
        "---\ndescription: Write a commit message\nargument-hint: <path>\n---\nCommit $1.\n",
    )
    .unwrap();
    app.discover_prompt_templates();

    let quit = type_and_enter(&mut app, &agent, "/templates");
    assert!(!quit);

    let shown = app
        .ui
        .lines()
        .iter()
        .map(|l| l.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        shown.contains("/commit"),
        "the name must be listed: {shown:?}"
    );
    assert!(
        shown.contains("Write a commit message"),
        "the description must be listed, not just the name: {shown:?}"
    );
    assert!(shown.contains("<path>"), "the argument hint: {shown:?}");
    // Crucially, the command LISTED it rather than EXPANDING it -- a template file called
    // `templates` must not shadow the command that lists templates.
    assert!(!shown.contains("Commit <path>"), "not the body: {shown:?}");
}

/// With nothing installed, `/templates` says where to put files rather than nothing.
#[tokio::test]
async fn templates_command_explains_where_to_put_files_when_empty() {
    let (mut app, agent, _receivers) = test_app();
    let dir = tempfile::tempdir().unwrap();
    app.workspace_root = dir.path().to_path_buf();
    app.discover_prompt_templates();

    let quit = type_and_enter(&mut app, &agent, "/templates");
    assert!(!quit);
    let shown = app
        .ui
        .lines()
        .iter()
        .map(|l| l.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        shown.contains(".epsilon/prompts"),
        "an empty list should say where to put files: {shown:?}"
    );
}

/// The user guide states the substitution grammar in a table. If the grammar changes, the
/// table is wrong, and a user following it sends a prompt that does not do what the page
/// said. Whole-token comparisons: `contains("turn_end")` passes against `turn_ending`, which
/// is how the extensions doc-drift test was caught after it initially passed.
#[test]
fn the_prompt_template_guide_documents_every_placeholder_the_code_implements() {
    let doc = include_str!("../../../../docs/user/prompt-templates.md");

    // Every form `expand_braced` / `parse_placeholder` recognises must be in the guide.
    for form in [
        "$1",
        "$2",
        "$@",
        "$ARGUMENTS",
        "${1:-none}",
        "${@:-none}",
        "${@:2}",
    ] {
        assert!(
            doc.contains(form),
            "docs/user/prompt-templates.md does not document {form}"
        );
    }
    // And the two forms the guide promises.
    for form in ["${@:1:2}", "$$"] {
        assert!(
            doc.contains(form),
            "the guide promises {form} but the guide no longer shows it"
        );
    }
    // The escape must be described, not merely present: the string `$$` appears in the
    // table, so check the sentence that explains it.
    assert!(
        doc.contains("Write `$$` for a dollar sign"),
        "the guide lists $$ but no longer explains it"
    );
}

/// The guide names the directories discovery actually scans. If a root is renamed, the
/// page sends users to a path that does nothing.
#[test]
fn the_prompt_template_guide_names_the_directories_discovery_scans() {
    let doc = include_str!("../../../../docs/user/prompt-templates.md");
    for dir in [".agents/prompts/", ".epsilon/prompts/"] {
        assert!(
            doc.contains(dir),
            "docs/user/prompt-templates.md does not mention {dir}"
        );
    }
    // The global path must match config::global_dir(), which resolves ~/.config/epsilon.
    assert!(
        doc.contains(".config/epsilon/prompts/"),
        "the guide's global path does not match config::global_dir()"
    );
}

/// The "does not work" section is load-bearing: a feature page listing only what works
/// invites people to build on capabilities that do not exist. `--prompt` is named as
/// absent, so when it is added this test fails and the line is removed deliberately.
#[test]
fn the_prompt_template_guide_states_what_does_not_work() {
    let doc = include_str!("../../../../docs/user/prompt-templates.md");
    for claim in [
        "cannot be nested",
        "not shell syntax",
        "no `--prompt` flag yet",
    ] {
        assert!(doc.contains(claim), "the guide no longer states: {claim}");
    }
}

/// `/tree` shows a branch the user rewound away from.
///
/// M196: the picker read `branch_entries()`, which walks parent links **from
/// the current leaf**. A branch abandoned by a rewind was therefore not in the
/// list at all — the row could not be selected because it was never built.
/// That is the specific capability a tree exists for, and it was missing.
///
/// The assertion is that the abandoned row is *present and marked*, not merely
/// that the list is longer: a test that only counted rows would also pass if
/// the picker dumped the whole file in file order with no structure.
#[test]
fn the_tree_picker_shows_an_abandoned_branch() {
    use epsilon_contracts::content::UserContent;
    use epsilon_contracts::message::{Message, UserMessage};
    use epsilon_contracts::session_entry::EntryKind;
    use epsilon_session::SessionStore;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let (first, abandoned) = {
        // A plain fn, not a closure: a `FnMut` capturing `&mut store` would hold
        // that borrow across the `navigate_to` below, which needs the store too.
        fn push(store: &mut SessionStore, text: &str) -> String {
            store
                .append(EntryKind::Message {
                    message: Message::User(UserMessage {
                        id: epsilon_contracts::new_id("msg"),
                        timestamp_ms: 0,
                        content: vec![UserContent::Text { text: text.into() }],
                    }),
                    api_request_sha256: None,
                })
                .unwrap()
                .id
        }
        let mut store = SessionStore::open(&path).unwrap();
        let first = push(&mut store, "first");
        // A branch the user is about to leave: two messages off the root.
        let b1 = push(&mut store, "abandoned one");
        let b2 = push(&mut store, "abandoned two");
        // Rewind to the root, then go a different way.
        store.navigate_to(&first).unwrap();
        push(&mut store, "kept one");
        (first, vec![b1, b2])
    };

    let store = SessionStore::open(&path).unwrap();
    let forest = store.tree().unwrap();
    let active: std::collections::HashSet<String> = store
        .branch_entries()
        .unwrap()
        .iter()
        .map(|e| e.id.clone())
        .collect();

    let mut rows: Vec<crate::app::TimelineRow> = Vec::new();
    super::commands::flatten_tree(&forest, 0, &active, &Default::default(), &mut rows);

    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    for b in &abandoned {
        assert!(
            ids.contains(&b.as_str()),
            "a message on the abandoned branch is in the picker: {ids:?}"
        );
        // Marked as off-lane, which is how a reader tells it from the
        // conversation they are actually in.
        let row = rows.iter().find(|r| &r.id == b).expect("row exists");
        assert!(
            !row.on_active,
            "an abandoned row is not marked as the current lane: {row:?}"
        );
    }
    // The live root sits at depth 0 and the abandoned branch hangs below it, so
    // nesting is visible. A flat dump in file order would put every row at
    // depth 0 -- which is exactly what the old `branch_entries()` picker did.
    let root = rows
        .iter()
        .find(|r| r.id == first)
        .expect("the root is a row");
    assert_eq!(root.depth, 0, "the root is at depth 0: {root:?}");
    assert!(
        abandoned
            .iter()
            .all(|b| rows.iter().find(|r| &r.id == b).expect("row").depth > 0),
        "the abandoned branch is nested under the root: {rows:?}"
    );
}

#[test]
fn the_label_editor_writes_through_the_store_and_clears_on_empty() {
    use epsilon_contracts::content::UserContent;
    use epsilon_contracts::message::{Message, UserMessage};
    use epsilon_contracts::session_entry::EntryKind;
    use epsilon_session::SessionStore;

    let dir = tempfile::tempdir().unwrap();
    let backing = std::sync::Arc::new(std::sync::Mutex::new(
        SessionStore::open(dir.path().join("s.jsonl")).unwrap(),
    ));
    let (mut app, agent, _receivers) = test_app();
    agent.attach_session_store(std::sync::Arc::clone(&backing));
    let target = {
        let mut store = backing.lock().unwrap();
        store
            .append(EntryKind::Message {
                message: Message::User(UserMessage {
                    id: epsilon_contracts::new_id("msg"),
                    timestamp_ms: 0,
                    content: vec![UserContent::Text {
                        text: "bookmark me".into(),
                    }],
                }),
                api_request_sha256: None,
            })
            .unwrap()
            .id
    };

    // Open the editor the way the `l` key does.
    app.timeline_open = true;
    app.timeline_entries = vec![crate::app::TimelineRow {
        seq: 1,
        id: target.clone(),
        kind: "user".into(),
        snippet: "bookmark me".into(),
        depth: 0,
        on_active: true,
        parent: None,
        label: None,
    }];
    app.timeline_index = 0;
    handle_key(
        &mut app,
        &agent,
        key_mod(KeyModifiers::SHIFT, KeyCode::Char('L')),
    );

    assert!(
        app.label_prompt_open,
        "Shift+L opens the label editor on the selected row (pi's chord)"
    );
    assert_eq!(
        app.label_prompt_target.as_deref(),
        Some(target.as_str()),
        "and it targets the row that was selected"
    );

    // Type, then Enter.
    for c in "the-good-part".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert!(!app.label_prompt_open, "Enter closes the editor");

    {
        let st = backing.lock().unwrap();
        let labels = st.labels().unwrap();
        assert_eq!(
            labels.get(&target).map(|(s, _)| s.as_str()),
            Some("the-good-part"),
            "the label reached the file: {labels:?}"
        );
    }

    // Empty box clears, so no separate "delete" gesture is needed.
    app.label_prompt_open = true;
    app.label_prompt_target = Some(target.clone());
    app.label_prompt_text.clear();
    app.label_prompt_cursor = 0;
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    {
        let st = backing.lock().unwrap();
        assert!(
            st.labels().unwrap().is_empty(),
            "an empty box clears the label rather than writing a blank one"
        );
    }
}

#[test]
fn a_filter_hides_entries_and_promotes_the_ones_left() {
    use super::commands::apply_filter;
    use crate::app::{TimelineRow, TreeFilter};

    let row = |id: &str, parent: Option<&str>, depth: usize, kind: &str| TimelineRow {
        seq: 0,
        id: id.into(),
        kind: kind.into(),
        snippet: String::new(),
        depth,
        on_active: true,
        parent: parent.map(str::to_string),
        label: None,
    };
    // root -> [user, tool] -> [user]   with a label entry hanging off the root
    let rows = vec![
        row("root", None, 0, "user"),
        row("tool", Some("root"), 1, "tool"),
        row("child", Some("tool"), 2, "user"),
        row("lbl", Some("root"), 1, "label"),
    ];

    // "user only" drops the tool result and the label, and the child that hung
    // off the dropped tool row is PROMOTED rather than dropped or left dangling.
    let only_user = apply_filter(&rows, TreeFilter::UserOnly, &Default::default());
    let ids: Vec<&str> = only_user.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["root", "child"], "only user messages: {ids:?}");
    let child = only_user
        .iter()
        .find(|r| r.id == "child")
        .expect("child survives");
    // Depth 0, not 1: its parent ("tool") is gone from the view, so the row is
    // PROMOTED to be a root. Leaving it at depth 1 would indent it under a row
    // the reader cannot see, which looks like a rendering bug.
    assert_eq!(
        child.depth, 0,
        "a row whose parent was filtered out is promoted to a root: {child:?}"
    );

    // "no tools" keeps the tool's CHILD but not the tool result itself.
    let no_tools = apply_filter(&rows, TreeFilter::NoTools, &Default::default());
    let ids: Vec<&str> = no_tools.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"child"),
        "the child is not a tool result: {ids:?}"
    );
    assert!(
        !ids.contains(&"tool"),
        "the tool result itself is gone: {ids:?}"
    );

    // "labelled only" needs the labels map, and finds nothing when there are none.
    assert!(apply_filter(&rows, TreeFilter::LabeledOnly, &Default::default()).is_empty());
    let mut labels = std::collections::HashMap::new();
    labels.insert("child".to_string(), "the good bit".to_string());
    let labelled = apply_filter(&rows, TreeFilter::LabeledOnly, &labels);
    assert_eq!(labelled.len(), 1);
    assert_eq!(labelled[0].id, "child");

    // "all" is everything -- and unlike the default, it keeps the bookkeeping.
    let all = apply_filter(&rows, TreeFilter::All, &Default::default());
    assert_eq!(all.len(), rows.len(), "all shows everything: {all:?}");
    let default = apply_filter(&rows, TreeFilter::Default, &Default::default());
    assert!(
        !default.iter().any(|r| r.id == "lbl"),
        "the DEFAULT filter hides bookkeeping entries -- that is the surprising rule: {default:?}"
    );
}

#[test]
fn cycling_the_filter_is_a_round_trip() {
    use crate::app::TreeFilter;
    // Five presses return to where it started, in pi's order. A filter that
    // cycled into a state it could not leave would strand the reader in a view
    // they cannot widen.
    let mut f = TreeFilter::default();
    for _ in 0..5 {
        f = f.next();
    }
    assert_eq!(f, TreeFilter::default());
    assert_eq!(TreeFilter::default().next(), TreeFilter::UserOnly);
    assert_eq!(TreeFilter::All.next(), TreeFilter::default());
    // Every mode names itself, so the title bar is never blank.
    for m in [
        TreeFilter::Default,
        TreeFilter::UserOnly,
        TreeFilter::NoTools,
        TreeFilter::LabeledOnly,
        TreeFilter::All,
    ] {
        assert!(!m.label().is_empty(), "{m:?} is named in the title");
    }
}

#[test]
fn folding_hides_the_whole_subtree_not_just_one_level() {
    use super::commands::{apply_folds, has_children};
    use crate::app::TimelineRow;

    let row = |id: &str, parent: Option<&str>| TimelineRow {
        seq: 0,
        id: id.into(),
        kind: "user".into(),
        snippet: String::new(),
        depth: 0,
        on_active: true,
        parent: parent.map(str::to_string),
        label: None,
    };
    // root -> a -> b -> c,  and a sibling of `a` under root.
    let rows = vec![
        row("root", None),
        row("a", Some("root")),
        row("b", Some("a")),
        row("c", Some("b")),
        row("sib", Some("root")),
    ];

    let mut folded = std::collections::HashSet::new();
    folded.insert("a".to_string());
    // Bind the Vec: collecting `&str` straight out of the returned temporary
    // would borrow a value that is dropped at the end of the statement.
    let kept = apply_folds(&rows, &folded);
    let shown: Vec<&str> = kept.iter().map(|r| r.id.as_str()).collect();

    assert_eq!(
        shown,
        vec!["root", "a", "sib"],
        "folding `a` hides b AND c, and leaves the sibling alone: {shown:?}"
    );
    // The property a one-level implementation fails: `c` is two levels down.
    assert!(
        !shown.contains(&"c"),
        "the fold is transitive -- c is a grandchild, not a child: {shown:?}"
    );

    // An empty fold set is a no-op, not an empty view. Getting this wrong would
    // make a picker that has never been folded show nothing at all.
    let none: std::collections::HashSet<String> = Default::default();
    assert_eq!(apply_folds(&rows, &none).len(), rows.len());

    // A leaf is not foldable -- pi refuses, so the key is never a silent no-op.
    let leaf = rows.iter().find(|r| r.id == "c").expect("c");
    assert!(!has_children(leaf, &rows), "a leaf has nothing to fold");
    let branch = rows.iter().find(|r| r.id == "a").expect("a");
    assert!(has_children(branch, &rows), "a branch does");
}

#[test]
fn search_requires_every_token_not_either_one() {
    use super::commands::apply_search;
    use crate::app::TimelineRow;

    let row = |id: &str, kind: &str, snippet: &str, label: Option<&str>| TimelineRow {
        seq: 0,
        id: id.into(),
        kind: kind.into(),
        snippet: snippet.into(),
        depth: 0,
        on_active: false,
        parent: None,
        label: label.map(str::to_string),
    };
    let rows = vec![
        row("a", "user", "refactor the auth module", None),
        row("b", "user", "refactor the parser", None),
        row("c", "user", "auth only", None),
        row("d", "user", "nothing relevant", Some("the good bit")),
    ];

    let ids = |v: Vec<TimelineRow>| -> Vec<String> { v.into_iter().map(|r| r.id).collect() };

    // ONE token: a plain substring match, so "auth" finds a and c.
    assert_eq!(
        ids(apply_search(&rows, "auth")),
        vec!["a", "c"],
        "a single token is a plain substring match"
    );
    // TWO tokens: BOTH must match. pi uses `searchTokens.every(...)`
    // (`tree-selector.ts:393`); an OR would return all three of a, b and c and
    // bury the answer among near-misses.
    assert_eq!(
        ids(apply_search(&rows, "refactor auth")),
        vec!["a"],
        "two tokens are ANDed, so only the row containing both survives"
    );
    // The label is searchable, not just the text.
    assert_eq!(
        ids(apply_search(&rows, "good")),
        vec!["d"],
        "a label is part of the searchable text"
    );
    // Case-insensitive; an empty or blank query is not a filter.
    assert_eq!(ids(apply_search(&rows, "AUTH")), vec!["a", "c"]);
    assert_eq!(apply_search(&rows, "   ").len(), rows.len());
    assert_eq!(apply_search(&rows, "").len(), rows.len());
}

/// The current leaf is shown however narrowly the view is filtered.
///
/// pi keeps the current leaf visible unconditionally
/// (`tree-selector.ts:342-343`): a view that can hide where you are cannot
/// answer "where am I?", which is the first question a session picker is asked.
///
/// The property under test is the RESCUE. A test that only checked "the matching
/// row is present" would pass against an implementation with no rescue at all,
/// because the leaf matches its own text — so each case below uses a query that
/// does NOT match the leaf.
#[test]
fn the_current_leaf_is_shown_however_narrowly_the_view_is() {
    use super::commands::visible_rows;
    use crate::app::{TimelineRow, TreeFilter};

    let rows = vec![
        TimelineRow {
            seq: 0,
            id: "leaf".into(),
            kind: "user".into(),
            snippet: "the current position".into(),
            depth: 0,
            on_active: true,
            parent: None,
            label: None,
        },
        TimelineRow {
            seq: 1,
            id: "other".into(),
            kind: "user".into(),
            snippet: "something else".into(),
            depth: 0,
            on_active: false,
            parent: None,
            label: None,
        },
    ];

    let show = |q: &str| -> Vec<String> {
        visible_rows(
            &rows,
            TreeFilter::All,
            &Default::default(),
            q,
            &Default::default(),
        )
        .into_iter()
        .map(|r| r.id)
        .collect()
    };

    // A query matching NOTHING still shows the leaf -- and ONLY the leaf, so the
    // rescue is not a disguised "show everything".
    assert_eq!(
        show("zzz-no-such-text"),
        vec!["leaf"],
        "the leaf is rescued; the other row stays hidden"
    );
    // "else" is in the OTHER row, and the leaf rides along because it is
    // mandatory -- in the tree's order, not appended at the end, which is what a
    // naive rescue does.
    assert_eq!(
        show("else"),
        vec!["leaf", "other"],
        "the leaf rides along with any search, in tree order"
    );
}

/// `/skills` shows what each skill IS, not only its name.
///
/// M196: this printed `names.join(", ")` — a comma-joined list of names, while the
/// `Skill` struct already carried a `description` that was simply discarded. The
/// repository had solved this exact problem for `/templates` one command away
/// (`/name — description`), which makes the omission obvious in hindsight.
///
/// The property is INFORMATION REACHING THE SCREEN. A test that only counted the
/// rows would pass against the old code, which emitted ONE line for the whole set.
#[test]
fn the_skills_listing_carries_a_description_per_skill() {
    use super::commands::skill_lines;
    use epsilon_core::skills::Skill;

    let mk = |name: &str, description: &str, manual: bool| Skill {
        name: name.into(),
        description: description.into(),
        dir: std::path::PathBuf::from("/tmp").join(name),
        disable_model_invocation: manual,
    };
    let skills = vec![
        mk("commit", "write a conventional commit message", false),
        mk("review", "review a diff for defects", true),
    ];

    // An empty disable-list first: the point of this half is the DESCRIPTION.
    let lines = skill_lines(&skills, &[]);
    assert_eq!(
        lines.len(),
        2,
        "one line per skill, not one line for the whole set: {lines:?}"
    );
    assert!(
        lines[0].contains("commit") && lines[0].contains("conventional commit"),
        "a name WITHOUT its description is the old behaviour: {lines:?}"
    );
    assert!(
        lines[1].contains("manual only"),
        "the manual-only flag is stated, as pi's resource panel does: {lines:?}"
    );

    // And a skill that has been switched OFF says so. A listing that merely
    // omitted it would be indistinguishable from one that was never installed,
    // and the user would go looking for a directory that is right there.
    let off = skill_lines(&skills, &["commit".to_string()]);
    assert!(
        off[0].contains("[off]"),
        "a disabled skill is marked, not hidden: {off:?}"
    );
    assert!(
        !off[1].contains("[off]"),
        "and only the disabled one is: {off:?}"
    );
}

/// A jump lands where pi's would.
///
/// Three properties, each of which a naive implementation gets wrong:
///
/// 1. **Strictly after (or before) the cursor.** A forward jump to the character
///    under the cursor must move, or it "finds" the cursor and appears broken.
/// 2. **Forward is the FIRST match, backward is the LAST.** Searching backward
///    with a forward scan lands on the nearest one going the wrong way.
/// 3. **`None` when there is no match**, rather than a clamped position — the
///    caller reports "not found" instead of silently moving the cursor somewhere
///    the reader did not ask for.
#[test]
fn jump_to_char_lands_where_pi_lands() {
    use crate::app::{JumpDir, jump_to_char};

    let text = "alpha.beta.gamma";
    let f = |c, n| jump_to_char(text, c, n, JumpDir::Forward);
    let b = |c, n| jump_to_char(text, c, n, JumpDir::Backward);

    // The string is 0:a 1:l 2:p 3:h 4:a 5:. 6:b 7:e 8:t 9:a 10:. 11:g 12:a 13:m 14:m 15:a
    // -- written out because the FIRST version of this test asserted 13 for the
    // nearest 'a' going back from 15, which is wrong: 12 is nearer. The
    // implementation was right; the hand-counted expected value was not.
    assert_eq!(f(0, '.'), Some(5), "the first '.' after the start");
    assert_eq!(
        f(5, '.'),
        Some(10),
        "and the next one, NOT the one under the cursor"
    );
    assert_eq!(
        f(0, 'a'),
        Some(4),
        "the first 'a' strictly after position 0"
    );
    assert_eq!(
        b(15, '.'),
        Some(10),
        "backward finds the LAST '.' before the cursor"
    );
    assert_eq!(
        b(15, 'a'),
        Some(12),
        // The string is 0:a 1:l 2:p 3:h 4:a 5:. 6:b 7:e 8:t 9:a 10:. 11:g 12:a
        // 13:m 14:m 15:a -- written out because this assertion was first written
        // as Some(13) by hand, and 12 is the NEAREST. The implementation was
        // right; the counted expected value was not. `len()` is 16, not 15.
        "the NEAREST 'a' going back, not the first in the string"
    );

    // No match ahead or behind.
    assert_eq!(f(13, 'z'), None, "nothing after the cursor");
    assert_eq!(b(0, 'z'), None, "nothing before it");
    assert_eq!(
        jump_to_char("", 0, 'a', JumpDir::Forward),
        None,
        "empty input"
    );

    // A cursor past the end must not panic -- the count is a cursor in CHARS.
    assert_eq!(
        f(999, 'a'),
        None,
        "an out-of-range cursor is a miss, not a panic"
    );
}

/// An armed jump consumes EXACTLY ONE character, and then stops being armed.
///
/// This is the failure mode that would make the feature unusable: if the mode
/// survived, the SECOND character the user typed would move the cursor instead of
/// being typed, and the message would come out mangled with no explanation.
#[test]
fn an_armed_jump_consumes_one_character_and_then_ends() {
    use crate::app::JumpDir;

    let (mut app, _agent, _rx) = test_app();
    app.input = "alpha.beta.gamma".into();
    app.cursor_chars = 0;

    app.toggle_jump(JumpDir::Forward);
    assert_eq!(app.jump_mode, Some(JumpDir::Forward), "armed");

    // The next printable character is a JUMP, not an insert.
    app.insert_char('.');
    assert_eq!(app.cursor_chars, 5, "moved to the next '.'");
    assert_eq!(
        app.input, "alpha.beta.gamma",
        "and the text is untouched: {}",
        app.input
    );
    assert_eq!(app.jump_mode, None, "the mode is consumed");

    // The character after that is an ordinary INSERT again.
    app.insert_char('!');
    assert_eq!(app.input, "alpha!.beta.gamma", "inserted normally");
    assert_eq!(app.cursor_chars, 6);
}

/// A jump that finds nothing says so, and changes neither the text nor the cursor.
///
/// The alternative -- inserting the character, or clamping the cursor -- is how
/// "there was no match" turns into "why did my cursor move".
#[test]
fn a_jump_with_no_match_reports_it_and_inserts_nothing() {
    use crate::app::JumpDir;

    let (mut app, _agent, _rx) = test_app();
    app.input = "alpha".into();
    app.cursor_chars = 5;

    app.toggle_jump(JumpDir::Forward);
    app.insert_char('z');
    assert_eq!(app.input, "alpha", "no character was inserted");
    assert_eq!(app.cursor_chars, 5, "and the cursor did not move");
    assert_eq!(app.jump_mode, None, "the mode is still consumed");
    assert!(
        app.ui.lines().iter().any(|l| l.text.contains("no 'z'")),
        "and the reason is on screen"
    );
}

/// The backward chord arms BACKWARD, which depends on the arms being in order.
///
/// M196: written the other way round, `Ctrl+]` was matched first, and its guard is
/// also satisfied by `Ctrl+Alt+]` — so backward was **unreachable** and the key
/// silently did the other thing. A comment about ordering is a promise; this is a
/// check, and it fails the moment the arms are swapped.
///
/// Driven through `handle_key` rather than by reading the file, because the
/// property is about which arm the runtime reaches.
#[test]
fn the_backward_jump_chord_arms_backward_not_forward() {
    let (mut app, agent, _rx) = test_app();

    // Ctrl+Alt+] -- the backward chord.
    let mut k = key(KeyCode::Char(']'));
    k.modifiers = KeyModifiers::CONTROL | KeyModifiers::ALT;
    handle_key(&mut app, &agent, k);
    assert_eq!(
        app.jump_mode,
        Some(crate::app::JumpDir::Backward),
        "ctrl+alt+] arms BACKWARD"
    );

    // Ctrl+] -- the forward chord, which must not be shadowed by the above.
    app.jump_mode = None;
    let mut k = key(KeyCode::Char(']'));
    k.modifiers = KeyModifiers::CONTROL;
    handle_key(&mut app, &agent, k);
    assert_eq!(
        app.jump_mode,
        Some(crate::app::JumpDir::Forward),
        "ctrl+] still arms FORWARD"
    );
}

/// The `/tree` panel DRAWS what the state says: the filter, the search field, and
/// the two different fold glyphs.
///
/// **This exists because every other test in this file asserts on state, and
/// "what nothing exercises" is a section of the milestone log rather than a
/// feeling.** A panel that renders the wrong rows, omits the search field, or
/// draws the same glyph for collapsed and expanded would pass every other test
/// here. Rendering is the only way to see those.
#[test]
fn the_tree_panel_draws_its_filter_search_field_and_fold_glyphs() {
    use crate::app::{TimelineRow, TreeFilter};

    let (mut app, _agent, _rx) = test_app();
    app.tree_filter = TreeFilter::UserOnly;
    app.timeline_open = true;
    app.tree_query = "refactor".into();
    app.tree_searching = true;
    let parent = TimelineRow {
        seq: 0,
        id: "a".into(),
        kind: "user".into(),
        snippet: "the parent".into(),
        depth: 0,
        on_active: true,
        parent: None,
        label: Some("the good bit".into()),
    };
    // A SECOND branch, not folded, so the frame shows one of each glyph. With
    // only the folded pair, the expanded glyph is legitimately absent -- the
    // assertion below is that BOTH appear, so the fixture has to produce both.
    let child = TimelineRow {
        seq: 1,
        id: "b".into(),
        kind: "user".into(),
        snippet: "the child".into(),
        depth: 1,
        on_active: false,
        parent: Some("a".into()),
        label: None,
    };
    let open_branch = TimelineRow {
        seq: 2,
        id: "c".into(),
        kind: "user".into(),
        snippet: "an open branch".into(),
        depth: 0,
        on_active: false,
        parent: None,
        label: None,
    };
    let open_leaf = TimelineRow {
        seq: 3,
        id: "d".into(),
        kind: "user".into(),
        snippet: "under the open branch".into(),
        depth: 1,
        on_active: false,
        parent: Some("c".into()),
        label: None,
    };
    app.folded.insert("a".into());
    // `a` is folded, so its child is NOT in the visible list; `c` is not folded,
    // so its child is. That is the two-state contrast, drawn.
    app.timeline_all = vec![parent.clone(), child, open_branch.clone(), open_leaf];
    app.timeline_entries = vec![parent, open_branch];
    app.timeline_index = 0;

    let rows = render_rows(&mut app, 100, 30);
    let screen: String = rows
        .iter()
        .map(|r| r.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        screen.contains("user only"),
        "the ACTIVE FILTER is named, so a picker that hides rows says why: {screen}"
    );
    assert!(
        screen.contains("/refactor"),
        "the search text is on screen while it is being typed: {screen}"
    );
    assert!(
        screen.contains("the good bit"),
        "a label is drawn on its row: {screen}"
    );
    assert!(
        screen.contains('\u{25be}'),
        "a collapsed row carries the collapsed glyph: {screen}"
    );
    assert!(
        screen.contains('\u{25b8}'),
        "and an expanded one carries the OTHER glyph -- the same glyph for both \
         states would make them indistinguishable: {screen}"
    );
}

/// The label prompt and the jump-mode notice actually DRAW.
///
/// Same reason as the tree-panel test, and the same payoff: the previous gap in
/// this file was a field rendering in the wrong panel, which no state assertion
/// could see. Two more surfaces asserted only on state, with the same risk.
///
/// `k` in the `/tree` picker forks the session at the SELECTED row.
///
/// The property that matters: the fork contains **only the ancestry of the
/// selected entry**, not the whole file. A copy would carry every abandoned
/// branch into the new session under one identity, which is the M194 defect this
/// primitive was written to fix -- so a test that only checked "a new file
/// appeared" would pass against the wrong implementation.
#[test]
fn forking_from_the_tree_writes_only_the_chosen_ancestry() {
    use epsilon_contracts::content::UserContent;
    use epsilon_contracts::message::{Message, UserMessage};
    use epsilon_contracts::session_entry::EntryKind;
    use epsilon_session::SessionStore;

    let dir = tempfile::tempdir().unwrap();
    let backing = std::sync::Arc::new(std::sync::Mutex::new(
        SessionStore::open(dir.path().join("s.jsonl")).unwrap(),
    ));
    fn push(store: &mut SessionStore, text: &str) -> String {
        store
            .append(EntryKind::Message {
                message: Message::User(UserMessage {
                    id: epsilon_contracts::new_id("msg"),
                    timestamp_ms: 0,
                    content: vec![UserContent::Text { text: text.into() }],
                }),
                api_request_sha256: None,
            })
            .unwrap()
            .id
    }
    let first = {
        let mut st = backing.lock().unwrap();
        let a = push(&mut st, "first");
        let b = push(&mut st, "second");
        let c = push(&mut st, "third");
        (a, b, c)
    };

    let (mut app, agent, _rx) = test_app();
    agent.attach_session_store(std::sync::Arc::clone(&backing));
    app.timeline_open = true;
    app.timeline_entries = vec![crate::app::TimelineRow {
        seq: 1,
        id: first.1.clone(),
        kind: "user".into(),
        snippet: "second".into(),
        depth: 0,
        on_active: false,
        parent: Some(first.0.clone()),
        label: None,
    }];
    app.timeline_index = 0;

    handle_key(&mut app, &agent, key(KeyCode::Char('k')));

    let st = backing.lock().unwrap();
    let dir_now = st.path().parent().unwrap();
    // `.jsonl` only: the directory also holds the session's own write lease, and
    // counting that as a fork is the kind of off-by-one that makes a correct
    // implementation look broken.
    let forks: Vec<_> = std::fs::read_dir(dir_now)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl") && p != st.path())
        .collect();
    assert_eq!(forks.len(), 1, "exactly one fork was written: {forks:?}");

    // The fork stops at the selected entry: "third" is NOT in it.
    let body = std::fs::read_to_string(&forks[0]).unwrap();
    assert!(body.contains("first"), "the ancestry is there: {body}");
    assert!(
        body.contains("second"),
        "including the fork point itself: {body}"
    );
    assert!(
        !body.contains("third"),
        "and NOTHING after it -- a copy would carry it: {body}"
    );
}

/// A prompt jump lands on a USER TURN, skips the one you are on, and says when
/// there is none.
///
/// Three properties, each of which a naive version gets wrong:
///
/// 1. **Starts one row away.** pi walks from `scrollTop + direction`
///    (`tui-alt-screen.ts:488`), so a prompt the reader is already looking at is
///    not a jump. Without it, the key appears to do nothing.
/// 2. **Skips non-user rows.** A session is mostly tool results; landing on one
///    is exactly what the feature exists to avoid.
/// 3. **`None` at the ends**, rather than clamping. Clamping is how "you are
///    already at the top" becomes a key that looks broken.
#[test]
fn a_prompt_jump_lands_on_a_user_turn_and_reports_the_ends() {
    use crate::app::jump_to_prompt;
    use crate::state::TranscriptLine;

    // Built with every field set explicitly: `TranscriptLine` carries six, and a
    // fixture that sets three will not compile the moment a seventh is added --
    // which is a good thing, but only if the test says what the shape is.
    let line = |kind: crate::state::LineKind| TranscriptLine {
        id: 0,
        kind,
        text: String::new(),
        full: None,
        code: false,
        md: None,
        card: None,
    };
    // 0 user, 1 assistant, 2 tool, 3 tool, 4 user, 5 assistant, 6 user
    let lines = vec![
        line(crate::state::LineKind::User),
        line(crate::state::LineKind::Assistant),
        line(crate::state::LineKind::ToolResultOk),
        line(crate::state::LineKind::ToolResultOk),
        line(crate::state::LineKind::User),
        line(crate::state::LineKind::Assistant),
        line(crate::state::LineKind::User),
    ];

    // Sitting at the bottom (row 6, a user turn). Backward skips row 6 itself and
    // lands on row 4, not on the assistant at 5.
    assert_eq!(
        jump_to_prompt(&lines, 0, -1),
        Some(2),
        "row 4 is 2 from the bottom"
    );
    // Forward from row 6 has nothing after it.
    assert_eq!(
        jump_to_prompt(&lines, 0, 1),
        None,
        "nothing after the last turn"
    );

    // From row 4 (2 from the bottom), forward reaches row 6.
    assert_eq!(jump_to_prompt(&lines, 2, 1), Some(0));
    // Backward from row 4 passes the tool results and lands on row 0.
    assert_eq!(
        jump_to_prompt(&lines, 2, -1),
        Some(6),
        "row 0 is 6 from the bottom"
    );

    // The ends report rather than clamp.
    assert_eq!(
        jump_to_prompt(&lines, 6, -1),
        None,
        "nothing before the first turn"
    );
    assert_eq!(
        jump_to_prompt(&[], 0, -1),
        None,
        "an empty transcript has no prompts"
    );

    // A transcript with no user turns at all is not a crash.
    let tool_only = vec![line(crate::state::LineKind::ToolResultOk); 3];
    assert_eq!(jump_to_prompt(&tool_only, 0, -1), None);
}

/// A half page is HALF THE MEASURED VIEWPORT, and top/bottom reach the ends.
///
/// The property worth pinning: the step tracks `geometry.visible`. A fixed ten
/// rows is not a half page -- it is a page, or a fifth of one, depending on the
/// terminal, and a user with a 50-row window would find the key does almost
/// nothing. The geometry is measured at paint time, so a frame that has never
/// been painted is the one case where no sensible half exists; that is clamped
/// to one row rather than guessed at.
///
/// The copy key reaches the same clipboard path `/copy` does.
///
/// The property is that the key is wired to the **command**, not to a private
/// copy of the clipboard logic. A test that only checked "the key is bound"
/// would pass against an arm that called `clipboard::copy` directly and
/// therefore did not park the OSC-52 sequence or print the multiplexer notice —
/// which is the part that works over SSH.
#[test]
fn the_copy_key_reaches_the_clipboard_path() {
    let (mut app, agent, _receivers) = test_app();
    // A real assistant row from a real (faux) turn, the same way the existing
    // `/copy` tests build one -- pushing a line into the transcript behind
    // `/copy`'s back would test a fixture rather than the path.
    // `/copy` with nothing to copy is the negative control, and it is the one
    // that proves the key is wired: with no assistant row, the key must produce
    // exactly the notice `/copy` produces, and nothing else.
    let before = app.ui.lines().len();
    handle_key(&mut app, &agent, key(KeyCode::F(4)));
    assert!(
        app.ui.lines().len() > before,
        "the key reached the copy command at all"
    );
    assert_eq!(
        last_line_text(&app),
        "copy: no assistant message to copy",
        "the SAME notice `/copy` gives -- so the key routes through the command \\
         rather than a private copy of the clipboard logic"
    );
    assert_eq!(
        app.osc_pending, None,
        "and parks nothing, because there was nothing to park"
    );
}

/// The copy key must work while the tree is open, not just in the transcript.
///
/// The key was bound (`keybindings.rs:140`, F4) and the global arm that honours it
/// exists — but the `if app.timeline_open` block returns for *every* key 620 lines
/// above it, so inside the tree the chord hit `_ => {}` and did nothing, silently.
/// `/hotkeys` advertised it and the panel printed it, so this is the "a silent
/// no-op is worse than a missing feature" case rather than an absent feature.
///
/// The negative is the test: `handle_key` returning without a notice is exactly
/// the old behaviour, and asserting on the *absence* of a notice would pass
/// vacuously against a build where the command never ran. So the assertion is on
/// the notice `/copy` produces, reached through the open tree.
#[tokio::test]
async fn the_copy_key_still_works_while_the_tree_is_open() {
    let (mut app, agent, _rx) = test_app();
    // The tree needs a store with entries in it — `test_app()` alone leaves the
    // picker closed, and the precondition assertion below caught exactly that on
    // the first run. An assertion that a fixture is not empty is worth having
    // even when it fires on the test rather than the code.
    let tmp = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(std::sync::Mutex::new(
        epsilon_core::SessionStore::open(tmp.path().join("s.jsonl")).unwrap(),
    ));
    agent.attach_session_store(std::sync::Arc::clone(&store));
    {
        let mut st = store.lock().unwrap();
        use epsilon_contracts::content::UserContent;
        use epsilon_contracts::message::UserMessage;
        use epsilon_contracts::session_entry::EntryKind;
        st.append(EntryKind::Message {
            message: epsilon_contracts::message::Message::User(UserMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: 0,
                content: vec![UserContent::Text {
                    text: "a question worth answering".into(),
                }],
            }),
            api_request_sha256: None,
        })
        .unwrap();
    }
    // Open the tree the way a user does, through the command, so the test is on
    // the production path rather than on a state flag poked by hand.
    app.run_command(&agent, crate::picker::Command::Timeline);
    assert!(
        app.timeline_open,
        "precondition: the tree is open, or this test proves nothing"
    );

    let before = app.ui.lines().len();
    handle_key(&mut app, &agent, key(KeyCode::F(4)));

    assert!(
        app.ui.lines().len() > before,
        "the copy key was swallowed by the tree's catch-all arm: the block returns \
         for every key before the global copy arm is ever reached"
    );
    assert_eq!(
        last_line_text(&app),
        "copy: no assistant message to copy",
        "and it is the SAME notice `/copy` gives, so the tree routes through the \
         command rather than owning a second copy of the clipboard logic"
    );
}

/// A compaction says what it cost, in the row, with the same numbers it records.
///
/// **The property is agreement, not presence.** A line that said "context
/// compacted" would pass a test that only looked for the word. The figure the
/// user reads and the figure folded into the header's totals come from the same
/// `Usage`, so a reader who adds up the transcript gets the header's number.
#[test]
fn a_compaction_row_reports_the_same_tokens_it_records() {
    use epsilon_contracts::agent_event::AgentEvent;

    let (mut app, _agent, _rx) = test_app();
    let usage = epsilon_contracts::message::Usage {
        input: 12_400,
        output: 310,
        cache_read: 8_900,
        cache_write: 0,
        total_tokens: 21_610,
        cache_reported: true,
    };

    // The file's own driver for an event, so the totals move through exactly the
    // path a real compaction takes.
    apply_with_scroll(
        &mut app,
        &AgentEvent::CompactionFinished {
            // `Usage` is `Copy`, so no clone -- and clippy says so, which is a
            // free correctness check on the fixture.
            usage: Some(usage),
            model_id: Some("test-model".into()),
        },
    );

    // The property is AGREEMENT between the row and the header, asserted through
    // the SAME formatter both use. Asserting on a literal would pass against a
    // second formatter that had quietly drifted, which is the failure this
    // catches: a header reading `12k` beside a row reading `12000`, with the
    // reader left to guess whether those are the same figure.
    let f = epsilon_core::cache_stats::fmt_tokens;
    let row = last_line_text(&app);
    assert!(
        row.contains(&f(usage.input)) && row.contains(&f(usage.output)),
        "the row names the input and output of this call: {row}"
    );
    assert!(
        row.contains(&f(usage.input + usage.cache_read)),
        "and the re-read that makes the figure large: {row}"
    );
    // "re-read" is the word that explains the size. Without it a user reads the
    // input figure as an ordinary turn rather than a whole history sent again.
    assert!(row.contains("re-read"), "and names WHY it is large: {row}");
}

#[test]
fn the_label_prompt_and_jump_notice_draw() {
    use crate::app::JumpDir;

    // 1. The label editor: target, caret, and the save/clear hint.
    let (mut app, _agent, _rx) = test_app();
    app.label_prompt_open = true;
    app.label_prompt_target = Some("ent_abc123".into());
    app.label_prompt_text = "the good bit".into();
    app.label_prompt_cursor = 12;

    let rows = render_rows(&mut app, 100, 30);
    let screen: String = rows
        .iter()
        .map(|r| r.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        screen.contains("ent_abc123"),
        "the entry being labelled is named, so the user knows what they are naming: {screen}"
    );
    assert!(
        screen.contains("the good bit"),
        "and the text typed so far is on screen: {screen}"
    );
    assert!(
        screen.contains('\u{2588}'),
        "with a caret, so the cursor position is visible: {screen}"
    );
    assert!(
        screen.contains("Enter saves"),
        "and the key that commits it: {screen}"
    );

    // 2. The jump notice names the DIRECTION, because forward and backward are
    //    different keys and the difference is the whole feature.
    let (mut app2, _a2, _r2) = test_app();
    app2.input = "alpha.beta".into();
    app2.toggle_jump(JumpDir::Backward);
    let rows = render_rows(&mut app2, 100, 30);
    let screen2: String = rows
        .iter()
        .map(|r| r.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        screen2.contains("jump backward"),
        "an armed jump says WHICH WAY, and what to do next: {screen2}"
    );
}

/// `!` runs a shell command without a model round-trip, and prints what it printed.
///
/// The property that matters is that the command **actually ran**. A test asserting
/// only that the draft was cleared, or that some notice appeared, would pass
/// against an implementation that echoed the text back — so the fixture is a real
/// command with a real output and the assertion is on the output.
///
/// The key path is driven one character at a time rather than by setting `app.input`
/// and calling `submit_editor`, because *that* path is the thing under test: it is
/// what decides whether a leading `!` is read as a shell escape or typed into a
/// prompt.
#[tokio::test]
async fn a_bang_runs_a_shell_command_and_shows_its_output() {
    let (mut app, agent, receivers, approvals) = test_app_with_bash();
    let (_frames, mut notices) = receivers;
    type_and_enter(&mut app, &agent, "!echo epsilon-bang-marker");
    // The command is gated, so a `!` run parks an approval ask first. A user
    // answers it, and this test does the same rather than bypassing the gate —
    // bypassing it would test a path no user takes.
    let _approver = tokio::spawn({
        let channel = approvals.clone();
        async move {
            channel
                .next_ask()
                .await
                .expect("a gated bash call asks")
                .reply(PermissionDecision::AllowOnce)
        }
    });

    // Drain until the output arrives, with a wall-clock bound so a hang fails the
    // test rather than the suite.
    //
    // **`Err` continues; it does not break.** A spawn is asynchronous: the first
    // notice ("running: ...") arrives at once, and the output follows only after
    // the gate is answered and the process exits. Breaking on the first timeout
    // ends the wait after 50ms holding the "running:" line and nothing else —
    // which is exactly what this test did on its first run, and exactly the shape
    // of the "TUI looks dead" symptom this repository has chased since M151. A
    // poll that finds nothing *yet* is not a closed channel.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut seen = String::new();
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(100), notices.recv()).await {
            Ok(Some(n)) => {
                seen.push_str(&n);
                seen.push('\n');
                // Break on the OUTPUT, not on the marker. The `running:`
                // notice names the command and therefore the marker, so
                // stopping on the marker alone would accept a "running" line
                // as proof the command printed anything.
                if seen.contains("shell") && seen.contains("epsilon-bang-marker") {
                    break;
                }
            }
            // Nothing yet: keep waiting, bounded by `deadline`.
            Err(_) => continue,
            // The sender is gone, so nothing more can arrive. That IS an ending.
            Ok(None) => break,
        }
    }

    eprintln!("PROBE seen = {seen:?}");
    // Asserted on the OUTPUT line, not on the marker alone: the `running:` notice
    // names the command and therefore the marker, so a `contains` on the marker
    // would pass against a program that printed the command back and never ran it.
    assert!(
        seen.lines()
            .any(|l| l.starts_with("shell: ") && l.contains("epsilon-bang-marker")),
        "the command ran and its OUTPUT reached the user: {seen}"
    );
    assert!(
        !app.ui.running(),
        "and it did not start a model run -- a `!` command is not a prompt"
    );
}

/// `!!` says on screen that the output was kept out of the context.
///
/// pi's two-bang form (`interactive-mode.ts:3107-3108`) is only worth having
/// because the output is excluded. So the test asserts the **label** says so, not
/// merely that some text appeared: an implementation that dropped the distinction
/// would still run the command and still print the output, and would pass a test
/// that only looked for the output.
#[tokio::test]
async fn a_double_bang_says_the_output_stays_out_of_the_context() {
    let (mut app, agent, receivers, approvals) = test_app_with_bash();
    let (_frames, mut notices) = receivers;
    type_and_enter(&mut app, &agent, "!!echo epsilon-excluded-marker");
    // The command is gated, so a `!` run parks an approval ask first. A user
    // answers it, and this test does the same rather than bypassing the gate —
    // bypassing it would test a path no user takes.
    let _approver = tokio::spawn({
        let channel = approvals.clone();
        async move {
            channel
                .next_ask()
                .await
                .expect("a gated bash call asks")
                .reply(PermissionDecision::AllowOnce)
        }
    });

    // Drain until the output arrives, with a wall-clock bound so a hang fails the
    // test rather than the suite.
    //
    // **`Err` continues; it does not break.** A spawn is asynchronous: the first
    // notice ("running: ...") arrives at once, and the output follows only after
    // the gate is answered and the process exits. Breaking on the first timeout
    // ends the wait after 50ms holding the "running:" line and nothing else —
    // which is exactly what this test did on its first run, and exactly the shape
    // of the "TUI looks dead" symptom this repository has chased since M151. A
    // poll that finds nothing *yet* is not a closed channel.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut seen = String::new();
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(100), notices.recv()).await {
            Ok(Some(n)) => {
                seen.push_str(&n);
                seen.push('\n');
                // Break on the OUTPUT, not on the marker. The `running:`
                // notice names the command and therefore the marker, so
                // stopping on the marker alone would accept a "running" line
                // as proof the command printed anything.
                if seen.contains("shell") && seen.contains("epsilon-excluded-marker") {
                    break;
                }
            }
            // Nothing yet: keep waiting, bounded by `deadline`.
            Err(_) => continue,
            // The sender is gone, so nothing more can arrive. That IS an ending.
            Ok(None) => break,
        }
    }

    assert!(
        seen.lines()
            .any(|l| l.starts_with("shell (kept out of context): ")
                && l.contains("epsilon-excluded-marker")),
        "the command still ran, under the excluded label: {seen}"
    );
}

/// A page key moves by the screenful that was actually drawn.
///
/// pi pages by `maxVisible` (`session-selector.ts:612-617`), so the property is
/// not "PageDown moves N rows" for some N — it is "PageDown moves as many rows as
/// the list showed". A test with a hardcoded expectation would pass against a
/// constant page size, which is a whole screen on a tall terminal and a fifth of
/// one on a short one.
///
/// The size is read back from a **real frame**: the paint is what measures it, so
/// a test that set the field by hand would be asserting against its own fixture.
#[test]
fn a_page_key_moves_by_the_window_the_list_actually_drew() {
    let (mut app, agent, _receivers) = test_app();
    // Open BEFORE the first frame: `draw_picker` is what measures the window, and
    // a closed picker draws nothing, so painting first leaves the capacity at its
    // initial 1 and the test would measure its own fixture.
    app.picker_open = true;
    app.picker_index = 0;

    // A tall frame and a short one, so the two capacities must differ. If they
    // did not, "pages by the window" would be indistinguishable from "pages by a
    // constant" and this test could not tell the difference.
    let tall = render_rows(&mut app, 100, 44);
    let tall_cap = app.picker_capacity;
    let tall_text: String = tall
        .iter()
        .map(|r| r.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    app.picker_index = 0;
    handle_key(&mut app, &agent, key(KeyCode::PageDown));
    let after_tall = app.picker_index;

    assert!(
        tall_cap > 1,
        "a 44-row frame shows more than one row, so a page is worth having: {tall_cap}"
    );
    assert_eq!(
        after_tall, tall_cap,
        "PageDown moved exactly the drawn window, not a hardcoded number"
    );

    // Now the short frame. The page must shrink with it.
    let short = render_rows(&mut app, 100, 14);
    let short_cap = app.picker_capacity;
    let short_text: String = short
        .iter()
        .map(|r| r.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    app.picker_index = 0;
    handle_key(&mut app, &agent, key(KeyCode::PageDown));
    let after_short = app.picker_index;

    assert!(
        short_cap < tall_cap,
        "a 14-row frame shows fewer rows than a 44-row one ({short_cap} < {tall_cap}), \
         so the page size must follow the frame"
    );
    assert_eq!(
        after_short, short_cap,
        "and the key moved the SHORT window, not the tall one"
    );
    assert!(
        tall_text.len() > short_text.len() && !short_text.is_empty(),
        "the frames really were different sizes ({}, {} bytes)",
        tall_text.len(),
        short_text.len()
    );
}
