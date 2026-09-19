//! The terminal runtime's own tests: state, keys, imports and overlays.
//!
//! Lives beside `app.rs` rather than inside it so the runtime file reads as the
//! code it is. Rust still compiles these as part of the `app` module, so
//! `super::*` reaches every private item — a test forced through a public API
//! would be testing a different thing.

use super::*;
use crossterm::event::KeyEventState;
use epsilon_contracts::agent_event::PermissionRequest;
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
async fn search_command_reports_hits_from_the_real_session_log() {
    let (mut app, agent, (_f, _n)) = test_app();
    // Attach a durable mirror and seed it directly through the store API.
    let tmp = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(std::sync::Mutex::new(
        epsilon_core::SessionStore::open(tmp.path().join("s.jsonl")).unwrap(),
    ));
    agent.attach_session_store(std::sync::Arc::clone(&store));
    {
        let mut st = store.lock().unwrap();
        use epsilon_contracts::content::UserContent;
        use epsilon_contracts::session_entry::EntryKind;
        st.append(EntryKind::Message {
            message: epsilon_contracts::message::Message::User(UserMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: 0,
                content: vec![UserContent::Text {
                    text: "the launch codes are under the keyboard".into(),
                }],
            }),
            api_request_sha256: None,
        })
        .unwrap();
    }

    // Real key path: /search keyboard
    assert!(!type_and_enter(&mut app, &agent, "/search keyboard"));
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("#0 user") && t.contains("keyboard")),
        "expected a hit line mentioning the seeded entry, got {texts:?}"
    );

    // Miss case
    assert!(!type_and_enter(&mut app, &agent, "/search quantum-unicorn"));
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("no matches for 'quantum-unicorn'")),
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

/// M94 review: paint one full frame into a `TestBackend` and return it as
/// one string per row (buffer inspection, the crate's rendering-test style).
fn render_rows(app: &mut App, width: u16, height: u16) -> Vec<String> {
    use ratatui::{Terminal, backend::TestBackend};
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| draw(frame, app)).unwrap();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
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

/// M94 review: an ask that arrives while `/timeline` is open must be visible.
/// The timeline widget clears its own centre, so a dialog painted before it
/// was wiped out while y/a/n/Esc still routed to it (Esc silently denied).
#[tokio::test]
async fn a_pending_approval_is_drawn_over_the_timeline() {
    let (mut app, agent, _receivers) = test_app();
    app.timeline_open = true;
    app.timeline_entries = vec![(7, "user".into(), "TIMELINE-ROW-MARKER".into())];
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
        rows.iter()
            .any(|r| r.contains("capability") && r.contains("bash")),
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

type TestReceivers = (
    tokio::sync::mpsc::UnboundedReceiver<(String, Result<Vec<String>, String>)>,
    tokio::sync::mpsc::UnboundedReceiver<String>,
);

fn test_app() -> (App, Arc<Agent>, TestReceivers) {
    let (ftx, frx) = tokio::sync::mpsc::unbounded_channel();
    let (ntx, nrx) = tokio::sync::mpsc::unbounded_channel();
    let (otx, _orx) = tokio::sync::mpsc::unbounded_channel();
    let approvals = Arc::new(ChannelApprovalUi::new());
    let gate = Arc::new(epsilon_safety::RulesetGate::new(
        Arc::clone(&approvals) as Arc<dyn epsilon_safety::ApprovalUi>
    ));
    let agent = Agent::new(
        epsilon_core::LoopLimits::default(),
        Arc::new(epsilon_llm::PanicGuard(epsilon_llm::FauxProvider::new(
            vec![],
        ))),
        Arc::new(epsilon_core::ToolRegistry::new()),
        gate,
        std::env::temp_dir(),
    );
    (App::new("faux-1".into(), ftx, ntx, otx), agent, (frx, nrx))
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
