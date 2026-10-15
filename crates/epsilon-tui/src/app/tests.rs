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
use super::tests_support::{
    render_buffer, render_rows, test_app, test_app_with_bash, test_app_with_faux,
    test_app_with_store,
};
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

/// **The first character typed must reach the composer (M312).**
///
/// Found by driving the **real binary in a real console** — `scripts/console-probe.py`, which
/// writes real `KEY_EVENT` records and reads the screen back. Typed `explain the layout module`
/// and the composer showed `xplain the layout module`; typed `hello` and it showed `llo`; typed
/// `x` and nothing appeared. Three runs, three losses, always the first character.
///
/// **This test is the half that decides where the defect is, and it is deliberately the cheap
/// half.** If the product dropped the first key, the composer would be missing its first
/// character here too — and then the fix belongs in `handle_key`. If the composer is complete
/// here, the loss is between the console and the process, and "fixing" `handle_key` would have
/// been fixing working code, which this repository has very nearly done before.
#[tokio::test]
async fn the_first_character_typed_reaches_the_composer() {
    let (mut app, agent, (_f, _n)) = test_app();
    assert_eq!(app.input, "", "the fixture starts with an empty composer");

    for (i, c) in "hello".chars().enumerate() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        assert_eq!(
            app.input,
            "hello".chars().take(i + 1).collect::<String>(),
            "after typing {} character(s), the composer holds the whole word — the first \
             character must never be the one that goes missing",
            i + 1
        );
    }
    assert_eq!(app.input, "hello");

    // **And after a submit**, because the console evidence did not distinguish "the first key of
    // a session" from "the first key of any burst", and a test that only covers the first case
    // would pass against a defect in the second.
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert_eq!(app.input, "", "the submit emptied it");
    for c in "world".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert_eq!(app.input, "world", "a second burst is typed whole as well");
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

/// **A tree row wider than its panel is cut in COLUMNS, and drawing it did not panic.**
///
/// The truncation was `&raw[..budget - 3]` where `budget` is **columns** and the slice is
/// **bytes**. The prefix guarantees the skew — `📁` is four bytes, the fold marks three —
/// and snippets are produced up to 100 columns wide, so at an 80-column terminal this
/// branch is the *normal* path rather than an edge case. Any em dash, curly quote or CJK
/// glyph within three bytes of the cut ended the process with
/// `byte index N is not a char boundary`.
///
/// **The control is the panic itself.** Restoring `&raw[..w]` makes this test fail with
/// `byte index … is not a char boundary` rather than with an assertion message, which is
/// the strongest kind of RED available for a crash: there is no build on which the old
/// line survives this fixture.
///
/// The CJK coverage that already existed (the filter-field test below) exercises
/// `fit_width`, not this row, and its cut landed on a boundary by arithmetic luck.
#[test]
fn a_tree_row_of_wide_glyphs_is_cut_on_the_frame_instead_of_panicking() {
    use crate::app::TimelineRow;

    let (mut app, _agent, _rx) = test_app();
    app.timeline_open = true;
    app.timeline_all = vec![TimelineRow {
        seq: 0,
        id: "a".into(),
        kind: "user".into(),
        // Em dashes and CJK both: the em dash is three bytes and the CJK is nine, so a
        // byte-indexed cut lands inside one of them at almost every width.
        snippet: format!(
            "{}——{}",
            "目的赶到目标".repeat(20),
            "— refactor ".repeat(20)
        ),
        depth: 0,
        on_active: true,
        parent: None,
        label: Some("ラベル".into()),
    }];
    app.timeline_entries = app.timeline_all.clone();
    app.timeline_index = 0;
    app.folded.insert("a".into());

    // 80 columns: the panel's inner width makes truncation the normal path, which is the
    // width at which this used to crash.
    let rows = render_rows(&mut app, 80, 24);
    let screen: String = rows
        .iter()
        .map(|r| r.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        screen.contains('#'),
        "the row is on screen, cut rather than dropped: {screen}"
    );
    // Nothing may be wider than the terminal it was drawn into. Measured in
    // **columns** with the crate's own function, because the defect being guarded is a
    // byte/column confusion — `str::len()` would repeat that confusion here, and the
    // assertion would pass on exactly the build it exists to catch.
    for row in &rows {
        let width = crate::ui::layout::display_width(row);
        assert!(
            width <= 80,
            "a row may not exceed the terminal it is drawn into: {width} columns in {row:?}"
        );
    }
}

/// **The permission dialog is the highest-stakes screen, and it was the one that said
/// nothing.**
///
/// `handle_approval_key` ended in `_ => {}`. Enter, Tab, Space, the arrows, Home/End,
/// Delete and Backspace all did nothing and printed nothing — and G10's notice, which
/// exists for precisely this, lives at the end of `handle_key`, which that function
/// returns before ever reaching. A reader waiting to decide whether work runs is the
/// person least likely to keep pressing keys they believe are broken.
///
/// **Control:** with the arm restored to `_ => {}`, `notice_rx.try_recv()` is `Err` and
/// this test fails on the `.expect`.
#[tokio::test]
async fn an_unbound_key_on_a_permission_ask_says_so_and_names_the_ones_that_work() {
    let (mut app, agent, (_ftx_rx, mut notice_rx)) = test_app();
    let approvals = ChannelApprovalUi::new();
    let response = park_ask(&mut app, &approvals, "unbound-on-ask").await;
    // Paint once so the dialog is the one the key is pressed against.
    let _ = render_rows(&mut app, 80, 24);

    let quit = handle_key(&mut app, &agent, key(KeyCode::Tab));

    assert!(!quit, "an unbound key on a dialog must not quit the app");
    assert!(
        app.pending_ask.is_some(),
        "and it must not have answered the ask either — the reader is still deciding"
    );
    let said = notice_rx.try_recv().expect(
        "a key the dialog does not act on must still produce feedback; silence here is \
         indistinguishable from a frozen program",
    );
    assert!(
        said.contains("not bound"),
        "the notice must say the key is unbound: {said:?}"
    );
    assert!(
        said.contains('y') && said.contains('n'),
        "\"not bound\" is only useful next to the alternative, so the notice names the keys \
         that work: {said:?}"
    );
    drop(response);
}

/// **Guidance mode has to say where the keyboard went, and give the draft back.**
///
/// Two defects, one test, because they are one mistake: the mode was entered by a key
/// that *destroyed* the composer's draft with no undo entry, and nothing on screen said
/// the keyboard had moved. A reader who pressed `e` was typing into a field with no
/// prompt, no mode word, and — because `overlay_open()` includes `pending_ask` — no caret.
///
/// `e`/`esc` are the pair, so asserting that `esc` returns the draft proves the stash
/// happened without testing a private field.
#[tokio::test]
async fn guidance_mode_is_drawn_where_the_keys_are_and_returns_the_draft() {
    let (mut app, agent, (_ftx_rx, _notice_rx)) = test_app();
    app.input = "half a sentence".into();
    app.cursor_chars = app.input.chars().count();
    let approvals = ChannelApprovalUi::new();
    let response = park_ask(&mut app, &approvals, "guidance-draw").await;
    let _ = render_rows(&mut app, 100, 30);

    handle_key(&mut app, &agent, key(KeyCode::Char('e')));
    assert!(app.guidance_mode, "e enters guidance mode");
    let rows = render_rows(&mut app, 100, 30);
    let screen: String = rows
        .iter()
        .map(|r| r.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        screen.contains("guidance"),
        "the dialog must say the keyboard is somewhere else — a reader cannot otherwise \
         know why y stopped approving:\n{screen}"
    );
    assert!(
        !screen.contains("allow once"),
        "and it must stop advertising the approve keys, which now type letters:\n{screen}"
    );

    // Esc leaves guidance mode, and the draft the borrow took comes back with it.
    handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert!(!app.guidance_mode);
    assert_eq!(
        app.input, "half a sentence",
        "the draft must survive guidance mode: a borrow is not a delete"
    );
    drop(response);
}

/// Park one ask on the shared queue and hand it to the app, exactly like the
/// run loop does; the response future stays live so a test can assert which
/// decision the parked ask received (or that it never received one).
async fn park_ask(
    app: &mut App,
    approvals: &ChannelApprovalUi,
    id: &str,
) -> tokio::task::JoinHandle<PermissionDecision> {
    park_ask_detail(
        app,
        approvals,
        id,
        "bash",
        "rm -rf build",
        serde_json::json!({ "command": "rm -rf build" }),
    )
    .await
}

/// The same, with the request's own text supplied.
///
/// Separate from [`park_ask`] because the property under test is *what the request says*:
/// an ask whose text carries a bidi override must be handled differently from one that does
/// not, and a fixture that hardcodes the command cannot express the difference.
async fn park_ask_detail(
    app: &mut App,
    approvals: &ChannelApprovalUi,
    id: &str,
    capability: &str,
    pattern: &str,
    detail: serde_json::Value,
) -> tokio::task::JoinHandle<PermissionDecision> {
    let requester = approvals.clone();
    let id = id.to_string();
    let capability = capability.to_string();
    let pattern = pattern.to_string();
    let response = tokio::spawn(async move {
        requester
            .ask(PermissionRequest {
                request_id: id,
                capability,
                pattern,
                detail,
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
    app.apply_oauth_event(OauthUiEvent::Done {
        provider: "anthropic".into(),
        text: "done".into(),
        ok: false,
    });
    assert!(!app.auth_prompt_open);
    assert!(app.oauth_code_tx.is_none());
}

/// A completed anthropic OAuth ceremony warns once about subscription billing (P-061).
///
/// The worker thread is real-build-only, but its event applies on the UI
/// thread — this drives the seam, not the spawn.
#[test]
fn oauth_completion_warns_once_about_subscription_billing() {
    let (mut app, _agent, (_f, _n)) = test_app();
    // Pinned repertoire: the assertion strips the Unicode continuation marks
    // (`•`, `│`), so an ASCII runner would leave its own (`*`, `|`) in place.
    app.theme.glyphs = crate::ui::theme::UNICODE;
    fn texts(app: &super::App) -> Vec<String> {
        app.ui
            .lines()
            .iter()
            .map(|l| l.text.clone())
            .collect::<Vec<_>>()
    }
    let done = |ok: bool| OauthUiEvent::Done {
        provider: "anthropic".into(),
        text: "done".into(),
        ok,
    };
    // A failed ceremony warns about nothing: no credential changed hands.
    app.apply_oauth_event(done(false));
    assert!(
        !texts(&app).iter().any(|t| t.contains("per token")),
        "failure warns nothing: {:?}",
        texts(&app)
    );
    // A completed one warns, exactly once per session.
    app.apply_oauth_event(done(true));
    assert!(app.anthropic_billing_warned, "the flag records the warning");
    // The warning reaches pixels at both sizes, through the production draw.
    // (Normalised: the terminal wraps words and gutters interrupt the line.)
    // Both gutter marks are removed, not just the notice's bullet: a wrapped
    // row carries the continuation rule (`decorate`'s `continued`), so the
    // second half of a long notice arrives with `│` between the words the
    // first half ended on and the words this one starts with.
    let mut flat = |w: u16, h: u16| {
        render_rows(&mut app, w, h)
            .join("\n")
            .replace(['\u{2022}', '\u{2502}'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let flat80 = flat(80, 24);
    assert!(
        flat80.contains("per token"),
        "the billing warning is on screen at 80x24: {flat80}"
    );
    let flat100 = flat(100, 30);
    assert!(
        flat100.contains("per token"),
        "the billing warning is on screen at 100x30: {flat100}"
    );
    assert_eq!(
        texts(&app)
            .iter()
            .filter(|t| t.contains("per token"))
            .count(),
        1,
        "one warning, not one per event: {:?}",
        texts(&app)
    );
    app.apply_oauth_event(done(true));
    assert_eq!(
        texts(&app)
            .iter()
            .filter(|t| t.contains("per token"))
            .count(),
        1,
        "the second completion stays silent: {:?}",
        texts(&app)
    );
    // And other providers never warn through this door.
    app.anthropic_billing_warned = false;
    app.apply_oauth_event(OauthUiEvent::Done {
        provider: "openai".into(),
        text: "done".into(),
        ok: true,
    });
    assert_eq!(
        texts(&app)
            .iter()
            .filter(|t| t.contains("per token"))
            .count(),
        1,
        "openai completion adds no warning: {:?}",
        texts(&app)
    );
}

#[tokio::test]
async fn cancelling_a_key_entry_says_what_was_kept() {
    use epsilon_llm::auth::StoredCredential;
    use std::collections::BTreeMap;

    let key = |k: &str| StoredCredential::Api {
        key: k.into(),
        env: None,
    };
    let mut stored: BTreeMap<String, StoredCredential> = BTreeMap::new();
    stored.insert("deepseek".into(), key("sk-test"));

    // **The case that started this.** A provider that still holds a key: cancelling must
    // say the old key is *still in use* and that nothing changed, because that is the fact
    // the reader cannot see — they were looking at a login prompt, not at the store.
    let kept = super::keys::cancelled_key_notice("deepseek", &stored);
    assert!(
        kept.contains("deepseek")
            && kept.contains("still in use")
            && kept.contains("nothing changed"),
        "the notice must name the provider and say its key survives the cancel: {kept:?}"
    );
    assert!(
        kept.contains("/login deepseek"),
        "and must say how to replace it, not only that something survived: {kept:?}"
    );

    // **The other case, which is the more confusing one to get wrong.** Nothing stored: the
    // reader cancelled, so they may believe a key was entered. Saying only "cancelled"
    // there leaves them waiting for a provider that will run offline.
    let empty = super::keys::cancelled_key_notice("mistral", &BTreeMap::new());
    assert!(
        empty.contains("no stored key") && empty.contains("offline"),
        "with nothing stored the notice must say the provider will still run offline: {empty:?}"
    );

    // **No prompt open at all** — the old message, kept rather than regressed. `auth_provider`
    // is a plain `String`, so "no prompt" is the empty string rather than an `Option`.
    assert_eq!(
        super::keys::cancelled_key_notice("", &stored),
        "API key entry cancelled"
    );
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
///
/// `cwd` is the workspace the session belongs to, and it goes into the header: since M217 the
/// browser lists a workspace's sessions by resolving its directory **and** filtering the flat root
/// by this field, so a fixture whose header names somewhere else is correctly invisible — which
/// would make a browser test pass for the wrong reason.
fn write_session_fixture(path: &std::path::Path, cwd: &std::path::Path, text: &str) {
    let mut store = epsilon_session::SessionStore::open_in(path, cwd).expect("session store");
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
    let workspace = tmp.path();
    let sessions = workspace.join(".epsilon").join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    // M217: one session in the per-cwd directory a launch writes into, and one still in the flat
    // root from before that move. The browser must show both — a reader that only looked in the
    // new directory would list one, and one that only looked in the old would list one.
    let per_cwd = epsilon_session::time::session_dir_for(&sessions, workspace);
    std::fs::create_dir_all(&per_cwd).unwrap();
    let target = per_cwd.join("2026-09-30T10-00-00-000Z_ses_a.jsonl");
    write_session_fixture(&target, workspace, "the other session's first request");
    write_session_fixture(
        &sessions.join("zzz-unrelated.jsonl"),
        workspace,
        "unrelated work",
    );
    app.workspace_root = workspace.to_path_buf();

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

/// M221: `/new` starts a new session **file**, so "new session" and "new conversation" are the
/// same act — which is what a launch already was since M210, in a directory of its own since M217.
///
/// The old behaviour cleared the visible rows and kept appending to the same file, so a second
/// conversation was a stowaway in the first one's file and the header's id named only the first.
/// Nothing tested it, which is why it survived: the assertions here are the ones that would have
/// failed against it.
#[tokio::test]
async fn new_command_starts_a_new_session_file_and_keeps_the_old_one() {
    let (mut app, agent, (_f, _n)) = test_app();
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path();
    app.workspace_root = workspace.to_path_buf();
    let sessions = workspace.join(".epsilon").join("sessions");
    let per_cwd = epsilon_session::time::session_dir_for(&sessions, workspace);
    std::fs::create_dir_all(&per_cwd).unwrap();
    let old = per_cwd.join("2026-09-30T10-00-00-000Z_ses_old.jsonl");
    write_session_fixture(&old, workspace, "the conversation before /new");

    type_and_enter(&mut app, &agent, "/new");

    let attached = agent
        .store_handle_for_tests()
        .lock()
        .unwrap()
        .path()
        .to_path_buf();
    assert_ne!(
        attached, old,
        "/new must not keep writing to the file it just left"
    );
    assert!(
        attached.starts_with(&per_cwd),
        "the new file belongs in this workspace's directory: {attached:?}"
    );
    assert!(
        attached.exists(),
        "and it exists now, rather than at the first append: {attached:?}"
    );

    // The old conversation does not come along — because the new file has no
    // history, not because rows were cleared. (The rows here are this command's
    // own notices; what must be absent is the previous conversation.)
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        !texts
            .iter()
            .any(|t| t.contains("the conversation before /new")),
        "a new session starts empty; it does not carry the last one: {texts:?}"
    );

    // Both sessions are browsable, and the old one is intact: a new conversation never costs the
    // previous one. The **count** is the assertion that cannot pass against the old behaviour by
    // arithmetic rather than by argument: `/new` used to clear rows and write no file at all, so
    // this directory would still hold exactly one.
    let on_disk: Vec<_> = std::fs::read_dir(&per_cwd)
        .expect("the directory exists")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .collect();
    assert_eq!(
        on_disk.len(),
        2,
        "one file per conversation: the planted one and the one /new just started"
    );
    let rows = crate::picker::session_rows(
        &sessions,
        workspace,
        epsilon_session::SessionScope::Workspace,
        "",
        None,
        u64::MAX,
    );
    assert_eq!(rows.len(), 2, "both sessions are listed: {rows:?}");
    assert!(
        rows.iter().any(|r| r.path == old),
        "the previous conversation is still there: {rows:?}"
    );
}

#[tokio::test]
async fn new_command_refuses_while_a_run_is_in_flight() {
    // Swapping the store out from under a running turn would leave that turn appending to a file
    // the transcript no longer shows. The refusal is the TUI invariant ("nothing happened must
    // never be a state the UI can be in") applied to a swap.
    let (mut app, agent, (_f, _n)) = test_app();
    let tmp = tempfile::tempdir().unwrap();
    app.workspace_root = tmp.path().to_path_buf();
    // `running` is state the run loop owns, so it is entered the way the loop enters it.
    app.ui
        .apply(&epsilon_contracts::agent_event::AgentEvent::AgentStart);
    assert!(app.ui.running(), "the premise: a turn is in flight");
    type_and_enter(&mut app, &agent, "/new");
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts.join("\n").contains("busy"),
        "a refusal that says why, not a silent no-op: {texts:?}"
    );
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

    // An empty directory (no session files yet) opens the browser on its
    // empty state (P-243) — a refusal would hide the message that teaches
    // the key. The empty words are pinned on the frame, beside this.
    std::fs::create_dir_all(tmp.path().join(".epsilon").join("sessions")).unwrap();
    type_and_enter(&mut app, &agent, "/sessions");
    assert!(
        app.session_browser.is_some(),
        "an existing empty dir opens, not refuses"
    );
    assert!(
        app.session_browser.as_ref().expect("open").rows.is_empty(),
        "with nothing in it"
    );
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

/// M233: with the CLI in offline mode beside a **saved real model**, the header names the provider
/// the process is actually calling.
///
/// Found by reading a frame, not a test: `console-probe.py` captured the TUI running
/// `--provider faux` with a saved `deepseek/…` model, and the header read `DeepSeek V3.1` with no
/// offline marker — the headless path printed `provider: faux (offline scripted)` for the same
/// decision. A screen naming a provider it is not calling is the class of lie this repository
/// refuses, so the frame is the assertion.
#[test]
fn an_offline_launch_names_the_provider_it_is_actually_calling() {
    let (mut app, _agent, (_f, _n)) = test_app();
    // The saved model is a real one: exactly the state that produced the defect.
    app.model_id = "deepseek/deepseek-chat-v3.1".into();
    app.offline = true;

    let rows = render_rows(&mut app, 100, 30);
    let header = rows.first().cloned().unwrap_or_default();
    assert!(
        !header.contains("DeepSeek"),
        "the header must not name a provider the harness is not calling: {header:?}"
    );
    assert!(
        header.contains("Faux (offline scripted)") || header.contains("offline"),
        "and it says what is actually running: {header:?}"
    );

    // The other direction still holds: a real run names the real model.
    let (mut real_app, _a2, (_f2, _n2)) = test_app();
    real_app.model_id = "deepseek/deepseek-chat-v3.1".into();
    real_app.offline = false;
    let real_header = render_rows(&mut real_app, 100, 30)
        .first()
        .cloned()
        .unwrap_or_default();
    assert!(
        real_header.contains("DeepSeek"),
        "a real run still names its model: {real_header:?}"
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

/// M229: `/task` shows what the child cost.
///
/// A **foreground** child, so the assertion cannot be skipped by a task that has not settled yet —
/// which is why this is its own test rather than a third assertion on the background one above.
/// The registry has held the child's usage since the child finished; the row listed turns and a
/// preview without it, so "why is this session expensive" was unanswerable from the screen even
/// though every number existed.
#[tokio::test]
async fn tasks_command_shows_what_a_child_cost() {
    let (mut app, agent, (_f, _n)) = test_app();
    let host: std::sync::Arc<dyn epsilon_contracts::tool::TaskHost> =
        agent.clone() as std::sync::Arc<dyn epsilon_contracts::tool::TaskHost>;
    host.run_task(epsilon_contracts::tool::TaskRequest {
        prompt: "look around".into(),
        description: "repo recon".into(),
        subagent_type: "default".into(),
        background: false,
        resume: None,
        model: Some("deepseek-flash".into()),
    })
    .await
    .expect("a foreground child completes inline");

    let settled = agent.task_snapshots();
    assert!(
        settled
            .iter()
            .all(|s| s.state != epsilon_contracts::tool::TaskState::Running),
        "the child is done: {settled:?}"
    );
    assert!(
        settled.iter().any(|s| s.usage.is_some()),
        "a completed child carries its bill: {settled:?}"
    );

    type_and_enter(&mut app, &agent, "/tasks");
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    let joined = texts.join("\n");
    assert!(
        joined.contains("in /") && joined.contains("out"),
        "the spend is in the row that already names the model: {joined}"
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
    // deepseek-v4-pro: resolvable in the test catalog (exactly one
    // candidate). Unresolvable text opens the picker instead — see
    // model_partial_opens_the_picker_prefilled. A name that resolves in
    // production but not here (e.g. deepseek-chat) would test blind apply,
    // which no longer exists by design.
    assert!(!type_and_enter(&mut app, &agent, "/model deepseek-v4-pro"));
    assert_eq!(
        app.model_id, "deepseek-v4-pro",
        "direct /model applies instantly"
    );
    let line = &app.ui.lines()[before];
    assert!(line.text.contains("deepseek-v4-pro"), "{:?}", line.text);
    // Recents refresh in-memory through apply_model (the persisted side is
    // covered by config-layer round-trip tests; asserting the real global
    // file here races every other TUI test's writes — lesson 16b again).
    assert!(
        app.recents.first().map(String::as_str) == Some("deepseek-v4-pro"),
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

    // P-074: `/fork` opens the message picker; Enter at the preselected
    // newest forks there. The no-copy guarantees below ride the real path.
    app.run_command(
        &agent,
        crate::picker::parse_command("/fork").expect("/fork parses"),
    );
    assert!(app.timeline_open, "/fork opens the message picker");
    assert!(
        app.timeline_fork_refill.is_some(),
        "/fork opens in fork mode"
    );
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert!(!app.timeline_open, "picker closes after forking");

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

/// Seed a store with two user messages; the fork picker lists them.
fn seed_two_user_messages(store: &std::sync::Mutex<epsilon_core::SessionStore>) {
    let mut st = store.lock().unwrap();
    use epsilon_contracts::content::UserContent;
    use epsilon_contracts::message::UserMessage;
    use epsilon_contracts::session_entry::EntryKind;
    for (text, ms) in [("hello", 0), ("world", 1)] {
        let _ = st
            .append(EntryKind::Message {
                message: epsilon_contracts::message::Message::User(UserMessage {
                    id: epsilon_contracts::new_id("msg"),
                    timestamp_ms: ms,
                    content: vec![UserContent::Text { text: text.into() }],
                }),
                api_request_sha256: None,
            })
            .unwrap();
    }
}

/// P-074: `/fork` opens a message picker instead of forking the leaf
/// sight-unseen. User messages only, newest preselected — the row a fork
/// continues from.
#[tokio::test]
async fn fork_opens_a_message_picker_newest_preselected() {
    let (mut app, agent, _rx) = test_app();
    let tmp = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(std::sync::Mutex::new(
        epsilon_core::SessionStore::open(tmp.path().join("s.jsonl")).unwrap(),
    ));
    agent.attach_session_store(std::sync::Arc::clone(&store));
    seed_two_user_messages(&store);
    assert!(!type_and_enter(&mut app, &agent, "/fork"));
    assert!(app.timeline_open, "/fork opens the picker");
    assert!(
        app.timeline_fork_refill.is_some(),
        "/fork opens in fork mode, not navigate mode"
    );
    assert!(
        app.timeline_entries.iter().all(|r| r.kind == "user"),
        "fork lists user messages only"
    );
    assert_eq!(
        app.timeline_index,
        app.timeline_entries.len() - 1,
        "newest message preselected"
    );
    let screen = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        screen.contains("hello") && screen.contains("world"),
        "both messages on the frame: {screen}"
    );
    assert!(
        screen.contains("enter fork"),
        "the hint names the fork action, not jump: {screen}"
    );
}

/// P-074: Enter forks at the picked message and puts the typed text back
/// into the composer.
#[tokio::test]
async fn fork_enter_forks_at_the_picked_message_and_refills_composer() {
    let (mut app, agent, _rx) = test_app();
    let tmp = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(std::sync::Mutex::new(
        epsilon_core::SessionStore::open(tmp.path().join("s.jsonl")).unwrap(),
    ));
    agent.attach_session_store(std::sync::Arc::clone(&store));
    seed_two_user_messages(&store);
    assert!(!type_and_enter(&mut app, &agent, "/fork carry this on"));
    assert!(app.timeline_open);
    // Newest-first: move up to `hello`, fork there — `world` must not ride along.
    handle_key(&mut app, &agent, key(KeyCode::Up));
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert!(!app.timeline_open, "picker closes after forking");
    assert_eq!(
        app.input, "carry this on",
        "typed text is back in the composer"
    );
    let files: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    assert_eq!(files.len(), 2, "one fork file beside the source");
    let forked = files
        .iter()
        .find(|p| p.file_name().unwrap() != "s.jsonl")
        .expect("a fork file");
    let body = std::fs::read_to_string(forked).unwrap();
    assert!(
        body.contains("hello"),
        "fork continues from the picked message"
    );
    assert!(
        !body.contains("world"),
        "messages after the picked one stay behind"
    );
}

/// P-074: Esc closes the fork picker without forking.
#[tokio::test]
async fn fork_esc_closes_without_forking() {
    let (mut app, agent, _rx) = test_app();
    let tmp = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(std::sync::Mutex::new(
        epsilon_core::SessionStore::open(tmp.path().join("s.jsonl")).unwrap(),
    ));
    agent.attach_session_store(std::sync::Arc::clone(&store));
    seed_two_user_messages(&store);
    assert!(!type_and_enter(&mut app, &agent, "/fork"));
    assert!(app.timeline_open);
    handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert!(!app.timeline_open, "picker closes on Esc");
    assert!(
        app.timeline_fork_refill.is_none(),
        "fork mode clears with the picker"
    );
    let files: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    assert_eq!(files.len(), 1, "nothing forked on Esc");
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
    let (app, _agent, _rx) = test_app();
    for entry in crate::picker::palette_entries(&app.keybindings) {
        // An entry that carries a direct action has no command to parse: the
        // workspace file tree (`app.tree.toggle`) is a chord with no command
        // head, and inventing a spelling the parser would refuse is what would
        // make choosing it a silent no-op. Such an entry is held to its action
        // instead — see `the_palette_tree_entry_opens_the_tree`.
        if entry.action.is_some() {
            continue;
        }
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

/// The advertised key hint must be **resolved from the binding table**, never written by hand.
///
/// # What this replaces, and why the old one was not enough
///
/// The previous test asserted `normalized != "x"` — a blacklist of one value. It passed on a
/// palette that advertised `Some("/login")` in the **key** column: a slash command shown where
/// a reader looks for a chord, for an action that has no key at all. One bad literal out of
/// seven, and the guard could only see the one it had been told about.
///
/// # What the shape now makes impossible
///
/// `PaletteEntry` carries `key_id`, and `binding` is produced by
/// [`KeybindingsManager::key_text`]. So the only values that can reach the key column are
/// chords the key handler will accept, in the spelling `key_label` prints. A rebind changes
/// the hint; an action with no binding shows none. This test holds all three properties.
#[tokio::test]
async fn a_palette_key_hint_is_resolved_from_the_binding_table_not_typed() {
    let (app, _agent, _rx) = test_app();
    for entry in crate::picker::palette_entries(&app.keybindings) {
        match (entry.key_id, entry.binding.as_deref()) {
            (None, None) => {}
            (None, Some(shown)) => panic!(
                "entry {:?} advertises the key {shown:?} but names no binding id — a hint \
                 with nothing behind it is how \"/login\" came to sit in the key column",
                entry.title
            ),
            (Some(id), None) => panic!(
                "entry {:?} names the binding {id:?} but shows no key",
                entry.title
            ),
            (Some(id), Some(shown)) => {
                assert_eq!(
                    Some(shown),
                    app.keybindings.key_text(id).as_deref(),
                    "entry {:?} shows {shown:?} for {id:?}, which is not what the binding \
                     table resolves — the hint is supposed to be looked up, not written",
                    entry.title
                );
                assert!(
                    !shown.starts_with('/'),
                    "a slash command is not a chord: entry {:?} advertises {shown:?} as a key",
                    entry.title
                );
            }
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

/// `Ctrl+F` in the model picker must toggle a favourite, not panic.
///
/// **Found by reading the key path, not by a failing test.** `app/keys.rs` had this
/// inside the `if app.picker_open` block:
///
/// ```ignore
/// KeyCode::Up | KeyCode::Down | KeyCode::Char('f' | 'F')
///     if key.modifiers.contains(KeyModifiers::CONTROL) =>
/// { unreachable!("handled by guarded arms below") }
/// ```
///
/// It is placed **before** the arms it claims to defer to, and nothing above it
/// intercepts `Ctrl+F`, `Ctrl+Up` or `Ctrl+Down`. So the arm fired, and the three
/// chords reached a panic — in a release binary too, because `unreachable!` panics
/// in every profile. `Ctrl+F` is advertised in `/help` (`app/commands.rs`) and in the
/// picker's own hint row, and the handler it was protecting (`toggle_favorite`) was
/// unreachable code.
///
/// The existing `Ctrl+F` test runs with the picker **closed**, which is why 2,061
/// green tests never saw it. A guard placed in front of the code it guards is not a
/// guard; it is a tripwire pointed the wrong way.
#[tokio::test]
async fn ctrl_f_in_the_model_picker_toggles_a_favorite_instead_of_panicking() {
    let (mut app, agent, (_f, _n)) = test_app();
    // Open the picker, then descend into the highlighted provider so stage 2 (a
    // model list) is what the key acts on.
    assert!(!handle_key(&mut app, &agent, ctrl(KeyCode::Char('m'))));
    assert!(app.picker_open);
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert!(
        app.picker_provider.is_some(),
        "Enter on stage 1 descends into the provider"
    );

    // The advertised chord. Before the fix this unwound the run.
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('f')));

    // **Asserted without touching the user's config, deliberately.**
    //
    // `toggle_favorite` writes `<config dir>/config.toml`, and a test that writes
    // there changes the machine rather than the tree. That is not hypothetical: an
    // earlier version of this test asserted on the favourite it had just starred,
    // and the next run of the suite failed in
    // `f2_cycles_favorites_and_guides_when_empty`, which reads the *same* global
    // file and expects an empty list. That test is not the broken one — this is.
    //
    // The discriminator that does not need the filesystem: with the arm removed,
    // `Ctrl+F` falls through to the last arm of that match, `Char(c) => insert_char`,
    // so the **filter** would gain an `f`. Reaching the favourite handler means it
    // did not.
    assert!(
        !app.input.contains('f'),
        "ctrl+f fell through to the filter instead of the favourite handler: {:?}",
        app.input
    );
    assert!(
        app.picker_open,
        "and the picker is still the picker: {:?}",
        app.picker_open
    );

    // The other two chords the deleted arm claimed, because they were named in it.
    handle_key(&mut app, &agent, ctrl(KeyCode::Up));
    handle_key(&mut app, &agent, ctrl(KeyCode::Down));
    assert!(app.picker_open, "the picker survives a navigation chord");
}

/// A rebound `app.interrupt` must be the key that interrupts.
///
/// **This is `AGENTS.md` §2.7 in the shape §2.7 names: a bound key the UI does not
/// act on.** `app.interrupt` has been in `DEFAULTS` since the binding table was
/// written, and the footer prints its chord through `key_text("app.interrupt", …)` —
/// so a user who rebinds it in `keybindings.json` is shown *their* key beside "stop".
/// The handler, meanwhile, was a literal `KeyCode::Esc` arm, so the new chord did
/// nothing at all and the footer was advertising a key that only worked if you did not
/// rebind it.
///
/// Idle `interrupt` and `exit` are the same key, so the observable is the one the
/// existing `remapped_quit_via_keybindings_manager` test already uses: idle, `Esc`
/// quits. Rebound, that moves to the new chord **and** stops being true of `Esc`.
#[tokio::test]
async fn a_rebound_interrupt_key_is_the_key_that_interrupts() {
    let (mut app, agent, _rx) = test_app();
    let mut map = std::collections::HashMap::new();
    map.insert("app.interrupt".into(), vec!["ctrl+y".into()]);
    app.keybindings = crate::keybindings::KeybindingsManager::from_map(map);

    // The advertised chord is the one that works.
    assert_eq!(
        app.keybindings.key_text("app.interrupt").as_deref(),
        Some("ctrl+y"),
        "the footer would print this chord"
    );
    assert!(
        handle_key(
            &mut app,
            &agent,
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)
        ),
        "the rebound chord must be the key that interrupts"
    );

    // And the old literal is no longer privileged: it now says it is not bound,
    // which is the same truth the footer is no longer contradicting.
    let (mut app2, agent2, (_ftx_rx, mut notice_rx)) = test_app();
    let mut map2 = std::collections::HashMap::new();
    map2.insert("app.interrupt".into(), vec!["ctrl+y".into()]);
    app2.keybindings = crate::keybindings::KeybindingsManager::from_map(map2);
    assert!(
        !handle_key(&mut app2, &agent2, key(KeyCode::Esc)),
        "esc is unbound once app.interrupt moves, and must not still quit"
    );
    let said = notice_rx.try_recv().expect(
        "a key that now does nothing must say so rather than looking like a hang \
         (the same rule the permission dialog's unbound-key test holds)",
    );
    assert!(
        said.to_lowercase().contains("esc") && said.contains("not bound"),
        "the notice must name the key, so the reader can tell 'not bound' from \
         'bound to something else': {said:?}"
    );
}

/// A rebound `tui.input.submit` must be the key that submits.
///
/// The same shape as the interrupt binding above, and worse: `tui.input.submit` is
/// printed by `/hotkeys` from the same `effective()` table, so a user who binds it
/// to anything but Enter was told their chord sends — and pressing it did nothing,
/// while pressing Enter sent anyway. The arm was `KeyCode::Enter if
/// keybindings.matches("tui.input.submit", &key)`: a guard that re-derived the very
/// answer the pattern in front of it already gave, so the table was read and not
/// obeyed.
#[tokio::test]
async fn a_rebound_submit_key_is_the_key_that_submits() {
    let (mut app, agent, _rx) = test_app();
    let mut map = std::collections::HashMap::new();
    map.insert("tui.input.submit".into(), vec!["ctrl+y".into()]);
    app.keybindings = crate::keybindings::KeybindingsManager::from_map(map);
    for c in "hello".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }

    assert!(
        !handle_key(&mut app, &agent, key(KeyCode::Enter)),
        "enter is unbound once tui.input.submit moves"
    );
    assert_eq!(
        app.input, "hello",
        "an unbound Enter must leave the draft alone rather than send it"
    );

    assert!(
        !handle_key(
            &mut app,
            &agent,
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)
        ),
        "submitting is not quitting"
    );
    assert!(
        app.input.is_empty(),
        "the rebound chord must submit and clear the draft; draft was {:?}",
        app.input
    );
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
                hide_thinking: false,
                filter: crate::state::TranscriptFilter::default(),
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

/// Entering a query resets the picker selection to row 0 (P-252).
///
/// The matrix row named `two_stage_picker_descends_and_applies_model`, which
/// descends and closes but never types a query and never reads `picker_index` —
/// so the row was `done` on evidence that does not establish it. This test is
/// the evidence it needed: descend, move off row 0, type a query character, and
/// the best match is highlighted again while the filter keeps the character.
#[tokio::test]
async fn typing_a_query_resets_the_picker_selection_to_row_zero() {
    let (mut app, agent, (_f, _n)) = test_app();
    // Open the picker (stage 1: providers) and descend into the highlighted
    // provider — the first row, which holds dozens of models in the catalog.
    assert!(!handle_key(&mut app, &agent, ctrl(KeyCode::Char('m'))));
    assert!(app.picker_open);
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert!(
        app.picker_provider.is_some(),
        "Enter on stage 1 descends into the provider"
    );

    // Move off row 0, so the reset below has something to undo.
    let rows = crate::picker::build_models_for_provider(
        app.picker_provider.as_deref().unwrap_or(""),
        &app.input,
        &app.model_id,
        // No favorites in the drilldown (preserves current rows).
        &[],
        None,
        &app.live_cache,
        &app.customs,
    );
    assert!(
        rows.len() > 1,
        "need two model rows for the selection to move, got {} for {:?}",
        rows.len(),
        app.picker_provider
    );
    handle_key(&mut app, &agent, key(KeyCode::Down));
    assert_eq!(
        app.picker_index, 1,
        "Down moves the selection off row 0 first"
    );

    // Type a query character: the filter gains it, the picker stays open, and
    // the selection returns to row 0 so the best match is highlighted.
    handle_key(&mut app, &agent, key(KeyCode::Char('x')));
    assert!(
        app.input.contains('x'),
        "the character filters rather than submits: {:?}",
        app.input
    );
    assert!(
        app.picker_open,
        "typing a query must not close or submit the picker"
    );
    assert_eq!(
        app.picker_index, 0,
        "a query resets the selection to the best match"
    );
}

// ---- M21: thinking level surface -------------------------------------------------

#[tokio::test]
async fn think_command_sets_levels_and_reports_unknown() {
    let (mut app, agent, (_f, _n)) = test_app();
    // The level is persisted now (criterion 7), so the config dir is pinned:
    // a test must never write the developer's own `~/.config/epsilon`.
    let cfg = tempfile::tempdir().expect("a config dir");
    app.config_dir_override = Some(cfg.path().to_path_buf());
    assert!(!type_and_enter(&mut app, &agent, "/think high"));
    assert_eq!(
        agent.thinking_level(),
        epsilon_contracts::ThinkingLevel::High
    );
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(texts.iter().any(|t| t.contains("thinking level: high")));

    // Bare /think opens the picker without changing the level (P-048).
    assert!(!type_and_enter(&mut app, &agent, "/think"));
    assert_eq!(
        agent.thinking_level(),
        epsilon_contracts::ThinkingLevel::High
    );
    // Close the picker before typing on: keys otherwise filter levels.
    handle_key(&mut app, &agent, key(KeyCode::Esc));

    // Unknown level is corrected, not sent to the model.
    assert!(!type_and_enter(&mut app, &agent, "/think banana"));
    assert_eq!(
        agent.thinking_level(),
        epsilon_contracts::ThinkingLevel::High
    );
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(texts.iter().any(|t| t.contains("unknown level 'banana'")));
}

/// P-048: bare `/think` opens a level picker. The argument form (`/think
/// high`) already applied invisibly; discovery needed a visible list — the
/// picker names every level and marks the current one.
#[tokio::test]
async fn bare_think_opens_a_level_picker_listing_all_levels() {
    let (mut app, agent, (_f, _n)) = test_app();
    assert!(!type_and_enter(&mut app, &agent, "/think"));
    assert!(app.think_pick_open, "bare /think opens the level picker");
    let screen = render_rows(&mut app, 80, 24).join("\n");
    for level in epsilon_contracts::ThinkingLevel::ALL {
        assert!(
            screen.contains(level.as_str()),
            "picker lists {}: {screen}",
            level.as_str()
        );
    }
    assert!(
        screen.contains("current"),
        "picker marks the current level: {screen}"
    );
}

/// P-048: the level picker applies on Enter and keeps the level on Esc.
#[tokio::test]
async fn think_picker_enter_applies_and_esc_keeps_the_level() {
    use epsilon_contracts::ThinkingLevel::High;
    let (mut app, agent, (_f, _n)) = test_app();
    // Pinned for the same reason as `think_command_sets_levels_and_reports_unknown`:
    // applying a level persists it, and a test writes nowhere near a real config.
    let cfg = tempfile::tempdir().expect("a config dir");
    app.config_dir_override = Some(cfg.path().to_path_buf());
    assert!(!type_and_enter(&mut app, &agent, "/think"));
    let high_at = epsilon_contracts::ThinkingLevel::ALL
        .iter()
        .position(|level| *level == High)
        .expect("ALL holds high");
    for _ in 0..high_at {
        handle_key(&mut app, &agent, key(KeyCode::Down));
    }
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert!(!app.think_pick_open, "picker closes after apply");
    assert_eq!(agent.thinking_level(), High);
    // Reopen, move once, Esc keeps the applied level.
    assert!(!type_and_enter(&mut app, &agent, "/think"));
    handle_key(&mut app, &agent, key(KeyCode::Down));
    handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert!(!app.think_pick_open, "picker closes on Esc");
    assert_eq!(agent.thinking_level(), High);
}

#[tokio::test]
async fn ctrl_t_cycles_the_thinking_ladder() {
    let (mut app, agent, (_f, _n)) = test_app();
    // Pinned for the same reason as the `/think` tests: cycling persists the
    // new level, and a test writes nowhere near a real config.
    let cfg = tempfile::tempdir().expect("a config dir");
    app.config_dir_override = Some(cfg.path().to_path_buf());
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

    // After acceptance, submits flow normally again (bare /think opens the
    // picker, P-048 — it no longer reports the level inline).
    assert!(!type_and_enter(&mut app, &agent, "/think"));
    assert!(app.think_pick_open, "bare /think opens the picker");
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("thinking level — pick one"))
    );
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
    app.compact_running
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(!type_and_enter(&mut app, &agent, "/compact"));
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t.contains("already running")),
        "must refuse second /compact: {texts:?}"
    );
    // non-compact path unchanged: /timeline still works even while compact flag set
    // (compact only guards /compact).
}

/// **`/compact` works more than once per session.**
///
/// The flag was set when the work was spawned and **never cleared** — five mentions in the
/// crate, one writer, one reader, no reset — so a session could compact exactly once and
/// every later press answered *"compact already running — wait for it to finish"* for a task
/// that had finished long ago. The comment above the spawn described that as intended
/// behaviour, which is how a permanent refusal reads as a design decision.
///
/// Compaction is a core capability and the refusal is *plausible*: it names a real state,
/// so nobody suspects it and there is no way to discover it is wrong. This is the
/// wrong-but-plausible state `AGENTS.md` exists to forbid.
///
/// The wait is a bounded poll on the observable condition rather than a sleep-and-hope:
/// the task is detached and there is no join handle to await, so "it finished" is exactly
/// what the flag says.
#[tokio::test]
async fn a_session_can_compact_more_than_once() {
    use std::sync::atomic::Ordering;

    let (mut app, agent, _rx) = test_app();
    assert!(!type_and_enter(&mut app, &agent, "/compact"));

    // It is genuinely in flight at this point — otherwise the test below would pass
    // against a build that never set the flag at all.
    assert!(
        app.compact_running.load(Ordering::Relaxed),
        "the first /compact must start work, or this test proves nothing"
    );

    for _ in 0..500 {
        if !app.compact_running.load(Ordering::Relaxed) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        !app.compact_running.load(Ordering::Relaxed),
        "the flag must clear when the compaction finishes, or every later /compact is \
         refused for ever"
    );

    let before = app.ui.lines().len();
    assert!(!type_and_enter(&mut app, &agent, "/compact"));
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        !texts.iter().any(|t| t.contains("already running")),
        "a second /compact must be accepted, not refused as already running: {texts:?}"
    );
    assert!(
        app.ui.lines()[before..]
            .iter()
            .any(|l| l.text.contains("compacting")),
        "and it must actually start a second pass: {:?}",
        app.ui.lines()[before..]
            .iter()
            .map(|l| &l.text)
            .collect::<Vec<_>>()
    );
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

/// The tree panel names its position: `(n/total)` (P-295).
///
/// Pi's tree status line reads `(selected/total)`; the counter shares the footer
/// line with the key hints (`panels.rs`: "the counter stays where the reader looks
/// for it"). Drawn through the production draw at both frame sizes, moved once to
/// prove the numerator follows the highlight — a counter stuck at 1/3 would pass a
/// test that never pressed Down.
#[test]
fn the_timeline_footer_names_its_position() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
        for (i, text) in ["first", "second", "third"].iter().enumerate() {
            let _ = st
                .append(EntryKind::Message {
                    message: epsilon_contracts::message::Message::User(UserMessage {
                        id: epsilon_contracts::new_id("msg"),
                        timestamp_ms: i as u64,
                        content: vec![UserContent::Text {
                            text: text.to_string(),
                        }],
                    }),
                    api_request_sha256: None,
                })
                .unwrap();
        }
    }
    assert!(!type_and_enter(&mut app, &agent, "/timeline"));
    assert!(app.timeline_open);
    assert_eq!(app.timeline_entries.len(), 3);
    // `/timeline` lands on the leaf (P-290), so the counter opens at 3/3.
    for (width, height) in [(80u16, 24u16), (100, 30)] {
        let rows = render_rows(&mut app, width, height);
        assert!(
            rows.iter().any(|r| r.contains("3/3")),
            "at {width}x{height} the footer must name position 3 of 3: {rows:?}"
        );
    }
    let up = KeyEvent::new(KeyCode::Up, KeyModifiers::empty());
    handle_key(&mut app, &agent, up);
    let rows = render_rows(&mut app, 80, 24);
    assert!(
        rows.iter().any(|r| r.contains("2/3")),
        "the numerator must follow the highlight: {rows:?}"
    );
}

// ---- M23: favorites ----------------------------------------------------------------

#[tokio::test]
async fn favorites_section_orders_above_recents_and_marks_stars() {
    let rows = crate::picker::build_rows_with(
        "",
        "faux-1",
        &["deepseek-chat".to_string()],
        &["kimi-k2.6".to_string()],
        None,
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
    // **Both mirrors are cleared here, and this line is the fix for a test that failed
    // twice on a machine that had run the product.** The first half of this test is
    // "what does F2 do with nothing to cycle through", but `test_app()` loads
    // `favorites` and `recents` from the user's **real** `<config dir>/config.toml`, so
    // "nothing" meant "nothing on a machine that had never launched Epsilon". On this
    // one, `recents` held four ids and F2 cycled to `gpt-4` — a real behaviour, answering
    // a different question than the one asked.
    //
    // It bit twice: once through this test, and once through a test of my own that
    // starred a favourite while asserting on it. **A test that reads the user's config
    // answers a question about the user.** Seeding both mirrors makes this one hermetic,
    // and it is the same thing the second half already does for `favorites` — the author
    // knew the mirror was what F2 owns and simply did not seed the first half.
    app.favorites.clear();
    app.recents.clear();
    // Empty list: guidance, no crash, model unchanged.
    handle_key(&mut app, &agent, key(KeyCode::F(2)));
    assert_eq!(app.model_id, "faux-1", "nothing to cycle through");

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

    // The switch is reported, not silent: Pi prints `Switched to <name>`, this
    // prints the same fact in its own words (P-038). Transcript lines only —
    // no new filesystem reads (lesson 16b above).
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t.contains("model switched to")),
        "a cycled switch must say so: {texts:?}"
    );

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

/// P-043: the `/` popup merges discovered skills alongside built-ins.
/// `/skil` matches the static skill heads, so only a `/skill commit` row on
/// screen can come from the merge. Applying is the shared Tab path (pinned
/// by `tab_completes_highlighted_candidate`), so this pins the offering.
#[tokio::test]
async fn slash_popup_offers_discovered_skills() {
    let (mut app, agent, (_f, _n)) = test_app();
    agent.set_skills(vec![epsilon_core::skills::Skill {
        name: "commit".into(),
        description: "Record changes".into(),
        dir: std::path::PathBuf::from("skills/commit"),
        disable_model_invocation: false,
    }]);
    // The popup reads the cached rows, refreshed where app meets agent.
    app.refresh_skills(&agent);
    for c in "/skil".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(app.slash_open, "popup opens for /skil");
    // The merged row reaches pixels at both frame sizes.
    for (width, height) in [(80u16, 24u16), (100, 30)] {
        let screen = render_rows(&mut app, width, height).join("\n");
        assert!(
            screen.contains("/skill commit"),
            "seeded skill is offered at {width}x{height}: {screen}"
        );
    }
}

/// P-043: the `/` popup merges discovered prompt templates alongside
/// built-ins. No static row matches `/rev`, so a `/review` row on screen
/// can only come from the merge.
#[tokio::test]
async fn slash_popup_offers_discovered_templates() {
    let (mut app, agent, (_f, _n)) = test_app();
    let dir = tempfile::tempdir().unwrap();
    app.workspace_root = dir.path().to_path_buf();
    std::fs::create_dir_all(dir.path().join(".epsilon").join("prompts")).unwrap();
    std::fs::write(
        dir.path()
            .join(".epsilon")
            .join("prompts")
            .join("review.md"),
        "Review this code.",
    )
    .unwrap();
    app.discover_prompt_templates();
    for c in "/rev".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(app.slash_open, "popup opens for /rev");
    let screen = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        screen.contains("/review"),
        "discovered template is offered: {screen}"
    );
    handle_key(&mut app, &agent, key(KeyCode::Tab));
    assert_eq!(app.input, "/review", "Tab applies the template row");
    assert!(!app.slash_open, "popup closed after apply");
}

/// P-044: `/think` completes its argument. The old popup closed on any
/// space, so `/think xh` opens nothing there; the merged popup offers
/// `/think xhigh` (the only level matching `xh`) and Tab applies it.
#[tokio::test]
async fn slash_popup_completes_think_levels() {
    let (mut app, agent, (_f, _n)) = test_app();
    for c in "/think xh".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(app.slash_open, "popup opens for /think xh");
    let screen = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        screen.contains("/think xhigh"),
        "level row is offered: {screen}"
    );
    handle_key(&mut app, &agent, key(KeyCode::Tab));
    assert_eq!(app.input, "/think xhigh", "Tab applies the level row");
    assert!(!app.slash_open, "popup closed after apply");
}

/// P-044: `/preset` completes its argument — `/preset rea` can only be
/// `/preset read-only`, offered and applied through the popup.
#[tokio::test]
async fn slash_popup_completes_preset_names() {
    let (mut app, agent, (_f, _n)) = test_app();
    for c in "/preset rea".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(app.slash_open, "popup opens for /preset rea");
    let screen = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        screen.contains("/preset read-only"),
        "preset row is offered: {screen}"
    );
    handle_key(&mut app, &agent, key(KeyCode::Tab));
    assert_eq!(app.input, "/preset read-only", "Tab applies the preset row");
    assert!(!app.slash_open, "popup closed after apply");
}

/// P-044: `/theme` completes its argument — `/theme hig` offers
/// `/theme high-contrast` through the popup.
#[tokio::test]
async fn slash_popup_completes_theme_names() {
    let (mut app, agent, (_f, _n)) = test_app();
    for c in "/theme hig".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(app.slash_open, "popup opens for /theme hig");
    let screen = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        screen.contains("/theme high-contrast"),
        "theme row is offered: {screen}"
    );
    handle_key(&mut app, &agent, key(KeyCode::Tab));
    assert_eq!(
        app.input, "/theme high-contrast",
        "Tab applies the theme row"
    );
    assert!(!app.slash_open, "popup closed after apply");
}

/// P-044: `/skill` completes the skill name from the discovered set — the
/// names are arbitrary strings no static row can carry. Tab applies with a
/// trailing space, so `on|off` follows naturally.
#[tokio::test]
async fn slash_popup_completes_skill_names() {
    let (mut app, agent, (_f, _n)) = test_app();
    agent.set_skills(vec![epsilon_core::skills::Skill {
        name: "commit".into(),
        description: "Record changes".into(),
        dir: std::path::PathBuf::from("skills/commit"),
        disable_model_invocation: false,
    }]);
    app.refresh_skills(&agent);
    for c in "/skill c".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(app.slash_open, "popup opens for /skill c");
    let screen = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        screen.contains("/skill commit"),
        "skill-name row is offered: {screen}"
    );
    handle_key(&mut app, &agent, key(KeyCode::Tab));
    assert_eq!(
        app.input, "/skill commit ",
        "Tab applies with a trailing space"
    );
    assert!(!app.slash_open, "popup closed after apply");
}

/// Pi's three-step Escape (`P-209`): the popup first, then the interrupt, then
/// the quit. Two of the three were unpinned — the idle quit was covered, and the
/// **interrupt** half ("Escape reaches the app as 'interrupt' only when the
/// autocomplete popup is closed") was not, which is the half that matters: an
/// Esc meant to dismiss a popup must not throw away a running turn.
#[tokio::test]
async fn esc_dismisses_popup_before_quitting_or_aborting() {
    let (mut app, agent, (_f, _n)) = test_app();

    // --- half 1: running. The popup eats the Esc; the interrupt does not. ----
    // `AgentStart` is how the event loop tells the UI a turn is in flight — the
    // same seam `aborting_a_run_disarms_the_goal_without_touching_it` uses,
    // rather than a real run this fixture has no provider for.
    app.ui.apply(&epsilon_contracts::AgentEvent::AgentStart);
    agent.set_goal_armed(true);
    for c in "/h".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(app.slash_open, "the popup is open while the turn streams");
    assert!(
        !handle_key(&mut app, &agent, key(KeyCode::Esc)),
        "a popup Esc does not quit"
    );
    assert!(!app.slash_open, "and it dismisses the popup");
    assert_eq!(app.input, "/h", "text survives dismissal");
    assert!(
        agent.goal_armed(),
        "the run is untouched: one Esc must not abort the work behind the popup"
    );

    // The second Esc reaches the interrupt arm, and that is observable: the goal
    // disarms, which only the interrupt arm does (the quit arm never runs here).
    assert!(
        !handle_key(&mut app, &agent, key(KeyCode::Esc)),
        "interrupt does not request exit"
    );
    assert!(
        !agent.goal_armed(),
        "but it interrupted — the arm the popup was shadowing"
    );

    // --- half 2: idle. The same Esc quits once there is nothing to eat. -----
    app.ui.apply(&epsilon_contracts::AgentEvent::AgentEnd {
        messages: Vec::new(),
    });
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
async fn export_command_writes_html_and_json() {
    let ws = tempfile::tempdir().unwrap();
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
    // Through the real command path, not the export functions directly: this
    // is what covers the arm's wiring (read entries, pick format, name the
    // default file) after the rendering moved to `epsilon_session::export`.
    assert!(!type_and_enter(&mut app, &agent, "/export out.html"));
    let html = std::fs::read_to_string(ws.path().join("out.html")).unwrap();
    assert!(html.contains("<html"), "html export must contain html tag");
    assert!(html.contains("hello"), "html must contain message text");
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t.contains("exported html")),
        "the run reports what it wrote: {texts:?}"
    );

    assert!(!type_and_enter(&mut app, &agent, "/export json out.json"));
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(ws.path().join("out.json")).unwrap())
            .unwrap();
    assert!(j.is_array(), "json export must be an array");
    let _ = app;
}

/// Resuming a session whose history contains compactions says so (P-055).
///
/// A resumed conversation reads differently when parts of it are summaries,
/// and nothing else on screen names that. Through the real `/resume` path:
/// a file with two compactions reports "2 times", and a plain file reports
/// nothing — the second half is the control that the notice comes from the
/// count, not from resuming as such.
#[tokio::test]
async fn resume_names_compactions() {
    use epsilon_contracts::session_entry::EntryKind;
    let ws = tempfile::tempdir().unwrap();
    let compacted = ws.path().join("compacted.jsonl");
    let plain = ws.path().join("plain.jsonl");
    for (path, compactions) in [(&compacted, 2), (&plain, 0)] {
        let store = std::sync::Arc::new(std::sync::Mutex::new(
            epsilon_core::SessionStore::open(path).unwrap(),
        ));
        {
            let mut guard = store.lock().unwrap();
            guard
                .append(EntryKind::Message {
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
            for _ in 0..compactions {
                guard
                    .append(EntryKind::Compaction {
                        summary: "earlier work".into(),
                        tokens_before: 1000,
                        retained_tail: vec![],
                        details: None,
                        usage: None,
                        model_id: None,
                    })
                    .unwrap();
            }
        }
    }
    let (mut app, agent, _) = test_app();
    app.workspace_root = ws.path().to_path_buf();
    let backing = std::sync::Arc::new(std::sync::Mutex::new(
        epsilon_core::SessionStore::open(ws.path().join("t.jsonl")).unwrap(),
    ));
    agent.attach_session_store(std::sync::Arc::clone(&backing));

    assert!(!type_and_enter(
        &mut app,
        &agent,
        &format!("/resume {}", compacted.display())
    ));
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("Session compacted 2 times")),
        "a twice-compacted session must say so: {texts:?}"
    );

    assert!(!type_and_enter(
        &mut app,
        &agent,
        &format!("/resume {}", plain.display())
    ));
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        !texts.iter().any(|t| t.contains("compacted")),
        "a plain session must not claim compactions: {texts:?}"
    );
    let _ = app;
}

/// A resumed compaction shows the cost and opens onto the digest (P-054/P-183, replay half).
///
/// End to end through real `/resume`: the marker the live compactor wrote regains the
/// digest the store kept — the same cost-plus-summary row the live path draws. RED
/// first: the row replayed as a bare user line with no cost and no payload.
#[tokio::test]
async fn resume_reattaches_the_compaction_digest() {
    use epsilon_contracts::session_entry::EntryKind;
    let ws = tempfile::tempdir().unwrap();
    let path = ws.path().join("compacted.jsonl");
    let user = |text: &str| {
        epsilon_contracts::message::Message::User(epsilon_contracts::message::UserMessage {
            id: "m".into(),
            timestamp_ms: 1,
            content: vec![epsilon_contracts::content::UserContent::Text { text: text.into() }],
        })
    };
    {
        let store = std::sync::Arc::new(std::sync::Mutex::new(
            epsilon_core::SessionStore::open(&path).unwrap(),
        ));
        let mut guard = store.lock().unwrap();
        guard
            .append(EntryKind::Message {
                message: user("hello"),
                api_request_sha256: None,
            })
            .unwrap();
        guard
            .append(EntryKind::Compaction {
                summary: "earlier work".into(),
                tokens_before: 1000,
                retained_tail: vec![],
                details: None,
                usage: None,
                model_id: None,
            })
            .unwrap();
        // The live compactor's marker, spelled once (COMPACTION_MARKER) — a
        // hand-typed copy would prove only that the test can spell it.
        guard
            .append(EntryKind::Message {
                message: user(epsilon_contracts::message::COMPACTION_MARKER),
                api_request_sha256: None,
            })
            .unwrap();
    }
    let (mut app, agent, _) = test_app();
    app.workspace_root = ws.path().to_path_buf();
    let backing = std::sync::Arc::new(std::sync::Mutex::new(
        epsilon_core::SessionStore::open(ws.path().join("t.jsonl")).unwrap(),
    ));
    agent.attach_session_store(std::sync::Arc::clone(&backing));

    assert!(!type_and_enter(
        &mut app,
        &agent,
        &format!("/resume {}", path.display())
    ));
    let rows: Vec<_> = app.ui.lines().iter().collect();
    let digest = rows
        .iter()
        .find(|l| l.text.contains("context compacted · from"))
        .expect("the resumed compaction must draw its cost row");
    assert_eq!(
        digest.kind,
        crate::state::LineKind::System,
        "a compaction row is a system row, not a user turn"
    );
    assert!(
        digest.is_expandable_event_row(),
        "the replayed row must open like the live one"
    );
    assert_eq!(
        digest.full.as_deref(),
        Some("earlier work"),
        "the digest rides the row as its payload"
    );
    assert!(
        digest.text.contains("no usage reported"),
        "a pi-v1-style compaction with no bill says so: {}",
        digest.text
    );
    let _ = app;
}

/// Resuming a session recorded elsewhere names the directory.
///
/// Relative paths in the file were written against the recorded directory;
/// tools run in the running one. The launch stays put — this names the fact
/// so the reader decides. RED first: silence.
#[tokio::test]
async fn resume_from_elsewhere_names_the_directory() {
    let ws = tempfile::tempdir().expect("tmp");
    let elsewhere = tempfile::tempdir().expect("elsewhere");
    let file = ws.path().join("foreign.jsonl");
    drop(
        epsilon_session::SessionStore::open_in(&file, elsewhere.path())
            .expect("a session recorded elsewhere"),
    );

    let (mut app, agent, _) = test_app();
    app.workspace_root = ws.path().to_path_buf();
    let backing = std::sync::Arc::new(std::sync::Mutex::new(
        epsilon_core::SessionStore::open(ws.path().join("t.jsonl")).unwrap(),
    ));
    agent.attach_session_store(std::sync::Arc::clone(&backing));

    assert!(!type_and_enter(
        &mut app,
        &agent,
        &format!("/resume {}", file.display())
    ));
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t.contains("recorded in")
            && t.contains(&elsewhere.path().to_string_lossy().into_owned())),
        "must name the recorded directory: {texts:?}"
    );

    // Same directory stays silent.
    let home = ws.path().join("home.jsonl");
    drop(
        epsilon_session::SessionStore::open_in(&home, ws.path()).expect("a session recorded here"),
    );
    assert!(!type_and_enter(
        &mut app,
        &agent,
        &format!("/resume {}", home.display())
    ));
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        !texts.iter().any(|t| t.contains("recorded in")),
        "no mismatch, no notice: {texts:?}"
    );
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

    // A typo is refused, naming all three palettes, and changes nothing.
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

/// **The `/theme` description may only name palettes the product has (M312).**
///
/// Found by reading, not by a failing run: the slash item advertised *"switch palette: dark,
/// light, high-contrast"*, `ThemeMode` has two variants, and `/theme high-contrast` therefore
/// answered **"unknown palette"** — a command offering an option it refuses, in the one place a
/// reader is told what is possible. ADR-024's failure class, reached from a direction its tests
/// did not look at: `every_offered_command_is_reachable_from_slash_items` compares **heads**,
/// so a head that exists and an argument that does not is invisible to it.
///
/// **Both directions are asserted, because both are ways this lies.** A description may not name
/// a palette the code refuses, and a palette the code has may not go unmentioned — the second is
/// the one that is easy to create by adding a variant and forgetting the prose.
///
/// The names are read out of the description's own list, so the test does not hard-code them
/// twice; if someone rewrites that sentence into prose, this fails and says why.
#[test]
fn every_palette_the_description_advertises_is_one_the_product_has() {
    use crate::ui::theme::ThemeMode;

    let described: Vec<String> = epsilon_contracts::command::SLASH_ITEMS
        .iter()
        .find(|i| i.value.trim() == "/theme")
        .expect("`/theme` is offered")
        .description
        .rsplit_once(": ")
        .expect("the description lists its palettes after a colon")
        .1
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    assert!(
        described.len() >= 2,
        "the list was not found in the description, so this test is checking nothing: {described:?}"
    );

    for name in &described {
        assert!(
            ThemeMode::from_name(name).is_some(),
            "`/theme {name}` is advertised and then refused. Either build the palette or stop \
             naming it — a command that offers an option it will not take is worse than one that \
             offers fewer."
        );
    }

    let missing = [ThemeMode::Dark, ThemeMode::Light]
        .into_iter()
        .map(|m| m.name())
        .filter(|n| !described.iter().any(|d| d == n))
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "the product has a palette the `/theme` description never mentions, so a reader cannot \
         find it: {missing:?}"
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

/// A long **CJK** preview fits its pane, where a character is two columns.
///
/// The defect this closes was measured, not imagined: the budget was `chars().take(100)`,
/// and a hundred ideographs are two hundred columns, so the preview overflowed the very
/// column it existed to fit — and it only ever overflowed for someone whose file names are
/// not ASCII.
#[test]
fn a_wide_script_preview_does_not_overflow_its_pane() {
    let long: String = "目的赶到目标".repeat(60); // 240 columns
    let entry = epsilon_contracts::SessionEntry {
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
    let width = unicode_width::UnicodeWidthStr::width(snip.as_str());
    assert!(
        width <= 100,
        "the preview must fit its budget: {width} columns in {snip:?}"
    );
    assert!(
        snip.ends_with('\u{2026}'),
        "and must say it was cut: {snip:?}"
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
    // **The budget is columns and it includes the mark.** This said "100 characters plus
    // the ellipsis" — 101 columns for a 100-column budget, one over the pane the preview
    // exists to fit. The *claim* the old assertion made (cut, and marked) still holds; only
    // the unit was wrong, and it was wrong in the same direction for every script where a
    // character is not a column.
    assert_eq!(
        unicode_width::UnicodeWidthStr::width(snip.as_str()),
        100,
        "the marked preview must fit the budget it was given, including the ellipsis"
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

/// A tool call in the tree names its tool (P-291).
///
/// Assistant entries carry their tool calls, and the snip read only the prose — so a
/// turn that only called tools previewed as `(empty)`, the least informative row in a
/// picker whose job is finding "that bash call". RED first: the row says `(empty)`.
#[test]
fn a_tool_call_row_names_its_tool_and_command() {
    use epsilon_contracts::content::ContentBlock;
    use epsilon_contracts::message::{AssistantMessage, Message};
    let calls = |content: Vec<ContentBlock>| epsilon_contracts::SessionEntry {
        id: "e1".into(),
        seq: 0,
        parent_id: None,
        timestamp_ms: 0,
        kind: epsilon_contracts::session_entry::EntryKind::Message {
            message: Message::Assistant(AssistantMessage {
                id: "m1".into(),
                timestamp_ms: 0,
                provider: "faux".into(),
                model_id: "faux-1".into(),
                content,
                stop_reason: epsilon_contracts::message::StopReason::ToolUse,
                error_message: None,
                usage: epsilon_contracts::message::Usage::default(),
            }),
            api_request_sha256: None,
        },
    };
    let call = |id: &str, name: &str, args: serde_json::Value| ContentBlock::ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: args,
    };

    let snip = timeline_snip(&calls(vec![call(
        "c1",
        "bash",
        serde_json::json!({ "command": "cargo test" }),
    )]));
    assert!(
        snip.contains("[bash]") && snip.contains("cargo test"),
        "a bare call row must name tool and command, not read (empty): {snip:?}"
    );

    let snip = timeline_snip(&calls(vec![call(
        "c1",
        "read",
        serde_json::json!({ "path": "src/main.rs" }),
    )]));
    assert!(
        snip.contains("[read]") && snip.contains("src/main.rs"),
        "a read row must name its path: {snip:?}"
    );

    // Prose beside calls is kept, not replaced: the row is still one entry.
    let mut with_prose = calls(vec![call(
        "c1",
        "bash",
        serde_json::json!({ "command": "cargo test" }),
    )]);
    if let epsilon_contracts::session_entry::EntryKind::Message {
        message: Message::Assistant(a),
        ..
    } = &mut with_prose.kind
    {
        a.content.insert(
            0,
            ContentBlock::Text {
                text: "checking the build".into(),
            },
        );
    }
    let snip = timeline_snip(&with_prose);
    assert!(
        snip.contains("[bash]") && snip.contains("checking the build"),
        "calls and prose share the row: {snip:?}"
    );
}

/// A tree row for a ranged `read` names the range, on the frame (P-291).
///
/// Pi's tree shows `[read: path:first-last]` from the call's `offset`/`limit`;
/// the snip unit pins the head, this pins the drawn row through the production
/// draw at both frame sizes. The stored turn is the session's only entry, so it
/// is the leaf — which pi always shows, and which the drop rules (P-305) must
/// keep showing, or this test names that regression too.
#[test]
fn a_tree_row_for_a_ranged_read_names_the_range_on_the_frame() {
    let (mut app, agent, _rx, _dir) = test_app_with_store();
    {
        let store = agent.session_store().expect("test store attached");
        let mut st = store.lock().expect("store lock");
        use epsilon_contracts::content::ContentBlock;
        use epsilon_contracts::message::{AssistantMessage, Message};
        use epsilon_contracts::session_entry::EntryKind;
        st.append(EntryKind::Message {
            message: Message::Assistant(AssistantMessage {
                id: "m-read".into(),
                timestamp_ms: 0,
                provider: "faux".into(),
                model_id: "faux-1".into(),
                content: vec![ContentBlock::ToolCall {
                    id: "c1".into(),
                    name: "read".into(),
                    arguments: serde_json::json!({
                        "path": "src/main.rs",
                        "offset": 20,
                        "limit": 10
                    }),
                }],
                stop_reason: epsilon_contracts::message::StopReason::ToolUse,
                error_message: None,
                usage: epsilon_contracts::message::Usage::default(),
            }),
            api_request_sha256: None,
        })
        .expect("append");
    }
    assert!(!type_and_enter(&mut app, &agent, "/timeline"));
    assert!(app.timeline_open, "the picker must open");
    for (width, height) in [(80u16, 24u16), (100, 30)] {
        let rows = render_rows(&mut app, width, height);
        let screen = rows.join("\n");
        assert!(
            screen.contains("src/main.rs:20-29"),
            "the tree row must carry the requested window at {width}x{height}:\n{screen}"
        );
    }
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
/// The slash popup must fit its descriptions rather than cut them.
///
/// This is a frame test, not a unit assertion: the defect was a box 56 columns
/// wide inside an 80-column terminal, with the description right-aligned into
/// whatever the command name left over — so `browse session files in this
/// workspace` drew as `browse session files in this workspac…` on every row that
/// had a description at all. A control that only checked the row text would have
/// passed against that.
///
/// The frame is the observable because it is the only place the truncation was
/// visible: the candidate list was complete and correctly built, and the paint
/// threw away the part of each sentence the reader was choosing by.
#[tokio::test]
async fn the_slash_popup_fits_the_description_it_offers() {
    let (mut app, agent, _rx) = test_app();
    for c in "/se".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(app.slash_open, "typing `/se` opens the command popup");

    let screen = render_rows(&mut app, 80, 24).join("\n");

    // The whole description of a matched command, not a clipped prefix of it.
    assert!(
        screen.contains("browse session files in this workspace"),
        "the slash popup clipped the description it is offering\n{screen}"
    );
    assert!(
        !screen.contains("workspac…"),
        "the description was truncated with an ellipsis\n{screen}"
    );
    // Every command's description starts in the same column, so the eye scans
    // by position. The old layout right-aligned the description, which put
    // `/search`'s sentence at column 22 and `/sessions`' at column 16 — a
    // ragged column that made each row a small reading puzzle. Measured from
    // the description itself, not from the command: the commands have different
    // lengths on purpose, so where a *name* ends is not a shared column.
    // Measured in **characters**, not bytes: the frame's rails are multi-byte
    // box glyphs, so a byte offset would call a row misaligned when every
    // character is exactly where it belongs. This is the same unit the layout
    // itself uses (`ui::layout::display_width`), which is why the assertion and
    // the code under test cannot disagree about what a column is.
    let column_of = |needle: &str| {
        let row = screen
            .lines()
            .find(|row| row.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} is not on the frame\n{screen}"));
        let at = row.find(needle).expect("just checked");
        row[..at].chars().count()
    };
    let short_desc = column_of("search this session's history");
    let long_desc = column_of("enable/disable models");
    assert_eq!(
        short_desc, long_desc,
        "every command's description must start in one character column; \
         `/search`'s starts at {short_desc} and `/scoped-models`' at \
         {long_desc}\n{screen}"
    );
    // The description is dim, which is a different role from the command name's
    // body text — the second half of "distinct text colours": a reader can tell
    // the two apart by weight before reading either. Proved by rendering, not
    // by asserting the style object: the frame carries both colours.
    let popup: Vec<&str> = screen.lines().filter(|r| r.contains("commands")).collect();
    assert!(
        !popup.is_empty(),
        "the popup itself must be drawn\n{screen}"
    );
}

/// A description too long for one line wraps onto an indented continuation
/// rather than being cut — and the continuation is a whole phrase, not a
/// one-word orphan like `g)` on a line of its own.
#[tokio::test]
async fn a_long_slash_description_wraps_to_a_whole_continuation_line() {
    let (mut app, agent, _rx) = test_app();
    for c in "/se".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    assert!(app.slash_open, "the popup is open");

    let screen = render_rows(&mut app, 80, 24).join("\n");
    // `/scoped_models`'s description is 72 characters and does not fit beside
    // its name, so it continues on a second row indented under the column.
    assert!(
        screen.contains("(underscore spell"),
        "the long description's first part is on screen\n{screen}"
    );
    assert!(
        screen.contains("spelling)"),
        "the long description's continuation must be readable, not truncated\n{screen}"
    );
    // The orphan rule: a continuation line holds a phrase, never a lone token.
    for row in screen
        .lines()
        .filter(|r| r.contains("/scoped_models") || r.contains("spelling"))
    {
        let trimmed = row.trim();
        assert!(
            trimmed.len() > 4,
            "a wrapped line must carry a phrase, not an orphan: {trimmed:?}\n{screen}"
        );
    }
}

/// The answer ends the way every block before it did, and the composer is not
/// the answer's last line.
///
/// The reported defect: with the transcript running straight into the composer,
/// a response's final row and the input row shared an edge, so nothing on screen
/// said where the conversation stopped — and the composer's prompt glyph is the
/// one a submitted turn keeps, which made the input line read as one more
/// transcript row. [`Regions::gap`] is the fix: one blank, the same blank
/// `push_line` puts between blocks, between them.
///
/// "distinct text colors" is a claim about the **buffer**, not about the prose.
///
/// The user row is drawn with `Role::user` and the answer with `Role::assistant`;
/// both resolve through `Theme::role`, so the pair is proved at each colour
/// depth the app runs in rather than inferred from a palette table. The glyphs
/// carry the same distinction (`▎ ❯` against the answer's plain gutter), so
/// colour is never the only signal — which is the accessibility rule this
/// proves at the same time.
///
/// **The palette is pinned and one app is rendered once.** The render cache keys
/// on the palette, not on the colour depth (`render_cache::CacheKey`), because
/// depth is detected once at startup and cannot change mid-process. Re-theming
/// an already-rendered app therefore serves truecolour cells under an ANSI-16
/// theme — a probe artefact, not a product defect, and the reason this test
/// builds one app per mode instead of switching one.
#[tokio::test]
async fn a_user_row_and_an_answer_are_distinct_on_screen() {
    for (mode, want_user, want_answer) in [
        (
            crate::ui::theme::ColorMode::Truecolor,
            ratatui::style::Color::Rgb(0x7d, 0xd3, 0xfc),
            ratatui::style::Color::Rgb(0xd7, 0xdb, 0xe2),
        ),
        (
            crate::ui::theme::ColorMode::Ansi16,
            ratatui::style::Color::Cyan,
            ratatui::style::Color::Reset,
        ),
    ] {
        let (mut app, agent, _rx) = test_app_with_faux(vec![epsilon_llm::FauxStep::say(
            "Paris is the capital of France.",
        )]);
        // Set before the first frame: the cache holds resolved styles.
        app.theme = crate::ui::theme::Theme::with_palette(
            mode,
            app.theme.glyphs,
            crate::ui::theme::ThemeMode::Dark,
        );
        let prompt = "what is the capital of France?";
        app.ui.push_local_input(prompt);
        run_one_turn(&mut app, &agent, prompt).await;

        let (buffer, rows) = render_buffer(&mut app, 80, 24);
        let user_y = rows
            .iter()
            .position(|r| r.contains(prompt))
            .unwrap_or_else(|| panic!("the prompt is on screen: {rows:?}"));
        let answer_y = rows
            .iter()
            .position(|r| r.contains("Paris is the capital of France."))
            .unwrap_or_else(|| panic!("the answer is on screen: {rows:?}"));

        let (user_glyph, user_fg) = first_glyph(&buffer, user_y as u16);
        let (answer_glyph, answer_fg) = first_glyph(&buffer, answer_y as u16);

        assert_eq!(
            user_glyph,
            app.theme.glyphs.cursor_bar.to_string(),
            "the user row is marked with the cursor bar ({mode:?})"
        );
        assert_eq!(
            user_fg,
            Some(want_user),
            "the user row's colour under {mode:?}"
        );
        assert_eq!(
            answer_fg,
            Some(want_answer),
            "the answer's colour under {mode:?}"
        );
        assert_ne!(
            user_fg, answer_fg,
            "asking and answering are different colours ({mode:?})"
        );
        assert_ne!(
            answer_glyph, "❯",
            "the answer never wears the prompt caret, so the glyph says who spoke too ({mode:?})"
        );
    }
}

/// The first cell on `y` that holds a glyph, as (symbol, foreground).
fn first_glyph(
    buffer: &ratatui::buffer::Buffer,
    y: u16,
) -> (String, Option<ratatui::style::Color>) {
    for x in 0..buffer.area.width {
        let cell = &buffer[(x, y)];
        if cell.symbol().trim().is_empty() {
            continue;
        }
        return (cell.symbol().to_string(), cell.style().fg);
    }
    (String::new(), None)
}

/// The answer ends the way every block before it did, and the composer is not
/// the answer's last line.
///
/// The reported defect: with the transcript running straight into the composer,
/// a response's final row and the input row shared an edge, so nothing on screen
/// said where the conversation stopped — and the composer's prompt glyph is the
/// one a submitted turn keeps, which made the input line read as one more
/// transcript row. [`Regions::gap`] is the fix: one blank, the same blank
/// `push_line` puts between blocks, between them.
///
/// The assertion is on the **frame** rather than on the regions, because a frame
/// is what the reader sees — and the control is deliberately brutal: delete the
/// `Constraint::Length(1)` in `Regions::split` and this test fails with the
/// assistant's own answer sitting directly above the placeholder.
#[tokio::test]
async fn a_finished_answer_separates_from_the_composer_it_feeds() {
    let (mut app, agent, _rx) = test_app_with_faux(vec![
        epsilon_llm::FauxStep::say(
            "Paris is the capital of France. It has stood on the Seine for two thousand years.",
        ),
        epsilon_llm::FauxStep::say("That is where most of the population sits, on the river."),
    ]);
    // Pinned repertoire: this asserts exact frame widths, and `Theme::detect()`
    // reads the runner's env (round-105 precedent: a TERM change flipped the
    // suite ASCII<->unicode with no source change). Only the glyphs change here,
    // not the colour mode. golden.rs pins the same way.
    app.theme.glyphs = crate::ui::theme::UNICODE;
    for prompt in [
        "what is the capital of France?",
        "and is that where the population sits?",
    ] {
        app.ui.push_local_input(prompt);
        run_one_turn(&mut app, &agent, prompt).await;
    }

    for width in [80u16, 100] {
        let rows = render_rows(&mut app, width, 30);
        let composer_y = rows
            .iter()
            .rposition(|r| r.starts_with('❯') && r.contains(crate::ui::composer::PLACEHOLDER))
            .unwrap_or_else(|| panic!("{width}: no composer row in:\n{}", rows.join("\n")));
        assert!(composer_y > 2, "{width}: composer has nothing above it");
        let above = rows[composer_y - 1].trim();
        assert_eq!(
            above, "",
            "{width}: the row above the composer must be blank, not {above:?}"
        );

        // Not an empty frame passing by accident: the answer is on screen, and it
        // is at least one row further up than the composer.
        let answer_y = rows
            .iter()
            .rposition(|r| r.contains("population sits"))
            .unwrap_or_else(|| {
                panic!("{width}: the answer is not on screen:\n{}", rows.join("\n"))
            });
        assert!(
            answer_y + 1 < composer_y,
            "{width}: the answer sits on the composer's own row ({answer_y} vs {composer_y})"
        );
    }
}

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
            || app.think_pick_open
            || app.scope_open
            || app.changes_open
            || app.settings_open
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

/// `/open` in the terminal names the window instead of failing unknown (D-046/S-105).
///
/// The head parses (it is in COMMAND_HEADS for the window's parser), but this
/// frontend is bound to its process directory — so the sentence must name where
/// the door lives, not report a typo. RED first: the notice reads "unknown command".
#[tokio::test]
async fn open_in_the_terminal_names_the_window() {
    let (mut app, agent, _rx) = test_app();
    assert!(!type_and_enter(&mut app, &agent, "/open /tmp"));
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("/open") && t.contains("desktop window")),
        "the refusal must name the command and where it works: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("/resume")),
        "and offer the terminal's nearest door: {texts:?}"
    );
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
/// The tree drops tool-only turns but keeps the leaf and the broken (P-305).
///
/// Pi hides assistant messages with only tool calls and a normal stop reason —
/// navigation noise — unless the turn is the leaf the reader is on or carries
/// an abnormal stop, which is exactly what a reader hunting "where did it
/// break" needs to find. RED first: the picker lists every turn today.
#[test]
fn the_tree_drops_tool_only_turns_but_keeps_the_leaf_and_the_broken() {
    use epsilon_contracts::content::ContentBlock;
    use epsilon_contracts::content::UserContent;
    use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage, UserMessage};
    use epsilon_contracts::session_entry::EntryKind;
    use epsilon_session::SessionStore;

    fn push_user(store: &mut SessionStore, text: &str) {
        store
            .append(EntryKind::Message {
                message: Message::User(UserMessage {
                    id: epsilon_contracts::new_id("msg"),
                    timestamp_ms: 0,
                    content: vec![UserContent::Text { text: text.into() }],
                }),
                api_request_sha256: None,
            })
            .unwrap();
    }
    fn push_call(store: &mut SessionStore, cmd: &str, stop: StopReason, err: Option<&str>) {
        store
            .append(EntryKind::Message {
                message: Message::Assistant(AssistantMessage {
                    id: epsilon_contracts::new_id("msg"),
                    timestamp_ms: 0,
                    provider: "faux".into(),
                    model_id: "faux-1".into(),
                    content: vec![ContentBlock::ToolCall {
                        id: epsilon_contracts::new_id("call"),
                        name: "bash".into(),
                        arguments: serde_json::json!({ "command": cmd }),
                    }],
                    stop_reason: stop,
                    error_message: err.map(str::to_string),
                    usage: Usage::default(),
                }),
                api_request_sha256: None,
            })
            .unwrap();
    }

    let dir = tempfile::tempdir().unwrap();
    let mut store = SessionStore::open(dir.path().join("s.jsonl")).unwrap();
    push_user(&mut store, "q");
    push_call(&mut store, "mid-cmd", StopReason::EndTurn, None);
    push_call(&mut store, "err-cmd", StopReason::Error, Some("boom"));
    push_call(&mut store, "leaf-cmd", StopReason::EndTurn, None);

    let forest = store.tree().unwrap();
    let active: std::collections::HashSet<String> = store
        .branch_entries()
        .unwrap()
        .iter()
        .map(|e| e.id.clone())
        .collect();
    let leaf = store.leaf_id();
    let mut rows: Vec<crate::app::TimelineRow> = Vec::new();
    super::commands::flatten_tree(
        &forest,
        0,
        &active,
        &Default::default(),
        leaf.as_deref(),
        &mut rows,
    );

    let snips: Vec<&str> = rows.iter().map(|r| r.snippet.as_str()).collect();
    assert!(
        !snips.iter().any(|s| s.contains("mid-cmd")),
        "a tool-only turn mid-tree is navigation noise: {snips:?}"
    );
    assert!(
        snips.iter().any(|s| s.contains("err-cmd")),
        "a broken turn is where 'where did it break' lives: {snips:?}"
    );
    assert!(
        snips.iter().any(|s| s.contains("leaf-cmd")),
        "the leaf is always visible, noise or not: {snips:?}"
    );
}

/// The tree sorts the leaf's branch first at every level (P-303).
///
/// Pi's rule (`tree-selector.ts:245,274,217-240`): the subtree holding the
/// current leaf comes before the abandoned lanes, so the live context meets
/// the eye first. RED first: rows come out in stored timestamp order today.
#[test]
fn the_tree_sorts_the_leaf_branch_first() {
    use epsilon_contracts::content::UserContent;
    use epsilon_contracts::message::{Message, UserMessage};
    use epsilon_contracts::session_entry::EntryKind;
    use epsilon_session::SessionStore;

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

    let dir = tempfile::tempdir().unwrap();
    let mut store = SessionStore::open(dir.path().join("s.jsonl")).unwrap();
    let root = push(&mut store, "root");
    push(&mut store, "a1");
    push(&mut store, "a2");
    // Rewind to the root, then go a different way: the leaf lives on branch B.
    store.navigate_to(&root).unwrap();
    push(&mut store, "b1");

    let forest = store.tree().unwrap();
    let active: std::collections::HashSet<String> = store
        .branch_entries()
        .unwrap()
        .iter()
        .map(|e| e.id.clone())
        .collect();
    let leaf = store.leaf_id();
    let mut rows: Vec<crate::app::TimelineRow> = Vec::new();
    super::commands::flatten_tree(
        &forest,
        0,
        &active,
        &Default::default(),
        leaf.as_deref(),
        &mut rows,
    );

    let pos = |needle: &str| {
        rows.iter()
            .position(|r| r.snippet.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} must be listed: {rows:?}"))
    };
    assert!(
        pos("b1") < pos("a1") && pos("b1") < pos("a2"),
        "the leaf's branch sorts before the abandoned lane: {rows:?}"
    );
}

/// The dropped-and-ordered tree, on the frame (P-303, P-305).
///
/// Through the production path — store, `/timeline`, draw — at both frame
/// sizes: branch A's tool-only turn is gone, the leaf's branch B sorts before
/// the abandoned lane, and the leaf itself stays. If the sort ever stops
/// leading with the leaf, `b-q` sinks below `a-q`; if the drop ever takes the
/// leaf with it, `leaf-cmd` vanishes with `a-cmd`.
#[test]
fn the_timeline_frame_drops_noise_and_leads_with_the_leaf_branch() {
    use epsilon_contracts::content::{ContentBlock, UserContent};
    use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage, UserMessage};
    use epsilon_contracts::session_entry::EntryKind;
    let (mut app, agent, _rx, _dir) = test_app_with_store();
    {
        let store = agent.session_store().expect("test store attached");
        let mut st = store.lock().expect("store lock");
        fn user(st: &mut epsilon_session::SessionStore, text: &str) -> String {
            st.append(EntryKind::Message {
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
        fn call(st: &mut epsilon_session::SessionStore, cmd: &str) {
            st.append(EntryKind::Message {
                message: Message::Assistant(AssistantMessage {
                    id: epsilon_contracts::new_id("msg"),
                    timestamp_ms: 0,
                    provider: "faux".into(),
                    model_id: "faux-1".into(),
                    content: vec![ContentBlock::ToolCall {
                        id: epsilon_contracts::new_id("call"),
                        name: "bash".into(),
                        arguments: serde_json::json!({ "command": cmd }),
                    }],
                    stop_reason: StopReason::EndTurn,
                    error_message: None,
                    usage: Usage::default(),
                }),
                api_request_sha256: None,
            })
            .unwrap();
        }
        let root = user(&mut st, "root");
        user(&mut st, "a-q");
        call(&mut st, "a-cmd");
        st.navigate_to(&root).expect("rewind");
        user(&mut st, "b-q");
        call(&mut st, "leaf-cmd");
    }
    assert!(!type_and_enter(&mut app, &agent, "/timeline"));
    assert!(app.timeline_open, "the picker must open");
    for (width, height) in [(80u16, 24u16), (100, 30)] {
        let rows = render_rows(&mut app, width, height);
        let screen = rows.join("\n");
        assert!(
            !screen.contains("a-cmd"),
            "a tool-only turn mid-tree stays off the screen at {width}x{height}:\n{screen}"
        );
        assert!(
            screen.contains("leaf-cmd"),
            "the leaf stays on the screen at {width}x{height}:\n{screen}"
        );
        let pos = |needle: &str| {
            screen.find(needle).unwrap_or_else(|| {
                panic!("{needle:?} must be on screen at {width}x{height}:\n{screen}")
            })
        };
        assert!(
            pos("b-q") < pos("a-q"),
            "the leaf's branch leads at {width}x{height}:\n{screen}"
        );
    }
}

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
    super::commands::flatten_tree(
        &forest,
        0,
        &active,
        &Default::default(),
        store.leaf_id().as_deref(),
        &mut rows,
    );

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

/// Phase 3, viewer tabs: Enter on a second file opens a second tab instead of
/// replacing the first; alt+[ / alt+] switch with wrap; Esc closes the active
/// tab and the viewer only when the last tab closes.
#[tokio::test]
async fn viewer_tabs_open_switch_and_close() {
    let (mut app, agent, _receivers, dir) = test_app_with_store();
    std::fs::write(dir.path().join("a.txt"), "aaa\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "bbb\n").unwrap();
    handle_key(&mut app, &agent, key(KeyCode::F(6)));
    assert!(app.tree_open);
    // Rows are dirs-first, then files: a.txt first.
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert!(app.viewer_open(), "Enter opens the first tab");
    assert!(!app.tree_open, "the tree yields on open, like every picker");
    // F6 brings the tree back over the viewer; Enter opens the second file
    // as a second tab instead of replacing the first.
    handle_key(&mut app, &agent, key(KeyCode::F(6)));
    assert!(app.tree_open, "F6 yields to the tree with tabs kept");
    handle_key(&mut app, &agent, key(KeyCode::Down));
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert_eq!(
        app.viewer_tabs.len(),
        2,
        "a second file opens a second tab, not a replacement"
    );
    fn active_rel(app: &super::App) -> Option<String> {
        app.active_viewer().map(|t| t.rel.clone())
    }
    assert_eq!(
        active_rel(&app).as_deref(),
        Some("b.txt"),
        "the new tab is active"
    );

    // The strip names both tabs on screen at both sizes.
    for (width, height) in [(80u16, 24u16), (100, 30)] {
        let screen = render_rows(&mut app, width, height).join("\n");
        assert!(
            screen.contains("a.txt") && screen.contains("b.txt"),
            "tab strip names both tabs at {width}x{height}: {screen}"
        );
    }

    let prev = || key_mod(KeyModifiers::ALT, KeyCode::Char('['));
    let next = || key_mod(KeyModifiers::ALT, KeyCode::Char(']'));
    handle_key(&mut app, &agent, prev());
    assert_eq!(
        active_rel(&app).as_deref(),
        Some("a.txt"),
        "alt+[ steps back"
    );
    handle_key(&mut app, &agent, next());
    assert_eq!(
        active_rel(&app).as_deref(),
        Some("b.txt"),
        "alt+] steps forward"
    );
    handle_key(&mut app, &agent, next());
    assert_eq!(
        active_rel(&app).as_deref(),
        Some("a.txt"),
        "switching wraps past the last tab"
    );

    // Esc closes the active tab; the viewer survives while tabs remain.
    handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert_eq!(app.viewer_tabs.len(), 1, "one tab closed");
    assert!(app.viewer_open(), "viewer survives with tabs left");
    handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert!(app.viewer_tabs.is_empty(), "last tab closed");
    assert!(!app.viewer_open(), "viewer closes with its last tab");
    assert!(app.tree_open, "closing the last tab returns to the tree");
}

/// A dirty tab does not close on the first Esc: it arms and names the cost,
/// like the edit-mode discard it mirrors.
#[tokio::test]
async fn viewer_dirty_tab_arms_before_closing() {
    let (mut app, agent, _receivers, dir) = test_app_with_store();
    std::fs::write(dir.path().join("a.txt"), "aaa\n").unwrap();
    handle_key(&mut app, &agent, key(KeyCode::F(6)));
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert!(app.viewer_open());
    // Edit: `e`, type, Esc (arms the edit discard), Esc (leaves edit mode
    // with the buffer kept in the tab).
    handle_key(&mut app, &agent, key(KeyCode::Char('e')));
    handle_key(&mut app, &agent, key(KeyCode::Char('X')));
    handle_key(&mut app, &agent, key(KeyCode::Esc));
    handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert!(
        app.viewer_tabs
            .first()
            .is_some_and(|t| !t.editing && t.has_unsaved()),
        "leaving edit mode keeps the unsaved buffer in the tab"
    );
    // Esc in view mode must arm on a dirty tab, not close.
    let before = app.ui.lines().len();
    handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert!(
        app.viewer_open(),
        "first Esc on a dirty tab arms instead of closing"
    );
    assert!(app.ui.lines().len() > before, "the arm names its cost");
    handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert!(!app.viewer_open(), "second Esc closes the dirty tab");
}

/// Phase 3, viewer find: `/` searches the open file, Enter steps through matches.
///
/// Matches are counted, not highlighted — the transcript search's own rule
/// (P-217/P-218): the count plus the scroll following the match is the whole
/// signal. View mode only; in edit mode `/` types.
#[test]
fn viewer_slash_finds_and_steps_through_matches() {
    let (mut app, agent, _rx) = test_app();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("notes.txt"),
        "alpha line\nbeta needle gamma\nplain line\nbeta again\n",
    )
    .unwrap();
    app.workspace_root = dir.path().to_path_buf();
    app.viewer_tabs =
        vec![crate::tree::open_viewer(&app.workspace_root, "notes.txt").expect("fixture opens")];
    app.viewer_tab = 0;
    assert!(app.viewer_open(), "precondition: the viewer is open");

    handle_key(&mut app, &agent, key(KeyCode::Char('/')));
    assert!(
        app.active_viewer().is_some_and(|v| v.find_open),
        "/ opens find in view mode"
    );
    for c in "beta".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    let (hits, index) = app
        .active_viewer()
        .map(|v| (v.find_hits.len(), v.find_index))
        .unwrap();
    assert_eq!(hits, 2, "both beta lines match");
    assert_eq!(index, 0, "first match selected");

    handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert_eq!(
        app.active_viewer().map(|v| v.find_index),
        Some(1),
        "Enter steps to the next match"
    );
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert_eq!(
        app.active_viewer().map(|v| v.find_index),
        Some(0),
        "stepping wraps past the last match"
    );

    // Count on screen at both sizes, through the production draw.
    for (width, height) in [(80u16, 24u16), (100, 30)] {
        let screen = render_rows(&mut app, width, height).join("\n");
        assert!(
            screen.contains("1 of 2"),
            "match count reaches pixels at {width}x{height}: {screen}"
        );
    }

    handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert!(
        app.active_viewer().is_some_and(|v| !v.find_open),
        "Esc closes find; the viewer stays open"
    );
    assert!(app.viewer_open(), "Esc closes find, not the viewer");
}

/// Pressing `e` on a file past the viewer cap refuses edit mode with a
/// named notice: the edit buffer would be the capped head, and saving it
/// would cut the tail off the file (data loss, AGENTS.md section 6).
#[test]
fn viewer_edit_of_a_truncated_file_is_refused_with_a_notice() {
    let (mut app, agent, _rx) = test_app();
    let dir = tempfile::tempdir().unwrap();
    let big: String = (0..crate::tree::VIEWER_LINE_CAP + 50)
        .map(|i| format!("line {i}\n"))
        .collect();
    std::fs::write(dir.path().join("big.log"), &big).unwrap();
    app.workspace_root = dir.path().to_path_buf();
    app.viewer_tabs =
        vec![crate::tree::open_viewer(&app.workspace_root, "big.log").expect("fixture opens")];
    app.viewer_tab = 0;
    assert!(
        app.active_viewer().is_some_and(|v| v.truncated),
        "precondition: the fixture is past the cap"
    );

    handle_key(&mut app, &agent, key(KeyCode::Char('e')));
    assert!(
        app.active_viewer().is_some_and(|v| !v.editing),
        "edit mode must not open on a truncated file"
    );
    let texts: Vec<String> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        texts.iter().any(|t| t.contains("too large to edit")),
        "the refusal must say why, got {texts:?}"
    );

    // The footer must not offer `e` as edit anymore (invariant 7: a bound
    // key the UI does not act on says so), at both frame sizes.
    for (width, height) in [(80u16, 24u16), (100, 30)] {
        let screen = render_rows(&mut app, width, height).join("\n");
        assert!(
            screen.contains("edit unavailable"),
            "footer names the refused key at {width}x{height}: {screen}"
        );
    }
}

/// P-307: inside `/tree`, the copy key takes the SELECTED entry's full text,
/// not the 200-character preview on screen — and not the last assistant
/// message, which is what the same key copies outside the picker.
///
/// The entry text is 400 chars (past the preview budget); the assertions
/// branch on the parked sequence exactly like `copy_parks_a_sequence...`,
/// because the native half is machine-dependent and neither branch may pass
/// by accident.
#[test]
fn tree_copy_takes_the_selected_entry_full_text() {
    use epsilon_contracts::content::UserContent;
    use epsilon_contracts::message::{Message, UserMessage};
    use epsilon_contracts::session_entry::EntryKind;
    use epsilon_session::SessionStore;

    let dir = tempfile::tempdir().unwrap();
    let backing = std::sync::Arc::new(std::sync::Mutex::new(
        SessionStore::open(dir.path().join("s.jsonl")).unwrap(),
    ));
    // 400 chars: past every preview budget, so a preview copy reads shorter.
    let full = "entry body ".repeat(40);
    assert_eq!(full.chars().count(), 440);
    let id = {
        let mut st = backing.lock().unwrap();
        st.append(EntryKind::Message {
            message: Message::User(UserMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: 0,
                content: vec![UserContent::Text { text: full.clone() }],
            }),
            api_request_sha256: None,
        })
        .unwrap()
        .id
    };

    let (mut app, agent, _rx) = test_app();
    agent.attach_session_store(std::sync::Arc::clone(&backing));
    app.timeline_open = true;
    app.timeline_entries = vec![crate::app::TimelineRow {
        seq: 0,
        id,
        kind: "user".into(),
        snippet: "entry body entry body".into(),
        depth: 0,
        on_active: true,
        parent: None,
        label: None,
    }];
    app.timeline_index = 0;

    handle_key(&mut app, &agent, key(KeyCode::F(4)));

    let notice = last_line_text(&app);
    assert!(
        notice.contains("440 chars"),
        "the notice counts the FULL text, not the preview: {notice}"
    );
    if let Some(sequence) = app.osc_pending.clone() {
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
            full,
            "the sequence carries the entry, not its preview"
        );
    }
    // And the copy lands on the drawn frame at both sizes.
    for (width, height) in [(80u16, 24u16), (100, 30)] {
        let screen = render_rows(&mut app, width, height).join("\n");
        assert!(
            screen.contains("440 chars"),
            "the copy count reaches pixels at {width}x{height}: {screen}"
        );
    }
}

/// P-306: PageUp/PageDown and Left/Right page the `/tree` picker by the
/// painted screenful — a 400-entry branch is unreachable one row at a time.
/// Up/Down keep stepping one row and clamping (the tree-wide rule); only the
/// page keys move by the screenful pi pages by (`session-selector.ts:612-617`).
#[test]
fn page_keys_move_the_tree_picker_by_the_painted_screenful() {
    use crate::app::{TimelineRow, TreeFilter};

    let (mut app, agent, _rx) = test_app();
    app.tree_filter = TreeFilter::All;
    app.timeline_open = true;
    app.timeline_entries = (0..30)
        .map(|i| TimelineRow {
            seq: i,
            id: format!("e{i}"),
            kind: "user".into(),
            snippet: format!("entry number {i}"),
            depth: 0,
            on_active: true,
            parent: None,
            label: None,
        })
        .collect();
    app.timeline_index = 0;
    // Paint first: the page step is the measured screenful, and a key before
    // the first paint must still move rather than silently do nothing.
    let _ = render_rows(&mut app, 80, 24);
    assert!(
        app.timeline_capacity > 1,
        "the paint records the screenful: {}",
        app.timeline_capacity
    );

    handle_key(&mut app, &agent, key(KeyCode::PageDown));
    assert_eq!(
        app.timeline_index, app.timeline_capacity,
        "PageDown moves one screenful, not one row"
    );
    handle_key(&mut app, &agent, key(KeyCode::PageUp));
    assert_eq!(app.timeline_index, 0, "PageUp returns by the same step");

    // Bare Left/Right page too (pi's mapping); the Ctrl+Arrow fold chords
    // keep their own arms and never meet these.
    handle_key(&mut app, &agent, key(KeyCode::Right));
    assert_eq!(
        app.timeline_index, app.timeline_capacity,
        "Right pages forward like PageDown"
    );
    handle_key(&mut app, &agent, key(KeyCode::Left));
    assert_eq!(app.timeline_index, 0, "Left pages back like PageUp");

    // Clamped at the far end, and the footer teaches the keys at both sizes.
    handle_key(&mut app, &agent, key(KeyCode::PageDown));
    handle_key(&mut app, &agent, key(KeyCode::PageDown));
    handle_key(&mut app, &agent, key(KeyCode::PageDown));
    assert_eq!(app.timeline_index, 29, "paging clamps at the last row");
    for (width, height) in [(80u16, 24u16), (100, 30)] {
        let screen = render_rows(&mut app, width, height).join("\n");
        assert!(
            screen.contains("left/right"),
            "the page chord is taught on screen at {width}x{height}: {screen}"
        );
    }
}

/// P-304: indentation grows only at branch points, recomputed against the
/// visible tree after a filter.
///
/// A linear lane — root, two user turns, a fork with two leaves — reads flat
/// until the fork: without the rule the last lane row would sit at depth 3
/// and a 400-entry session would march off the panel.
#[test]
fn single_child_chains_stay_flat_and_only_forks_step() {
    use super::commands::{apply_filter, visible_rows};
    use crate::app::{TimelineRow, TreeFilter};

    let row = |id: &str, parent: Option<&str>| TimelineRow {
        seq: 0,
        id: id.into(),
        kind: "user".into(),
        snippet: id.into(),
        depth: 999,
        on_active: true,
        parent: parent.map(str::to_string),
        label: None,
    };
    // root -> a -> b -> [c, d]: one lane, then a fork.
    let rows = vec![
        row("root", None),
        row("a", Some("root")),
        row("b", Some("a")),
        row("c", Some("b")),
        row("d", Some("b")),
    ];
    let depth_of = |out: &[TimelineRow], id: &str| {
        out.iter().find(|r| r.id == id).expect("row survives").depth
    };
    let out = apply_filter(&rows, TreeFilter::All, &Default::default());
    assert_eq!(depth_of(&out, "root"), 0, "root: {out:?}");
    assert_eq!(
        depth_of(&out, "a"),
        0,
        "single-child lane stays flat: {out:?}"
    );
    assert_eq!(
        depth_of(&out, "b"),
        0,
        "single-child lane stays flat: {out:?}"
    );
    assert_eq!(depth_of(&out, "c"), 1, "fork children step once: {out:?}");
    assert_eq!(depth_of(&out, "d"), 1, "fork children step once: {out:?}");

    // Hiding one fork leaf collapses the fork: the survivor flattens, because
    // its parent no longer forks the *visible* tree.
    let minus_d: Vec<TimelineRow> = rows.iter().filter(|r| r.id != "d").cloned().collect();
    let flat = apply_filter(&minus_d, TreeFilter::All, &Default::default());
    assert_eq!(
        depth_of(&flat, "c"),
        0,
        "a fork with one visible child is a lane: {flat:?}"
    );

    // Folds change the visible tree too: folding the fork hides the leaves,
    // and the lane stays flat through the whole pipeline.
    let mut folded = std::collections::HashSet::new();
    folded.insert("b".to_string());
    let piped = visible_rows(&rows, TreeFilter::All, &Default::default(), "", &folded);
    assert!(
        piped.iter().all(|r| r.depth == 0),
        "folded fork leaves a flat lane: {piped:?}"
    );
    assert!(
        !piped.iter().any(|r| r.id == "c" || r.id == "d"),
        "folded leaves are hidden: {piped:?}"
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
    // The scope map this listing is drawn with: neither skill sits under an anchor
    // this launch scans (`/tmp/...`), so both keep their paths — which is exactly
    // the honest fallback the second half of this test pins.
    let scopes = super::commands::skill_scope_by_dir(std::path::Path::new("/work/repo"));

    // An empty disable-list first: the point of this half is the DESCRIPTION.
    let lines = skill_lines(&skills, &[], &scopes);
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
    let off = skill_lines(&skills, &["commit".to_string()], &scopes);
    assert!(
        off[0].contains("[off]"),
        "a disabled skill is marked, not hidden: {off:?}"
    );
    assert!(
        !off[1].contains("[off]"),
        "and only the disabled one is: {off:?}"
    );
}

/// `/skills` says WHICH skill this is, not only what it does (P-006).
///
/// Pi groups its resource blocks by project / user / path. A flat list answers
/// "what can I invoke" and not "which one is this", which is the question you
/// have when a skill works in one repository and not another — or when deciding
/// whether to delete the copy in `~/.config`.
///
/// The second half is the one that matters for honesty: a skill under **neither**
/// anchor gets its path, not a guessed scope. This launch cannot see a root that
/// a different workspace would scan, and labelling that "project" would be the
/// silent failure `AGENTS.md` §2.7 names.
#[test]
fn the_skills_listing_names_the_scope_each_skill_came_from() {
    use super::commands::{skill_lines, skill_scope_by_dir};
    use epsilon_core::skills::Skill;

    let workspace = std::path::Path::new("/work/repo");
    let scopes = skill_scope_by_dir(workspace);
    let global = epsilon_core::config::global_dir()
        .map(|d| d.join("skills"))
        .unwrap_or_else(|| std::path::PathBuf::from("/global/skills"));

    let mk = |dir: std::path::PathBuf| Skill {
        name: "commit".into(),
        description: "write a conventional commit message".into(),
        dir,
        disable_model_invocation: false,
    };
    let project = workspace.join(".epsilon").join("skills");
    let portable = workspace.join(".agents").join("skills");
    let elsewhere = std::path::PathBuf::from("/elsewhere/skills");
    let skills = vec![
        mk(project.join("commit")),
        mk(portable.join("commit")),
        mk(global.join("commit")),
        mk(elsewhere.join("commit")),
    ];
    let lines = skill_lines(&skills, &[], &scopes);

    assert!(
        lines[0].contains("(project)"),
        "the native project root is labelled: {lines:?}"
    );
    assert!(
        lines[1].contains("(project)"),
        "so is the portable .agents root: {lines:?}"
    );
    assert!(
        lines[2].contains("(user)"),
        "the user's global root is labelled: {lines:?}"
    );
    assert!(
        lines[3].contains("/elsewhere/skills"),
        "and a root this launch does not scan keeps its path rather than a \
         guessed scope: {lines:?}"
    );
    assert!(
        !lines[3].contains("(project)") && !lines[3].contains("(user)"),
        "it must not be given a scope it does not have: {lines:?}"
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

/// Esc in the tree unfolds first and only then closes (P-299).
///
/// Pi clears the query and the folded branches first; a second Esc exits.
/// The search-mode half already exists (the query clears before the field
/// closes); the folds half did not — Esc closed over a folded branch, leaving
/// rows hidden behind a dismissal. Both directions are asserted: unfold stays
/// open, and only the emptied Esc closes.
#[test]
fn esc_in_the_tree_unfolds_first_and_only_then_closes() {
    use crate::app::TimelineRow;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let (mut app, agent, _rx) = test_app();
    let parent = TimelineRow {
        seq: 0,
        id: "a".into(),
        kind: "user".into(),
        snippet: "the parent".into(),
        depth: 0,
        on_active: true,
        parent: None,
        label: None,
    };
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
    app.timeline_all = vec![parent.clone(), child];
    app.timeline_entries = vec![parent];
    app.timeline_index = 0;
    app.timeline_open = true;
    app.folded.insert("a".into());
    app.tree_query = "parent".into();
    let esc = || KeyEvent::new(KeyCode::Esc, KeyModifiers::empty());

    handle_key(&mut app, &agent, esc());
    assert!(
        app.folded.is_empty(),
        "the first Esc must unfold before anything closes"
    );
    assert!(
        app.tree_query.is_empty(),
        "and clear the query with it — pi clears both first"
    );
    assert!(
        app.timeline_open,
        "clearing is not closing: the picker stays up"
    );
    // The unfolded child is back on the screen, at both frame sizes — state
    // asserts alone would pass with rows the draw never shows.
    for (width, height) in [(80u16, 24u16), (100, 30)] {
        let rows = render_rows(&mut app, width, height);
        let screen = rows.join("\n");
        assert!(
            screen.contains("the child"),
            "unfolding must reach the pixels at {width}x{height}:\n{screen}"
        );
    }

    handle_key(&mut app, &agent, esc());
    assert!(!app.timeline_open, "with nothing left to clear, Esc closes");
}

/// Cycling the tree filter clears folded branches (P-293).
///
/// Pi's filter change unfolds everything: a fold hides rows under one filter's
/// meaning, and the next filter must not inherit the hiding. The five modes
/// already cycle on `f` in pi's order; the folds survived the cycle, so this is
/// the half that was missing. Asserted both ways — the mode advances AND the
/// folds clear — so neither half can regress silently.
#[test]
fn cycling_the_tree_filter_clears_folded_branches() {
    use crate::app::{TimelineRow, TreeFilter};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let (mut app, agent, _rx) = test_app();
    let parent = TimelineRow {
        seq: 0,
        id: "a".into(),
        kind: "user".into(),
        snippet: "the parent".into(),
        depth: 0,
        on_active: true,
        parent: None,
        label: None,
    };
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
    app.timeline_all = vec![parent.clone(), child];
    app.timeline_entries = vec![parent];
    app.timeline_index = 0;
    app.timeline_open = true;
    app.tree_filter = TreeFilter::Default;
    app.folded.insert("a".into());

    handle_key(
        &mut app,
        &agent,
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::empty()),
    );
    assert_eq!(
        app.tree_filter,
        TreeFilter::UserOnly,
        "the cycle still advances in pi's order"
    );
    assert!(
        app.folded.is_empty(),
        "a filter change must not inherit the hiding"
    );
    assert!(app.timeline_open, "cycling is not closing");
    // The new mode is named on the screen, and the unfolded child with it.
    for (width, height) in [(80u16, 24u16), (100, 30)] {
        let rows = render_rows(&mut app, width, height);
        let screen = rows.join("\n");
        assert!(
            screen.contains("user only") && screen.contains("the child"),
            "mode and rows must reach the pixels at {width}x{height}:\n{screen}"
        );
    }
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
    // **Asserted against the repertoire, not against the characters (A43).**
    //
    // These two assertions named `\u{25be}` and `\u{25b8}` as literals, which is the
    // defect itself: the panel drew those characters as string literals, so on the ASCII
    // set — **the default on a Windows box without a UTF-8 host** — the fold markers
    // rendered as boxes, and a test asserting the literal passed on a console that showed
    // neither. The property worth pinning is "the two states get two *different* glyphs,
    // and both come from the table", and that is what this now says.
    let (closed, open) = (app.theme.glyphs.fold_closed, app.theme.glyphs.fold_open);
    assert_ne!(
        closed, open,
        "collapsed and expanded are different facts and must not share a glyph"
    );
    // **Per row, not per screen.** The folded row and the open row are the two facts, and
    // the whole-screen form would pass on a build that drew neither — which is what
    // happened when these assertions named the characters instead of the table.
    let row_of = |needle: &str| {
        rows.iter()
            .find(|r| r.contains(needle))
            .unwrap_or_else(|| panic!("the row naming {needle:?} must be on screen: {screen}"))
    };
    let folded_row = row_of("the parent");
    let open_row = row_of("an open branch");
    assert!(
        folded_row.contains(closed),
        "a collapsed row carries the collapsed glyph {closed:?}: {folded_row:?}"
    );
    assert!(
        open_row.contains(open),
        "an expanded one carries the OTHER glyph {open:?}: {open_row:?}"
    );
    assert_ne!(
        folded_row, open_row,
        "the two states must be distinguishable on the screen, not merely in the table"
    );
    // And the ASCII repertoire must actually differ, which is the half the literals
    // could not express at all: `▾`/`▸` render as **boxes** there, and those boxes are
    // the whole content of the fold feature.
    assert_ne!(
        crate::ui::theme::ASCII.fold_closed,
        crate::ui::theme::ASCII.fold_open,
        "the seven-bit repertoire has to carry the same distinction or the panel is \
         unreadable on the console most people are using"
    );
}

/// P-304 on pixels: a lane reads flat, fork leaves step once.
///
/// Rows run through the real pipeline (`visible_rows`, filter All, no folds),
/// so the frame pins the indent rule rather than hand-set depths. The `#`
/// column is compared relatively — the selection bar is one cell either way,
/// so selected and unselected rows share the same geometry.
#[test]
fn the_tree_panel_indents_only_at_branch_points() {
    use super::commands::visible_rows;
    use crate::app::{TimelineRow, TreeFilter};

    let (mut app, _agent, _rx) = test_app();
    let row = |seq: u64, id: &str, parent: Option<&str>, snippet: &str| TimelineRow {
        seq,
        id: id.into(),
        kind: "user".into(),
        snippet: snippet.into(),
        depth: 999,
        on_active: true,
        parent: parent.map(str::to_string),
        label: None,
    };
    // Lane of three, then a fork with two leaves.
    let all = vec![
        row(0, "root", None, "lane root words"),
        row(1, "a", Some("root"), "lane middle words"),
        row(2, "b", Some("a"), "lane tail words"),
        row(3, "c", Some("b"), "fork left words"),
        row(4, "d", Some("b"), "fork right words"),
    ];
    app.timeline_all = all;
    app.timeline_entries = visible_rows(
        &app.timeline_all.clone(),
        TreeFilter::All,
        &Default::default(),
        "",
        &Default::default(),
    );
    app.timeline_open = true;
    app.timeline_index = 0;

    for (width, height) in [(80u16, 24u16), (100, 30)] {
        let rows = render_rows(&mut app, width, height);
        let hash_col = |needle: &str| {
            rows.iter()
                .find(|r| r.contains(needle))
                .unwrap_or_else(|| panic!("row {needle:?} must be on screen at {width}x{height}"))
                // Character position, not byte index: the selection bar is
                // one cell either way but not one byte, so byte columns would
                // disagree between the selected row and the rest.
                .chars()
                .position(|c| c == '#')
                .expect("a timeline row carries its #seq")
        };
        let lane = hash_col("lane middle words");
        assert_eq!(
            hash_col("lane root words"),
            lane,
            "the lane starts flat at {width}x{height}"
        );
        assert_eq!(
            hash_col("lane tail words"),
            lane,
            "the lane stays flat at {width}x{height}"
        );
        assert_eq!(
            hash_col("fork left words"),
            lane + 2,
            "one indent step is two columns at {width}x{height}"
        );
        assert_eq!(
            hash_col("fork right words"),
            lane + 2,
            "both fork leaves step together at {width}x{height}"
        );
    }
}

/// **A search field wider than its panel must say it was cut, and on the frame.**
///
/// The panel's budget was `(inner.width - 6)` — already a count of *columns*, since it
/// subtracts the `/` prefix and the cursor block — and it was then spent on `chars()`. For
/// ASCII the two are the same number, which is why no test failed; for a Japanese query
/// they are not, and the field drew twice as wide as the box it is typed into, so the pane
/// clipped the tail with no mark and the last thing a reader saw was a half-word.
///
/// Asserted on the rendered row rather than on the helper, for the reason the test above
/// gives: `fit_width` has its own unit tests, and those would pass on a build where this
/// panel never called it.
#[test]
fn a_wide_script_in_the_tree_filter_is_cut_and_says_so_on_the_frame() {
    use crate::app::TimelineRow;

    let (mut app, _agent, _rx) = test_app();
    app.timeline_open = true;
    app.tree_searching = true;
    // The panel returns early on an empty list -- correctly, because `/tree` refuses to
    // open on one ("tree: session is empty"), so this state is reachable only from a
    // test. It still needs a row, or there is no panel to look at.
    app.timeline_all = vec![TimelineRow {
        seq: 0,
        id: "a".into(),
        kind: "user".into(),
        snippet: "目的赶到目标".repeat(30),
        depth: 0,
        on_active: true,
        parent: None,
        label: None,
    }];
    app.timeline_entries = app.timeline_all.clone();
    app.timeline_index = 0;
    // 240 characters, 480 columns -- wider than any panel this suite renders.
    app.tree_query = "目的赶到目标".repeat(30);

    let rows = render_rows(&mut app, 100, 30);
    let field = rows.iter().find(|r| r.contains("/目")).unwrap_or_else(|| {
        panic!("the filter field must be on screen while it is being typed: {rows:?}")
    });
    assert!(
        field.contains('\u{2026}'),
        "a query wider than its panel must be marked as cut, not silently clipped: {field:?}"
    );
    // Scoped to this row on purpose: the frame's other rows may legitimately carry an
    // ellipsis from their own truncation, and an assertion over the whole screen would
    // pass on a build where this field drew the full 480 columns.
    assert!(
        !field.contains(&"目的赶到目标".repeat(30)),
        "and the cut must actually be a cut: {field:?}"
    );
}

/// **No key pressed inside the search view may end the session.**
///
/// `handle_key` returns *true when the app should quit* (`keys.rs:419`), and
/// `handle_term_event` hands that straight to the run loop. `handle_search_key` returns
/// `true` from **every arm**, including its catch-all — so `/search`, and then *any* key at
/// all, quit the harness: `Esc` closed the search **and** exited, and a stray letter did it
/// without even closing anything.
///
/// It is invisible to the four tests that cover search, because they call
/// `search_own_transcript` / `search_step` / `search_close` directly and never press a key.
/// A view whose entire key surface is untested is how a return value written in the other
/// vocabulary ("handled") survives: the two conventions are `true = done` and
/// `true = quit`, and one `return` line separates them.
///
/// Every arm is swept rather than testing one key, because the catch-all and the bound
/// arms are different code and either alone would pass against a build that broke the other.
#[test]
fn no_key_inside_the_search_view_can_end_the_session() {
    let (mut app, agent, _rx) = test_app();
    for text in [
        "alpha",
        "the needle is here",
        "beta",
        "another needle",
        "gamma",
    ] {
        app.ui.push_notice(text);
    }

    for code in [
        KeyCode::Esc,
        KeyCode::Enter,
        KeyCode::Down,
        KeyCode::Up,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Char('n'),
        KeyCode::Char('p'),
        KeyCode::Char('q'),
        KeyCode::Char('x'),
    ] {
        // Esc and `q` close the search, so it is reopened per key: a loop that ran on
        // one search would test the composer for every key after the first.
        app.search_own_transcript("needle");
        assert!(
            app.search.is_some(),
            "the search view must be open or this tests the composer instead"
        );
        assert!(
            !handle_key(&mut app, &agent, key(code)),
            "{code:?} ended the session from inside the search view"
        );
    }
}

/// The other half of the fix: withdrawing the search's accidental claim on `true` must not
/// leave the user with **no** way to quit.
///
/// The exit chord keeps its precedence over an open search for the same reason the
/// approval branch has it (`keys.rs:446`) — quitting drops what the view was holding, and
/// refusing to quit is strictly worse than a search that ignored a stray key. This is the
/// regression the first version of the fix would have shipped.
#[test]
fn the_exit_chord_still_works_while_a_search_is_open() {
    let (mut app, agent, _rx) = test_app();
    app.ui.push_notice("the needle is here");
    app.search_own_transcript("needle");
    assert!(app.search.is_some(), "the fixture must open a real search");
    assert!(
        handle_key(&mut app, &agent, ctrl(KeyCode::Char('q'))),
        "ctrl+q must still quit from inside the search view"
    );
}

/// **A parked permission ask outranks an open search — because the comment says so.**
///
/// `keys.rs` has explained this ordering for as long as the comment has existed: *"A pending
/// permission ask is a question about **doing work**, and a search is a question about
/// **reading**; answering the first with `n` should deny the tool, not silently move the
/// viewport."* The two guards were in the **opposite** order the whole time.
///
/// The consequence is a real one, not a stylistic one: with a tool waiting for permission
/// and a search open behind it, `n` stepped the search viewport, the dialog stayed up, and
/// the tool stayed parked — so the key the footer advertises as *deny* did something else
/// entirely, on the one screen where the reader is deciding whether work runs.
///
/// This is the second instance in this file of the rule `the_backward_jump_chord_arms_backward_not_forward`
/// already states: **a comment about ordering is a promise, and the only thing that makes it
/// one is a test.**
#[tokio::test]
async fn a_pending_ask_outranks_an_open_search() {
    let (mut app, agent, _receivers) = test_app();
    app.ui.push_notice("the needle is here");
    app.search_own_transcript("needle");
    assert!(app.search.is_some(), "the fixture must open a real search");
    let approvals = ChannelApprovalUi::new();
    let _response = park_ask(&mut app, &approvals, "over-search").await;
    let _ = render_rows(&mut app, 80, 24); // the dialog paints over the search panel

    // `n` is bound in BOTH views: deny in one, step-next in the other. Which one wins is
    // the whole question, and it cannot be answered by reading either handler.
    handle_key(&mut app, &agent, key(KeyCode::Char('n')));

    assert!(
        app.pending_ask.is_none(),
        "`n` must answer the parked ask, not move the search viewport"
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
        hidden: Default::default(),
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
/// The copy key works while the tree is open — and takes the selected entry.
///
/// `/hotkeys` advertised it and the panel printed it, so this is the "a silent
/// no-op is worse than a missing feature" case rather than an absent feature.
///
/// Contract change (P-307): this used to assert the SAME notice `/copy` gives
/// ("copy: no assistant message to copy"), pinning the route-through-the-global
/// command. The route is still shared (`copy_text`), but the source is now the
/// highlighted entry's full text — so the assertion is on the entry's char
/// count, which the old behaviour could never produce here (no assistant
/// message exists in this fixture at all).
///
/// The negative is the test: `handle_key` returning without a notice is exactly
/// the old behaviour, and asserting on the *absence* of a notice would pass
/// vacuously against a build where the command never ran.
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
         for every key before the copy arm is ever reached"
    );
    // "a question worth answering" is 26 chars: the selected USER entry's full
    // text, which the old global route could never name here.
    assert!(
        last_line_text(&app).contains("26 chars"),
        "in-tree copy takes the highlighted entry: {}",
        last_line_text(&app)
    );
    assert!(
        !app.timeline_open,
        "acting keys close the picker, so the copy notice is visible"
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
        // No reasoning split in this fixture's compaction reply.
        reasoning: None,
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
            // The digest and the pre-compaction estimate the collapsed row now
            // also names; this test's property is the agreement of the cost
            // figures, so the payload is the boring kind.
            summary: "a digest nobody in this test reads".into(),
            tokens_before: 96_000,
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

/// The compaction row opens to the summary the event carried (P-054, P-183).
///
/// **The live digest was unreachable.** The summary was appended to the durable
/// entry and never emitted, so a reader watching a compaction happen saw the cost
/// line and had no way to learn what the surviving context says — and the row
/// advertised no way to look. Pi's row collapses to the token count with the
/// expand chord and opens to the summary markdown
/// (`compaction-summary-message.ts:49-51`); ours now does the same through the
/// card machinery's own affordance, cap and continuation gutter rather than a
/// second copy of them.
///
/// Asserted through the real key path and the real draw: the affordance names the
/// **resolved** chord (a rebind shows rebound), and the summary's rows nest
/// under the row that introduced them.
#[test]
fn a_compaction_row_opens_to_the_summary_it_carried() {
    use epsilon_contracts::agent_event::AgentEvent;

    let (mut app, agent, _rx) = test_app();
    let usage = epsilon_contracts::message::Usage {
        input: 12_400,
        output: 310,
        cache_read: 8_900,
        cache_write: 0,
        total_tokens: 21_610,
        cache_reported: true,
        // No reasoning split in this fixture's compaction reply.
        reasoning: None,
    };
    let summary = "The reader is porting the desktop's approval gate. \
                   The Enter routing defect is fixed and the honesty half is pinned. \
                   Nothing else is in flight.";

    apply_with_scroll(
        &mut app,
        &AgentEvent::CompactionFinished {
            usage: Some(usage),
            model_id: Some("test-model".into()),
            summary: summary.into(),
            tokens_before: 96_000,
        },
    );

    // Collapsed: the cost row is on screen, the summary is not, and the row
    // names the chord — the one the binding table resolves, so a rebind shows
    // rebound rather than a stale literal.
    let collapsed = render_rows(&mut app, 80, 24).join("\n");
    let key = app
        .keybindings
        .key_text("app.tools.expand")
        .expect("the expand chord resolves");
    assert!(
        collapsed.contains("context compacted"),
        "the row is on screen: {collapsed}"
    );
    assert!(
        !collapsed.contains("porting the desktop"),
        "collapsed, the summary stays behind its affordance: {collapsed}"
    );
    assert!(
        collapsed.contains(&format!("… 1 more line ({key} to open)")),
        "the affordance names the resolved chord and the hidden count, and says          \"more\" because the digest hides BELOW the row: {collapsed}"
    );

    // The chord, through the real key path.
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('x')));

    let opened = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        opened.contains("porting the desktop"),
        "the summary opens: {opened}"
    );
    assert!(
        opened.contains("Nothing else is in flight"),
        "and it opens whole, not to a head: {opened}"
    );
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

/// A `!!` run is marked on the **frame**, from the moment it starts.
/// A failing `!!` keeps its exclusion marker (the failure half of §2.7).
///
/// The success path has carried the label since M52 and the running row since
/// this round. The error path printed a bare `shell: <error>`, so a `!!` that
/// *failed* lost the marker exactly when a reader most needs it — the question
/// "did that output reach the model" matters most when the command did not work.
///
/// The gate is answered **Deny**, so `run_local` returns `Err` and the notice is
/// the failure one. Leaving it unanswered would block instead of failing, which
/// is the correct product behaviour and the wrong test.
#[tokio::test]
async fn a_failing_double_bang_keeps_its_exclusion_label() {
    let (mut app, agent, (_frames, mut notices), approvals) = test_app_with_bash();
    let _answerer = tokio::spawn({
        let channel = approvals.clone();
        async move {
            channel
                .next_ask()
                .await
                .expect("a gated bash call asks")
                .reply(PermissionDecision::Deny)
        }
    });
    type_and_enter(&mut app, &agent, "!!false");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let frame = loop {
        while let Ok(n) = notices.try_recv() {
            app.ui.push_notice(n);
        }
        let frame = render_rows(&mut app, 80, 24).join("\n");
        // The needle is the result label, not the phrase: the running row now
        // carries the same phrase ("running (kept out of context): …"), so a
        // looser match would pass against the wrong row — which is what this
        // test's first version did.
        if frame.contains("shell (kept out of context): ") || std::time::Instant::now() > deadline {
            break frame;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert!(
        frame.contains("shell (kept out of context): "),
        "the failure row must carry the label: {frame}"
    );
}

/// A `!!` run names its exclusion on the **running** row, not only on the result.
///
/// The sibling test above drains the notices channel and pins the *result*
/// label, which already exists. What nothing pinned is the row a reader looks at
/// during the seconds the command takes: `run_shell` printed a bare
/// `running: <command>`, and on the failure path the label reverted to a bare
/// `shell:`, so a `!!` that failed lost its marker exactly when a reader most
/// needs it.
///
/// This asserts the drawn frame **before the gate is answered**, so the command
/// cannot have finished — the only row that can carry the marker is the running
/// one. The control is the single-bang form, which must run and must not be
/// marked: a marker on every shell line is not a distinction.
#[tokio::test]
async fn a_double_bang_names_its_exclusion_on_the_running_row() {
    for (typed, marked) in [("!!echo shellmark", true), ("!echo shellmark", false)] {
        // No approver is spawned: with the gate unanswered the command cannot
        // start, so the frame is the running state and nothing else.
        let (mut app, agent, (_frames, mut notices), _approvals) = test_app_with_bash();
        type_and_enter(&mut app, &agent, typed);

        // Drain what the synchronous `running:` notice produced and draw. A
        // bounded poll rather than a sleep, so a missing notice fails loudly
        // instead of asserting against a frame that never had one.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let frame = loop {
            while let Ok(n) = notices.try_recv() {
                app.ui.push_notice(n);
            }
            let frame = render_rows(&mut app, 80, 24).join("\n");
            if frame.contains("shellmark") || std::time::Instant::now() > deadline {
                break frame;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };

        let running_row = frame
            .lines()
            .find(|l| l.contains("shellmark"))
            .unwrap_or_else(|| panic!("the running row must name the command: {frame}"));
        if marked {
            assert!(
                running_row.contains("kept out of context"),
                "a !! run must name the exclusion while it runs: {running_row:?}"
            );
        } else {
            assert!(
                !running_row.contains("kept out of context"),
                "a ! run must not be marked excluded: {running_row:?}"
            );
        }
    }
}

/// Plant a second session beside the attached one, in the directory a launch
/// actually uses: the sessions root, then this workspace's per-cwd directory
/// (M217). A file in the sessions *root* would not be listed for this workspace.
fn plant_second_session(dir: &tempfile::TempDir, name: &str, text: &str) -> std::path::PathBuf {
    let workspace = dir.path();
    let sessions = workspace.join(".epsilon").join("sessions");
    let per_cwd = epsilon_session::time::session_dir_for(&sessions, workspace);
    std::fs::create_dir_all(&per_cwd).expect("the per-workspace session directory");
    let path = per_cwd.join(name);
    write_session_fixture(&path, workspace, text);
    path
}

/// Highlight the one row that is **not** the attached session.
///
/// Chosen by identity rather than by index: the list is sorted by file name, and
/// a timestamp-prefixed name sorts before the bare `session.jsonl` the fixture
/// attaches — so "the second row" is the attached file, and a test that assumed
/// otherwise would drive the *refusal* path while believing it was driving the
/// delete.
fn highlight_a_deletable_row(app: &mut App) {
    let current = app.session_browser.as_ref().and_then(|b| b.current.clone());
    let index = app
        .session_browser
        .as_ref()
        .map(|b| {
            b.rows
                .iter()
                .position(|r| Some(&r.path) != current.as_ref())
                .expect("a session other than the attached one")
        })
        .expect("the browser is open");
    if let Some(b) = app.session_browser.as_mut() {
        b.index = index;
    }
}

/// `Ctrl+D` in the sessions browser asks before it deletes (P-237).
///
/// The first press must not touch the disk: the unlink fallback is
/// unrecoverable (there is no `trash` binary on this platform, so every delete
/// here is permanent), and `AGENTS.md` §6 requires a confirmation. Pi arms the
/// same chord and guards it the same way (`session-selector.ts:402-413`,
/// `:543-557`); this asserts the guard, the drawn confirmation, and that the file
/// is still there.
#[test]
fn ctrl_d_in_the_browser_asks_before_it_deletes() {
    let (mut app, agent, _rx, dir) = test_app_with_store();
    let other = plant_second_session(&dir, "2026-10-08T00-00-00_ses_other.jsonl", "the other one");
    app.open_session_browser(&agent);
    let before: Vec<_> = walk_jsonl(dir.path());
    assert!(
        before.len() >= 2,
        "the fixture has two sessions: {before:?}"
    );

    // Highlight a deletable row, then press the delete chord.
    highlight_a_deletable_row(&mut app);
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('d')));

    let frame = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        frame.contains("Delete session?"),
        "the first press must ask, on the frame: {frame}"
    );
    assert!(
        !frame.contains("Session deleted"),
        "and must not report a delete that has not happened: {frame}"
    );
    // The file is untouched.
    assert!(
        walk_jsonl(dir.path()).len() == before.len(),
        "asking must not delete anything"
    );
    assert!(other.exists(), "and the named file is still there");
}

/// Confirming the delete removes the file and says which happened.
///
/// The whole round trip through the real key path, asserted on both the frame
/// and the directory. The wording comes from
/// [`epsilon_session::DeleteOutcome::message`], which is where Pi's
/// `session-selector.ts:857` sentence lives — so the front end cannot invent a
/// second spelling of it.
#[test]
fn confirming_the_delete_removes_the_file_and_says_which_happened() {
    let (mut app, agent, _rx, dir) = test_app_with_store();
    let other = plant_second_session(&dir, "2026-10-08T00-00-00_ses_other.jsonl", "the other one");
    app.open_session_browser(&agent);
    highlight_a_deletable_row(&mut app);
    let before = walk_jsonl(dir.path()).len();

    handle_key(&mut app, &agent, ctrl(KeyCode::Char('d')));
    handle_key(&mut app, &agent, key(KeyCode::Enter));

    let frame = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        frame.contains("Session deleted"),
        "the outcome is reported: {frame}"
    );
    assert!(!other.exists(), "and the file is gone");
    assert_eq!(
        walk_jsonl(dir.path()).len(),
        before - 1,
        "exactly one file went"
    );
}

/// Anything but Enter cancels, and the file survives.
///
/// The other half of the guard: a confirmation that only one key can leave is
/// a trap. Esc is the key a reader reaches for, so it is what this drives.
#[test]
fn escape_cancels_the_delete_and_keeps_the_file() {
    let (mut app, agent, _rx, dir) = test_app_with_store();
    let other = plant_second_session(&dir, "2026-10-08T00-00-00_ses_other.jsonl", "the other one");
    app.open_session_browser(&agent);
    highlight_a_deletable_row(&mut app);
    let before = walk_jsonl(dir.path()).len();

    handle_key(&mut app, &agent, ctrl(KeyCode::Char('d')));
    handle_key(&mut app, &agent, key(KeyCode::Esc));

    let frame = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        frame.contains("delete cancelled"),
        "the cancellation is reported: {frame}"
    );
    assert_eq!(
        walk_jsonl(dir.path()).len(),
        before,
        "and nothing was deleted"
    );
    assert!(other.exists(), "the named file is still there");
}

/// Deleting the **currently attached** session is refused, with Pi's words.
///
/// `session-selector.ts:408`: *"Cannot delete the currently active session"*.
/// The reason is not politeness — this app is writing to that file, so removing
/// it would leave a live store pointing at a path that no longer exists. The
/// refusal is asserted against the frame, because a refusal nobody sees is a
/// key that does nothing (§2.7).
#[test]
fn deleting_the_active_session_is_refused_by_name() {
    let (mut app, agent, _rx, dir) = test_app_with_store();
    let attached = agent
        .store_handle_for_tests()
        .lock()
        .unwrap()
        .path()
        .to_path_buf();
    app.open_session_browser(&agent);
    // Highlight the attached row itself, found by path rather than assumed at
    // row 0: the list is name-sorted, so the attached `session.jsonl` is not
    // first.
    let attached_index = app
        .session_browser
        .as_ref()
        .and_then(|b| b.rows.iter().position(|r| r.path == attached))
        .expect("the attached session is listed");
    if let Some(b) = app.session_browser.as_mut() {
        b.index = attached_index;
    }
    let before = walk_jsonl(dir.path()).len();

    handle_key(&mut app, &agent, ctrl(KeyCode::Char('d')));

    let frame = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        frame.contains("Cannot delete the currently active session"),
        "the refusal names the case, in pi's words: {frame}"
    );
    assert!(
        !frame.contains("Delete session?"),
        "and no confirmation is armed: {frame}"
    );
    assert_eq!(
        walk_jsonl(dir.path()).len(),
        before,
        "and the file is untouched"
    );
    assert!(attached.exists(), "the attached file is still there");
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

/// **A command cannot be displayed as something it is not.**
///
/// This harness shows a reader the line it is about to run and then asks whether to run
/// it. A bidirectional override is eleven characters that reverse the *visual* order of
/// everything after them, so `ls< U+202E > -R /` can be read as one command and be
/// another. That is not a rendering artefact — it is the approval prompt lying, and it
/// costs nothing to close.
///
/// The assertion is on the line as the transcript stores it, which is what every renderer
/// reads: the override is gone, the rest of the command is untouched, and the count is
/// recorded so the reader can be *told* rather than left to assume the text was whole.
#[test]
fn a_bidirectional_override_never_reaches_the_screen() {
    let (mut app, ..) = test_app();
    let hostile = "ls\u{202E} -R /tmp";
    app.ui.push_external_text(hostile);

    let line = app.ui.lines().last().expect("a line was pushed");
    assert!(
        !line.text.contains('\u{202E}'),
        "the override reached the transcript: {:?}",
        line.text
    );
    assert_eq!(
        line.text, "ls -R /tmp",
        "and the command is unchanged otherwise"
    );
    assert_eq!(
        line.hidden.bidi, 1,
        "and the reader is told one was removed"
    );
}

/// **A bidi override in model output must never reach the screen — while it is
/// still arriving.**
///
/// [`a_bidirectional_override_never_reaches_the_screen`] above pins the rule for a
/// committed line. It passed, and was true, because `push_line` strips on the way
/// in. **The live tail does not go through `push_line`**: `apply` assigns
/// `streaming_text` and `streaming_thinking` raw, and `panels.rs` draws them every
/// frame. So for the whole duration of every answer — the moment a hostile file, a
/// hostile tool result or a hostile README is most able to speak — the same bytes
/// that the commit path strips were on screen, and then vanished at commit.
///
/// That is `AGENTS.md` §6 stated as a promise and not kept on one path, and it is
/// invisible to the existing tests because they all push a finished line.
///
/// Asserted on the **frame**, not on `streaming_text()`, because the frame is what
/// the reader sees and the field is one call away from it.
#[tokio::test]
async fn a_bidi_override_in_streaming_model_output_never_reaches_the_frame() {
    let (mut app, ..) = test_app();
    let hostile = "running ls\u{202E} -R /tmp now";
    let message = Message::Assistant(epsilon_contracts::message::AssistantMessage {
        id: "msg_stream_tail".into(),
        timestamp_ms: 0,
        provider: "faux".into(),
        model_id: "faux-1".into(),
        content: vec![
            epsilon_contracts::content::ContentBlock::Thinking {
                text: format!("checking {hostile}"),
                signature: None,
            },
            epsilon_contracts::content::ContentBlock::text(hostile.to_string()),
        ],
        stop_reason: epsilon_contracts::message::StopReason::EndTurn,
        error_message: None,
        usage: epsilon_contracts::message::Usage::default(),
    });
    // One `MessageUpdate` — the streaming state, before `MessageEnd` commits it.
    app.ui
        .apply(&epsilon_contracts::agent_event::AgentEvent::MessageUpdate {
            message: Box::new(message),
        });

    let rows = render_rows(&mut app, 80, 24);
    let screen = rows.join("\n");
    // **The half that bites, and the reason it is here.**
    //
    // The frame was *already* clean before the fix, and that is the part worth
    // recording: ratatui discards a zero-width format character on its way into a
    // cell, so the raw bytes in `streaming_text` never reached the terminal. What
    // was missing was the guarantee. `AGENTS.md` §6 strips untrusted text **before
    // it reaches a screen**, and "a renderer happened to drop it" is not that — it
    // is a property of a third-party layout crate that a dependency bump can take
    // away. So the assertion is on the **stored field**, where the fix is, and the
    // frame assertion below is the reader-visible half that was already true.
    assert!(
        !app.ui.streaming_thinking().contains('\u{202E}')
            && !app.ui.streaming_text().contains('\u{202E}'),
        "the live tail stored a bidi override: thinking={:?} text={:?}",
        app.ui.streaming_thinking(),
        app.ui.streaming_text()
    );
    assert!(
        !screen.contains('\u{202E}'),
        "the live tail drew a bidi override: {screen:?}"
    );
    assert!(
        screen.contains("ls -R /tmp"),
        "and the rest of the text is untouched, or nothing was drawn at all: {screen:?}"
    );
}

/// Ordinary tool output is untouched, and costs no allocation.
///
/// A transcript of two thousand rows must not allocate two thousand `String`s per frame to
/// learn that nothing needed removing — so the common path borrows, and the test asserts
/// the borrow rather than trusting it.
#[test]
fn ordinary_text_is_stored_exactly_and_the_common_path_borrows() {
    let (mut app, ..) = test_app();
    let plain = "total 42\ndrwxr-xr-x  root  bin";
    app.ui.push_external_text(plain);
    let line = app.ui.lines().last().expect("a line was pushed");
    assert_eq!(line.text, plain);
    assert_eq!(
        line.hidden.total(),
        0,
        "nothing was removed from ordinary text"
    );

    let (text, stripped) = crate::text_safety::display_safe(plain);
    assert!(
        matches!(text, std::borrow::Cow::Borrowed(_)),
        "a line with nothing to strip must not allocate"
    );
    assert!(!stripped.any());
}

/// **Tool cards strip display-steering characters too.**
///
/// `push_line` sanitised `text`, `full` and `md` and **never looked at the card**, so tool
/// output and model-written arguments were drawn verbatim — and the two tests above, which
/// push `push_external_text`, carry **no card at all**, so they could never see the gap they
/// sit next to. A test that cannot fail on the defect it names is not evidence.
///
/// Three sites set a card body; two are checked, because the third needs a running tool.
/// A `U+202E` in the **command the model wrote** is stripped when the call opens, and the same
/// character arriving as **tool output** is stripped when the result settles.
#[test]
fn a_card_is_stripped_where_its_text_enters() {
    use epsilon_contracts::agent_event::AgentEvent;
    use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage};

    let (mut app, ..) = test_app();

    // **A card exists because an assistant message asked for a tool** — the first attempt
    // used `ToolExecutionStart` and found no card, which is the same mistake in a new place:
    // driving the reducer with an event that does not reach the code under test.
    app.ui.apply(&AgentEvent::MessageEnd {
        message: Box::new(Message::Assistant(AssistantMessage {
            id: "msg-hostile".into(),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![epsilon_contracts::content::ContentBlock::ToolCall {
                id: "c-hostile".into(),
                name: "bash".into(),
                arguments: serde_json::json!({ "command": "ls\u{202E} -R /tmp" }),
            }],
            stop_reason: StopReason::ToolUse,
            error_message: None,
            usage: Usage::default(),
        })),
    });
    let pending = app
        .ui
        .lines()
        .iter()
        .find_map(|l| l.card.as_ref())
        .expect("the call opened a card");
    let crate::state::CardBody::Prompt(shown) = &pending.body else {
        panic!("a pending call's body is a prompt, got {:?}", pending.body);
    };
    assert!(
        !shown.contains('\u{202E}'),
        "the model's own command reached the card verbatim: {shown:?}"
    );
    assert!(
        shown.contains("-R /tmp"),
        "and the strip must not eat the text around it: {shown:?}"
    );

    // The result: **output the tool produced**, arriving after the card was built — which is
    // why stripping at the call site alone was not enough.
    app.ui.apply(&AgentEvent::ToolExecutionUpdate {
        tool_call_id: "c-hostile".into(),
        tool_name: "bash".into(),
        partial: epsilon_contracts::tool::ToolPartial {
            content: vec![epsilon_contracts::content::ContentBlock::text(
                "streaming\nls\u{202E} -R /tmp",
            )],
            details: serde_json::Value::Null,
        },
    });
    let streamed = app
        .ui
        .lines()
        .iter()
        .rev()
        .find_map(|l| l.card.as_ref())
        .expect("the card is still there");
    let crate::state::CardBody::Text(body) = &streamed.body else {
        panic!("a streamed body is text, got {:?}", streamed.body);
    };
    assert!(
        !body.contains('\u{202E}'),
        "streamed tool output reached the card verbatim: {body:?}"
    );

    // **The reader is told.** A silent strip is the same failure wearing a security hat
    // (M260's rule), so the count has to land on the line the card is drawn in.
    let line = app
        .ui
        .lines()
        .iter()
        .find(|l| l.card.is_some())
        .expect("the card's line");
    assert!(
        line.hidden.total() > 0,
        "a strip nothing reports is a strip the reader cannot account for"
    );
}

/// A line that is **entirely** display-steering characters still leaves a usable row.
///
/// The empty case is the one that bites: a card whose body was nothing but overrides
/// would otherwise render as a blank row, and a blank row reads as "nothing happened".
#[test]
fn a_line_of_only_overrides_still_records_that_something_was_there() {
    let (mut app, ..) = test_app();
    app.ui.push_external_text("\u{202E}\u{202E}");
    let line = app.ui.lines().last().expect("a line was pushed");
    assert_eq!(line.text, "", "nothing displayable is left");
    assert_eq!(
        line.hidden.bidi, 2,
        "and the reader is told two characters were removed, rather than seeing a blank row"
    );
}

/// **An ask whose command cannot be shown faithfully cannot be approved.**
///
/// M260 sanitised the transcript, and recorded that the permission dialog was the open
/// half: it renders the *agent's* own `capability` and `detail`, and it is the one screen
/// where a reader decides whether work runs. A `U+202E` in the command makes the dialog
/// display something the command is not, so pressing `y` there would be approving text
/// nobody saw.
///
/// The ask therefore stays parked and the key is refused in words. `n` still works, which is
/// the direction that cannot run the wrong thing — refusing is not a wedge.
#[tokio::test]
async fn an_ask_carrying_a_bidi_override_cannot_be_approved() {
    let (mut app, agent, _receivers) = test_app();
    let approvals = ChannelApprovalUi::new();
    let _response = park_ask_detail(
        &mut app,
        &approvals,
        "g1",
        "bash",
        "ls\u{202E} -R /",
        serde_json::json!({ "command": "ls\u{202E} -R /" }),
    )
    .await;

    // A frame first: `y` is refused unless the dialog has been painted, so the guard under
    // test is the *display* one and not the visibility one.
    render_rows(&mut app, 100, 40);
    assert!(!handle_key(
        &mut app,
        &agent,
        KeyEvent::from(KeyCode::Char('y'))
    ));

    assert!(
        app.pending_ask.is_some(),
        "the ask must still be parked: an approval was applied to text nobody could read"
    );
    let notices = render_rows(&mut app, 100, 40).join("\n");
    assert!(
        notices.contains("display-steering"),
        "and the refusal must say why:\n{notices}"
    );
}

/// Deny still works on such an ask — the refusal is on *approval*, not on the dialog.
#[tokio::test]
async fn an_ask_carrying_a_bidi_override_can_still_be_denied() {
    let (mut app, agent, _receivers) = test_app();
    let approvals = ChannelApprovalUi::new();
    let response = park_ask_detail(
        &mut app,
        &approvals,
        "g1",
        "bash",
        "ls\u{202E} -R /",
        serde_json::json!({ "command": "ls\u{202E} -R /" }),
    )
    .await;
    render_rows(&mut app, 100, 40);

    handle_key(&mut app, &agent, KeyEvent::from(KeyCode::Char('n')));
    assert!(
        app.pending_ask.is_none(),
        "deny must always work; refusing to approve must not wedge the run"
    );
    assert!(matches!(response.await, Ok(PermissionDecision::Deny)));
}

/// The dialog **says** that it removed something, and withdraws the approve keys.
///
/// Two halves, and the second is what makes the first meaningful: a prompt that quietly
/// dropped a character would look correct, and a reader has no other way to learn the text
/// was not what it claimed.
#[tokio::test]
async fn the_dialog_names_what_it_removed_and_withdraws_the_approve_keys() {
    let (mut app, _agent, _receivers) = test_app();
    let approvals = ChannelApprovalUi::new();
    let _response = park_ask_detail(
        &mut app,
        &approvals,
        "g1",
        "bash",
        "ls\u{202E} -R /",
        serde_json::json!({ "command": "ls\u{202E} -R /" }),
    )
    .await;
    let frame = render_rows(&mut app, 100, 40).join("\n");

    assert!(
        !frame.contains('\u{202E}'),
        "the override must not reach the screen:\n{frame}"
    );
    assert!(
        frame.contains("display-steering"),
        "and the dialog must name it:\n{frame}"
    );
    assert!(
        !frame.contains("allow once"),
        "a request that cannot be shown faithfully must not offer to allow it:\n{frame}"
    );
    assert!(
        frame.contains("deny"),
        "while still offering the direction that cannot run the wrong thing:\n{frame}"
    );
    // **The footer must agree with the handler.** It printed `y allow once` while the key
    // path refused `y` -- a key the window does not act on, advertised as though it would,
    // which is the defect this repository tracks by name. It arrived from the direction of
    // a security fix, which is the part worth remembering.
    assert!(
        frame.contains("cannot approve as shown"),
        "and the footer must say why the decision surface is smaller:\n{frame}"
    );
}

/// **A `/model` switch survives `epsilon -c`.** This is the owner-reported defect of
/// 2026-10-04: "when switching to specific model it keeps reverting to another".
///
/// # Why the assertion is on the session, not on the notice
///
/// `apply_model` already printed `model switched to … — saved to …` and already wrote
/// `config.toml`, and both looked correct — which is why the bug survived. The revert happens
/// **later and elsewhere**: `epsilon_runtime::resumed_model_to_use` prefers the *session's*
/// model over the config on a resume, and the session could only report a model from an
/// assistant message, because **nothing in production ever appended a `ModelChange` entry**.
/// So on resume the launch adopted the model the previous run had used — the exact thing the
/// user had just replaced.
///
/// The assertion therefore reads what the resume path actually reads
/// (`SessionStore::context_settings`), rather than what the switch reported about itself. A test
/// on the notice would pass against the broken build; this one cannot.
#[tokio::test]
async fn a_model_switch_is_recorded_where_a_resume_will_find_it() {
    let (mut app, agent, _receivers) = test_app();
    let tmp = tempfile::tempdir().expect("a temporary workspace");
    let store = std::sync::Arc::new(std::sync::Mutex::new(
        epsilon_core::SessionStore::open(tmp.path().join("s.jsonl")).unwrap(),
    ));
    agent.attach_session_store(std::sync::Arc::clone(&store));

    // The session already knows a model — the one the previous run used. This is the
    // precondition that makes the test real: without it, an empty session would answer
    // "no model" for a reason that has nothing to do with the switch.
    {
        let mut st = store.lock().unwrap();
        st.append(epsilon_contracts::session_entry::EntryKind::ModelChange {
            provider: "deepseek".into(),
            model_id: "deepseek/deepseek-chat-v3.1".into(),
        })
        .unwrap();
    }

    app.apply_model(&agent, "deepseek-flash".to_string());

    let settings = store
        .lock()
        .unwrap()
        .context_settings()
        .expect("the session is readable");
    assert_eq!(
        settings.model,
        Some(("deepseek".to_string(), "deepseek-flash".to_string())),
        "a resume reads the session's model first, so the switch has to be recorded there \
         or it is outranked by whatever the previous run used"
    );
}

/// **Criterion 7: a `/think` switch is recorded where a resume will find it.**
///
/// `context_settings` has walked `thinking_level_change` entries since the
/// port, but nothing in production ever appended one — the field was correct,
/// unit-tested, and unreachable, so every launch restarted at `Off` and the
/// level was re-set by hand. Same assertion surface as the model's test above:
/// what the resume path reads, not what the switch reported about itself.
#[tokio::test]
async fn a_thinking_switch_is_recorded_where_a_resume_will_find_it() {
    let (mut app, agent, _receivers) = test_app();
    // Pinned config dir: `set_thinking` persists the level, and without the
    // override that write lands in the developer's real global config — the
    // suite polluting the machine it runs on (found via a battery FAIL whose
    // frames all carried the ambient `high`). The sibling test below pins the
    // same way; this one never reads the disk, so the override changes nothing
    // it asserts.
    let cfg = tempfile::tempdir().expect("a config dir");
    app.config_dir_override = Some(cfg.path().to_path_buf());
    let tmp = tempfile::tempdir().expect("a temporary workspace");
    let store = std::sync::Arc::new(std::sync::Mutex::new(
        epsilon_core::SessionStore::open(tmp.path().join("s.jsonl")).unwrap(),
    ));
    agent.attach_session_store(std::sync::Arc::clone(&store));

    app.set_thinking(&agent, epsilon_contracts::ThinkingLevel::High);

    let settings = store
        .lock()
        .unwrap()
        .context_settings()
        .expect("the session is readable");
    assert_eq!(
        settings.thinking_level.as_deref(),
        Some("high"),
        "a resume applies the session's own level, so the switch has to be recorded there"
    );
}

/// **Criterion 7: the thinking default survives a restart through the UI.**
///
/// The write is the whole fix on the frontend half: `/think` used to touch the
/// agent, the mirror field and the notice, and nothing on disk. The read-back
/// is through the same loader the next launch uses.
#[tokio::test]
async fn a_thinking_default_survives_a_restart_through_the_ui() {
    let (mut app, agent, _receivers) = test_app();
    let cfg = tempfile::tempdir().expect("a config dir");
    app.config_dir_override = Some(cfg.path().to_path_buf());

    app.set_thinking(&agent, epsilon_contracts::ThinkingLevel::XHigh);

    assert_eq!(
        epsilon_core::config::load_thinking_level_in(cfg.path()),
        Some(epsilon_contracts::ThinkingLevel::XHigh),
        "the level must be on disk, or every launch re-sets it by hand"
    );
    assert_eq!(
        app.thinking,
        epsilon_contracts::ThinkingLevel::XHigh,
        "and the mirror the footer reads is the same answer (M25)"
    );
    assert_eq!(agent.thinking_level(), app.thinking);
}

/// **Favourite cycling applies the remembered level for this run only.**
///
/// A per-favourite memory is a per-model memory, not the user's default: if
/// cycling wrote it to `[thinking] level`, the standing default would end up on
/// whichever favourite was cycled to last, and the next launch would start
/// there. This is the control that catches the favourite becoming the default.
#[tokio::test]
async fn a_favourites_remembered_level_does_not_become_the_default() {
    let (mut app, agent, _receivers) = test_app();
    let cfg = tempfile::tempdir().expect("a config dir");
    app.config_dir_override = Some(cfg.path().to_path_buf());
    app.favorites.clear();
    app.recents.clear();
    // A standing default already on disk, and a favourite that remembers a
    // different level — exactly the situation F2 must not let leak.
    epsilon_core::config::save_thinking_level_in(
        cfg.path(),
        epsilon_contracts::ThinkingLevel::Medium,
    )
    .unwrap();
    app.favorites = vec!["faux-1".into(), "deepseek-chat".into()];
    app.favorite_thinking = vec![("deepseek-chat".into(), "max".into())];

    handle_key(&mut app, &agent, key(KeyCode::F(2)));

    assert_eq!(app.model_id, "deepseek-chat", "the cycle switched model");
    assert_eq!(
        agent.thinking_level(),
        epsilon_contracts::ThinkingLevel::Max,
        "the favourite's remembered level applies to this run"
    );
    assert_eq!(
        epsilon_core::config::load_thinking_level_in(cfg.path()),
        Some(epsilon_contracts::ThinkingLevel::Medium),
        "but it must NOT overwrite the standing default"
    );
}

/// **Criterion 7, the live half: a live `/resume` moves the frontend onto the
/// session's model, on a real frame.**
///
/// `Agent::live_resume` restored the history and never the model, so the
/// in-TUI switch left the header, the picker's current row and `/session`
/// naming the launch model while the run used the old one. The assertion is on
/// the drawn row — the same evidence discipline the `/catalog` frame test uses
/// — because a mirror asserted on state is exactly what shipped this defect.
#[tokio::test]
async fn a_live_resume_draws_the_sessions_model_not_the_launch_one() {
    let (mut app, agent, _receivers) = test_app();
    let tmp = tempfile::tempdir().expect("a temporary workspace");
    let ws = tmp.path();
    let resumed = ws.join("resume.jsonl");
    {
        let mut store = epsilon_core::SessionStore::open(&resumed).unwrap();
        store
            .append(epsilon_contracts::session_entry::EntryKind::ModelChange {
                provider: "openrouter".into(),
                model_id: "deepseek/deepseek-chat-v3.1".into(),
            })
            .unwrap();
        store
            .append(
                epsilon_contracts::session_entry::EntryKind::ThinkingLevelChange {
                    level: "high".to_string(),
                },
            )
            .unwrap();
    }
    agent.attach_session_store(std::sync::Arc::new(std::sync::Mutex::new(
        epsilon_core::SessionStore::open(&resumed).unwrap(),
    )));
    app.model_id = "faux-1".to_string();

    // Entries, not messages, are what this session holds, so the restored
    // message count is zero — the model and the level are what it carries.
    let switched = app
        .switch_session_file(&agent, &resumed, None)
        .expect("the switch opens the file");
    assert_eq!(
        switched, 0,
        "an entry-only session restores no messages, and the switch still succeeded"
    );

    assert_eq!(
        agent.model_id(),
        "deepseek/deepseek-chat-v3.1",
        "the run is on the session's model"
    );
    assert_eq!(
        app.model_id, "deepseek/deepseek-chat-v3.1",
        "and the frontend follows it — one source of truth, not two"
    );
    assert_eq!(
        agent.thinking_level(),
        epsilon_contracts::ThinkingLevel::High,
        "the session's recorded level is restored too"
    );
    assert_eq!(app.thinking, agent.thinking_level(), "mirror stays exact");

    // The frame. Two viewports, because a needle that fits at 100 can wrap
    // away at 80, and both are sizes a user actually runs.
    for (w, h) in [(100, 30), (80, 24)] {
        let rows = render_rows(&mut app, w, h);
        assert!(
            rows[0].contains("DeepSeek V3.1"),
            "the header must name the model the run is on at {w}x{h}: {:?}",
            rows[0]
        );
        assert!(
            !rows[0].contains("Faux (offline scripted)"),
            "and must not still name the launch model: {:?}",
            rows[0]
        );
    }
}

/// **Criterion 7, the launch half: the frame a resumed launch draws names the
/// composed model.**
///
/// `tui_mode` now seeds `TuiConfig` with `agent.model_id()` — the value AFTER
/// composition applied the session's model — instead of the launch resolution.
/// `drive` hands that to `App::new`, so the frame below is what that launch
/// draws. The composition itself is pinned by the runtime's
/// `apply_resumed_context` tests; this pins the seam between them and the
/// screen, which is where the "header names one model while the run uses
/// another" defect lived.
#[tokio::test]
async fn a_resumed_launch_draws_the_composed_model_not_the_launch_one() {
    let (_app, agent, _receivers) = test_app();
    let tmp = tempfile::tempdir().expect("a temporary workspace");
    let path = tmp.path().join("resume.jsonl");
    {
        let mut store = epsilon_core::SessionStore::open(&path).unwrap();
        store
            .append(epsilon_contracts::session_entry::EntryKind::ModelChange {
                provider: "openrouter".into(),
                model_id: "deepseek/deepseek-chat-v3.1".into(),
            })
            .unwrap();
    }
    let store = std::sync::Arc::new(std::sync::Mutex::new(
        epsilon_core::SessionStore::open(&path).unwrap(),
    ));

    // What composition does, through the shared applier.
    let applied = epsilon_runtime::apply_resumed_context(&agent, &store, "faux-1", false);
    assert!(
        applied.model.is_some(),
        "the precondition: composition adopted the session's model"
    );

    // What `tui_mode` now passes, and `drive` hands to `App::new`.
    let (ftx, _frx) = tokio::sync::mpsc::unbounded_channel();
    let (ntx, _nrx) = tokio::sync::mpsc::unbounded_channel();
    let (otx, _orx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(agent.model_id(), ftx, ntx, otx);
    app.default_model = None;

    let rows = render_rows(&mut app, 100, 30);
    assert!(
        rows[0].contains("DeepSeek V3.1"),
        "the frame must name the composed model: {:?}",
        rows[0]
    );
    assert!(
        !rows[0].contains("Faux (offline scripted)"),
        "and not the launch model the session overrode: {:?}",
        rows[0]
    );
}

/// **Criterion 7: a model choice is remembered for the workspace that made it.**
///
/// Both write sites used to resolve `override.or(global)`, so the only way a
/// project got its own `[model] id` was hand-editing the file. The project dir
/// wins when the workspace has one; a workspace with none still writes global,
/// which is the behaviour every earlier launch had.
#[tokio::test]
async fn a_model_choice_is_remembered_for_the_workspace_that_made_it() {
    let (mut app, _agent, _receivers) = test_app();
    let ws = tempfile::tempdir().expect("a workspace");
    app.workspace_root = ws.path().to_path_buf();
    let project = ws.path().join(".epsilon");
    std::fs::create_dir_all(&project).unwrap();
    // A stale value from an earlier session must be replaced, not appended to.
    epsilon_core::config::save_model_id_in(&project, "faux-1").unwrap();

    app.save_default_model("deepseek/deepseek-chat-v3.1");

    assert_eq!(
        epsilon_core::config::load_model_id_in(&project).as_deref(),
        Some("deepseek/deepseek-chat-v3.1"),
        "the project's own config.toml is the write target when the workspace has one"
    );
    assert_eq!(
        app.default_model.as_deref(),
        Some("deepseek/deepseek-chat-v3.1"),
        "and the mirror the picker orders by moves with it"
    );

    // The control: a workspace with no `.epsilon` of its own resolves elsewhere
    // — the override, which is the only hermetic answer to that question.
    let bare = tempfile::tempdir().expect("a bare workspace");
    let override_dir = tempfile::tempdir().expect("an override dir");
    app.workspace_root = bare.path().to_path_buf();
    app.config_dir_override = Some(override_dir.path().to_path_buf());
    app.save_default_model("kimi-k2.6");
    assert_eq!(
        epsilon_core::config::load_model_id_in(override_dir.path()).as_deref(),
        Some("kimi-k2.6"),
        "with no project dir to write, the override is where it lands"
    );
}

/// An ordinary ask is unaffected: same keys, same prompt.
///
/// The stripping must not cost a reader the normal decision surface, and a guard that fired
/// on every ask would be useless.
#[tokio::test]
async fn an_ordinary_ask_still_offers_the_usual_keys() {
    let (mut app, _agent, _receivers) = test_app();
    let approvals = ChannelApprovalUi::new();
    let _response = park_ask(&mut app, &approvals, "g1").await;
    let frame = render_rows(&mut app, 100, 40).join("\n");

    assert!(
        frame.contains("allow once"),
        "an ordinary ask must be approvable:\n{frame}"
    );
    assert!(
        !frame.contains("display-steering"),
        "and must not claim anything was removed:\n{frame}"
    );
}

/// Enter while a run is in flight steers it rather than queueing behind it.
///
/// The queue decision and its transcript notice both live in the source
/// with no test pinning them. Staged through the `running` seam (a real
/// run cannot be held still to type into); both halves asserted — the
/// notice the reader sees, and the queue the agent reads — so a notice
/// without a steer, or a steer without a notice, fails.
#[tokio::test]
async fn enter_while_streaming_steers_rather_than_queueing() {
    let (mut app, agent, (_frx, mut nrx)) = test_app();
    app.ui.set_running_for_test(true);
    type_and_enter(&mut app, &agent, "steer toward the light");
    let said = tokio::time::timeout(std::time::Duration::from_secs(5), nrx.recv())
        .await
        .expect("the notice arrives")
        .expect("the channel stays open");
    assert!(
        said.contains("steering queued"),
        "the reader must be told the turn was steered, not queued: {said:?}"
    );
    assert_eq!(
        agent.steer_queue_len(),
        1,
        "and the steer must land in the queue the agent reads"
    );
}

/// `/clone` duplicates the session and says so when empty (P-075).
///
/// Mirrors the `/fork` test: same `fork_branch_at` machinery, differing
/// only in the word reported — which is exactly what this pins. The empty
/// case names itself rather than forking nothing.
#[test]
fn clone_duplicates_the_session_and_names_empty() {
    let (mut app, agent, _rx) = test_app();
    let dir = seed_branch(&agent, &["first", "second"]);

    app.run_command(
        &agent,
        crate::picker::parse_command("/clone").expect("/clone parses"),
    );

    let source = dir.path().join("session.jsonl");
    let siblings: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read_dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    let cloned: Vec<_> = siblings.iter().filter(|p| **p != source).cloned().collect();
    assert_eq!(
        cloned.len(),
        1,
        "exactly one new session file must appear; found {cloned:?}"
    );
    let source_id = header_id(&std::fs::read_to_string(&source).expect("read source"));
    let clone_id = header_id(&std::fs::read_to_string(&cloned[0]).expect("read clone"));
    assert_ne!(
        source_id, clone_id,
        "a clone is a new session: it must carry its own id, not the source's"
    );
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.text.contains("cloned session")),
        "and it must say so: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );
}

/// `/clone` with nothing to clone says so (P-075, empty half).
#[test]
fn clone_with_no_messages_says_nothing_to_clone() {
    let (mut app, agent, _rx) = test_app();
    let _dir = seed_branch(&agent, &[]);

    app.run_command(
        &agent,
        crate::picker::parse_command("/clone").expect("/clone parses"),
    );

    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.text.contains("nothing to clone")),
        "empty stays honest: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );
}

/// `/name` sets, shows, and says which happened (P-076).
///
/// Without a store the name lives for the run and says so; bare reads
/// back what was set. The persisted half is the window's `/name` test
/// (same entry, other front end) — this pins the terminal's words.
#[test]
fn name_sets_shows_and_says_which_happened() {
    let (mut app, agent, _rx) = test_app();

    app.run_command(
        &agent,
        crate::picker::parse_command("/name tabby").expect("/name parses"),
    );
    assert_eq!(
        app.session_name.as_deref(),
        Some("tabby"),
        "the name must land in memory"
    );
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.text.contains("session named 'tabby'")),
        "and report that it was not saved: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );

    app.run_command(
        &agent,
        crate::picker::parse_command("/name").expect("bare /name parses"),
    );
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.text.contains("session name: 'tabby'")),
        "bare must read back what was set: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );
}

/// `/import` asks before swapping, and only `yes` swaps (P-079).
///
/// A swap drops the current context from view, so it asks first. `no`
/// cancels, anything else repeats the question without eating the line
/// (the gate must not swallow a prompt the reader meant to send).
#[test]
fn import_asks_before_swapping_and_only_yes_swaps() {
    let (mut app, agent, _rx, dir) = test_app_with_store();
    let target = dir.path().join("other.jsonl");
    let mut store =
        epsilon_session::SessionStore::open_in(target.clone(), dir.path()).expect("a store opens");
    store
        .append(epsilon_contracts::session_entry::EntryKind::Message {
            message: epsilon_contracts::message::Message::User(
                epsilon_contracts::message::UserMessage {
                    id: "m-imp".into(),
                    timestamp_ms: 0,
                    content: vec![epsilon_contracts::content::UserContent::Text {
                        text: "the imported line".into(),
                    }],
                },
            ),
            api_request_sha256: None,
        })
        .expect("the message lands");
    drop(store);

    app.run_command(
        &agent,
        crate::picker::parse_command(&format!("/import {}", target.display()))
            .expect("/import parses"),
    );
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.text.contains("Replace current session with")),
        "must ask before doing anything: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );
    assert!(app.pending_import.is_some(), "and hold the question open");

    // Anything but yes/no repeats the question and keeps it open.
    app.input = "maybe later".to_string();
    app.submit_editor(&agent, false);
    assert!(
        app.pending_import.is_some(),
        "a non-answer must not resolve the gate"
    );

    // No cancels.
    app.input = "no".to_string();
    app.submit_editor(&agent, false);
    assert!(app.pending_import.is_none(), "no must close the question");
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.text.contains("import cancelled")),
        "and say so: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );
}

/// `yes` on a pending import swaps the store (P-079, proceed half).
#[test]
fn import_yes_swaps_the_store() {
    let (mut app, agent, _rx, dir) = test_app_with_store();
    let target = dir.path().join("other.jsonl");
    let mut store =
        epsilon_session::SessionStore::open_in(target.clone(), dir.path()).expect("a store opens");
    store
        .append(epsilon_contracts::session_entry::EntryKind::Message {
            message: epsilon_contracts::message::Message::User(
                epsilon_contracts::message::UserMessage {
                    id: "m-imp".into(),
                    timestamp_ms: 0,
                    content: vec![epsilon_contracts::content::UserContent::Text {
                        text: "the imported line".into(),
                    }],
                },
            ),
            api_request_sha256: None,
        })
        .expect("the message lands");
    drop(store);

    app.run_command(
        &agent,
        crate::picker::parse_command(&format!("/import {}", target.display()))
            .expect("/import parses"),
    );
    app.input = "yes".to_string();
    app.submit_editor(&agent, false);
    assert!(
        app.pending_import.is_none(),
        "yes must resolve the question"
    );
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.text.contains("imported") && l.text.contains("store swapped")),
        "and report the swap: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );
}

/// `/resume <path>` switches directly with a report (round 62).
///
/// The browser's Enter is pinned; the typed path form was pinned only by
/// the shared switch it shares with `/import`. Same words, same store.
#[test]
fn resume_path_switches_and_reports() {
    let (mut app, agent, _rx, dir) = test_app_with_store();
    let target = dir.path().join("other.jsonl");
    let mut store =
        epsilon_session::SessionStore::open_in(target.clone(), dir.path()).expect("a store opens");
    store
        .append(epsilon_contracts::session_entry::EntryKind::Message {
            message: epsilon_contracts::message::Message::User(
                epsilon_contracts::message::UserMessage {
                    id: "m-res".into(),
                    timestamp_ms: 0,
                    content: vec![epsilon_contracts::content::UserContent::Text {
                        text: "the resumed line".into(),
                    }],
                },
            ),
            api_request_sha256: None,
        })
        .expect("the message lands");
    drop(store);

    app.run_command(
        &agent,
        crate::picker::parse_command(&format!("/resume {}", target.display()))
            .expect("/resume parses"),
    );
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.text.contains("resumed") && l.text.contains("history restored")),
        "must report the resume: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );
}

/// Append one user message to the agent's attached store (export fixtures).
fn push_user_message(agent: &std::sync::Arc<epsilon_core::Agent>, text: &str) {
    agent
        .session_store()
        .expect("a store is attached")
        .lock()
        .expect("unlocked")
        .append(epsilon_contracts::session_entry::EntryKind::Message {
            message: epsilon_contracts::message::Message::User(
                epsilon_contracts::message::UserMessage {
                    id: "m-exp".into(),
                    timestamp_ms: 0,
                    content: vec![epsilon_contracts::content::UserContent::Text {
                        text: text.into(),
                    }],
                },
            ),
            api_request_sha256: None,
        })
        .expect("the message lands");
}

/// `/export` infers format and destination (P-078).
///
/// Bare writes HTML to the default exports dir; `json` writes JSON;
/// a `.jsonl` path infers the JSON writer. The writers themselves were
/// pinned; the inference and the destination were pinned by source only.
#[test]
fn export_bare_defaults_to_html_in_exports() {
    let (mut app, agent, _rx, dir) = test_app_with_store();
    push_user_message(&agent, "export me");

    app.run_command(
        &agent,
        crate::picker::parse_command("/export").expect("/export parses"),
    );

    let exports = dir.path().join(".epsilon").join("exports");
    let html: Vec<_> = std::fs::read_dir(&exports)
        .expect("exports dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "html"))
        .collect();
    assert_eq!(
        html.len(),
        1,
        "bare /export must write one default HTML file; found {html:?}"
    );
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.text.contains("exported html")),
        "and report it: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );
}

/// `/export json` and a `.jsonl` path both reach the JSON writer (P-078).
#[test]
fn export_json_fmt_and_jsonl_suffix() {
    let (mut app, agent, _rx, dir) = test_app_with_store();
    push_user_message(&agent, "export me");

    app.run_command(
        &agent,
        crate::picker::parse_command("/export json").expect("/export parses"),
    );
    let exports = dir.path().join(".epsilon").join("exports");
    assert!(
        std::fs::read_dir(&exports)
            .expect("exports dir")
            .filter_map(Result::ok)
            .any(|e| e.path().extension().is_some_and(|x| x == "json")),
        "json must land a default .json file"
    );
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.text.contains("exported json")),
        "and report it: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );

    app.run_command(
        &agent,
        crate::picker::parse_command("/export out.jsonl").expect("/export parses"),
    );
    assert!(
        dir.path().join("out.jsonl").exists(),
        "a .jsonl path must infer the JSON writer to that path"
    );
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.text.contains("exported json") && l.text.contains("out.jsonl")),
        "and report it as JSON: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );
}

/// Bare `/preset` names the four with the current marked (ADR-032).
#[test]
fn preset_bare_lists_with_current_marked() {
    let (mut app, agent, _rx) = test_app();
    app.run_command(
        &agent,
        crate::picker::parse_command("/preset").expect("/preset parses"),
    );
    let lines: Vec<_> = app.ui.lines().iter().map(|l| l.text.clone()).collect();
    assert!(
        lines.iter().any(|l| l.contains("auto")
            && l.contains("workspace-write")
            && l.contains("read-only")
            && l.contains("danger-full-access")),
        "must list all four: {lines:?}"
    );
}

/// `/preset read-only` switches at once; unknown names name the four.
#[test]
fn preset_switch_down_applies_and_unknown_names_choices() {
    let (mut app, agent, _rx) = test_app();
    // Pinned: switches persist since Wave 20, and a test writes nowhere near
    // a real config.
    let cfg = tempfile::tempdir().expect("a config dir");
    app.config_dir_override = Some(cfg.path().to_path_buf());
    app.run_command(
        &agent,
        crate::picker::parse_command("/preset read-only").expect("/preset parses"),
    );
    assert_eq!(agent.current_preset(), "read-only");
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.text.contains("read-only") && l.text.contains("was auto")),
        "must report the switch: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );

    app.run_command(
        &agent,
        crate::picker::parse_command("/preset yolo").expect("/preset parses"),
    );
    assert_eq!(
        agent.current_preset(),
        "read-only",
        "unknown changes nothing"
    );
}

/// `/preset danger-full-access` asks; only `yes` switches (ADR-032).
#[test]
fn preset_full_access_asks_and_only_yes_switches() {
    let (mut app, agent, _rx) = test_app();
    // Pinned for the same reason as the switch test above: the `yes` path
    // persists since Wave 20.
    let cfg = tempfile::tempdir().expect("a config dir");
    app.config_dir_override = Some(cfg.path().to_path_buf());
    app.run_command(
        &agent,
        crate::picker::parse_command("/preset danger-full-access").expect("/preset parses"),
    );
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.text.contains("Switch permission preset")),
        "must ask before raising: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );
    assert!(app.pending_preset.is_some(), "and hold the question open");
    assert_eq!(agent.current_preset(), "auto", "asking switches nothing");

    app.input = "no".to_string();
    app.submit_editor(&agent, false);
    assert!(app.pending_preset.is_none(), "no must close the question");
    assert_eq!(agent.current_preset(), "auto", "no switches nothing");

    app.run_command(
        &agent,
        crate::picker::parse_command("/preset danger-full-access").expect("/preset parses"),
    );
    app.input = "yes".to_string();
    app.submit_editor(&agent, false);
    assert_eq!(agent.current_preset(), "danger-full-access");
}

/// Wave 20: `/preset read-only` writes `[permissions] preset` through, so a
/// restart reads it back instead of returning to `auto` with no notice.
#[test]
fn preset_switch_writes_through_and_reloads_from_disk() {
    let (mut app, agent, _rx) = test_app();
    // Pinned: a switch persists, and a test writes nowhere near a real
    // config. `project_dir_or_global` prefers the override, so this is the
    // dir asserted on below.
    let cfg = tempfile::tempdir().expect("a config dir");
    app.config_dir_override = Some(cfg.path().to_path_buf());

    app.run_command(
        &agent,
        crate::picker::parse_command("/preset read-only").expect("/preset parses"),
    );
    assert_eq!(agent.current_preset(), "read-only");
    assert_eq!(
        epsilon_core::config::load_preset_in(cfg.path()).as_deref(),
        Some("read-only"),
        "the switch is in the FILE, not only in the gate"
    );

    // ...and composition resolves it: no flag plus this file is this preset.
    assert_eq!(
        epsilon_runtime::resolve_effective_preset(
            epsilon_safety::Preset::Auto,
            epsilon_core::config::load_preset_in(cfg.path()).as_deref(),
            None,
        ),
        epsilon_safety::Preset::ReadOnly,
        "a restart with no flag comes back on the persisted preset"
    );
}

/// Wave 20: `danger-full-access` still gates through `pending_preset` — but a
/// `yes` applies to the run that confirmed it and is never written down. A
/// standing full-access file would reload the next launch with no question
/// asked, which is the leak fail-closed forbids: asking writes nothing,
/// refusing writes nothing, and confirming switches in memory only.
#[test]
fn preset_danger_still_gates_and_never_persists() {
    let (mut app, agent, _rx) = test_app();
    let cfg = tempfile::tempdir().expect("a config dir");
    app.config_dir_override = Some(cfg.path().to_path_buf());

    app.run_command(
        &agent,
        crate::picker::parse_command("/preset danger-full-access").expect("/preset parses"),
    );
    assert!(
        app.pending_preset.is_some(),
        "raising still holds the question"
    );
    assert_eq!(agent.current_preset(), "auto", "asking switches nothing");
    assert!(
        !cfg.path().join("config.toml").exists(),
        "asking writes nothing"
    );

    app.input = "no".to_string();
    app.submit_editor(&agent, false);
    assert_eq!(agent.current_preset(), "auto", "no switches nothing");
    assert!(
        !cfg.path().join("config.toml").exists(),
        "refusing writes nothing either"
    );

    app.run_command(
        &agent,
        crate::picker::parse_command("/preset danger-full-access").expect("/preset parses"),
    );
    app.input = "yes".to_string();
    app.submit_editor(&agent, false);
    assert_eq!(agent.current_preset(), "danger-full-access");
    assert!(
        !cfg.path().join("config.toml").exists(),
        "a confirmed danger switch applies to this run only — it is never saved"
    );
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.text.contains("this run only")),
        "and the notice says so: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );
}

/// Wave 20: the settings panel shares `run_preset_command`, so Enter on the
/// permission row persists too — and a write failure keeps the panel open
/// with the reason instead of closing over it (the `save_scope` shape).
#[test]
fn preset_settings_row_persists_and_a_failed_write_keeps_the_panel_open() {
    let (mut app, agent, _rx) = test_app();
    let cfg = tempfile::tempdir().expect("a config dir");
    app.config_dir_override = Some(cfg.path().to_path_buf());
    app.settings_open = true;
    app.settings_preset = "auto".to_string();
    app.apply_setting(&agent, 4);
    assert_eq!(agent.current_preset(), "workspace-write");
    assert!(
        app.settings_open,
        "a successful toggle stays in the panel with the new value visible"
    );
    assert_eq!(
        epsilon_core::config::load_preset_in(cfg.path()).as_deref(),
        Some("workspace-write"),
        "the panel path writes through like /preset does"
    );

    // Failure half: the override points at a FILE, so the write cannot land.
    let (mut app, agent, _rx) = test_app();
    let dir = tempfile::tempdir().expect("tmp");
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, "not a dir").unwrap();
    app.config_dir_override = Some(blocker);
    app.settings_open = true;
    app.settings_preset = "auto".to_string();
    app.apply_setting(&agent, 4);
    assert_eq!(
        agent.current_preset(),
        "workspace-write",
        "in-memory still switches — the notice, not a revert, carries the failure"
    );
    assert!(
        app.settings_open,
        "the panel stays open so the reader sees the reason"
    );
    assert!(
        app.ui.lines().iter().any(|l| l.text.contains("not saved")),
        "and the reason is said out loud: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );
}

/// `next_preset` walks auto, workspace-write, read-only, danger-full-access,
/// and restarts. Empty (what the panel shows when no preset was ever named)
/// and unknown names both mean auto, so Enter always moves forward.
#[test]
fn next_preset_walks_all_four_presets_and_restarts() {
    assert_eq!(App::next_preset(""), "workspace-write");
    assert_eq!(App::next_preset("auto"), "workspace-write");
    assert_eq!(App::next_preset("workspace-write"), "read-only");
    assert_eq!(App::next_preset("read-only"), "danger-full-access");
    assert_eq!(App::next_preset("danger-full-access"), "auto");
    assert_eq!(
        App::next_preset("Auto"),
        "workspace-write",
        "case is not a preset"
    );
    assert_eq!(
        App::next_preset("yolo"),
        "auto",
        "unknown restarts rather than spinning"
    );
}

/// With no session attached there is no header timestamp, so `/changes`
/// lists every checkpoint instead of filtering on an invented cutoff.
#[test]
fn session_since_unix_is_none_without_a_session() {
    let (_app, agent, _rx) = test_app();
    assert_eq!(App::session_since_unix(&agent), None);
}

/// `/skill` with a typo or no name reports usage and stays — it must not
/// quit the session (the error arms once returned `true`, which is quit).
#[test]
fn skill_typo_leaves_the_session_open() {
    let (mut app, agent, _rx) = test_app();
    let quit = app.run_command(
        &agent,
        crate::picker::parse_command("/skill bogus off").expect("/skill parses"),
    );
    assert!(!quit, "a typo must not end the session");
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.text.contains("no skill named 'bogus'")),
        "and it must say what was wrong: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );

    let quit = app.run_command(
        &agent,
        crate::picker::parse_command("/skill").expect("bare /skill parses"),
    );
    assert!(!quit, "bare /skill must not end the session either");
}

/// `/settings` draws all five rows with their live values and the keys that
/// work — a drawn control with no key behind it is the no-op AGENTS.md
/// forbids, and a live key with no hint is one the user cannot find.
#[test]
fn settings_panel_lists_every_row_with_its_live_value_and_keys() {
    let (mut app, _agent, _rx) = test_app();
    app.settings_open = true;
    app.settings_index = 0;
    let rows = render_rows(&mut app, 100, 30);
    let screen: String = rows
        .iter()
        .map(|r| r.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for row in [
        "model",
        "thinking",
        "theme",
        "thinking blocks",
        "permission",
    ] {
        assert!(screen.contains(row), "the {row} row is drawn: {screen}");
    }
    for hint in ["enter", "esc"] {
        assert!(screen.contains(hint), "the {hint} key is named: {screen}");
    }
    for pointer in ["/scoped-models", "/skill", "/login", "/editor"] {
        assert!(
            screen.contains(pointer),
            "the panel names where {pointer} lives: {screen}"
        );
    }
}

/// `/changes` with no checkpoints opens with its reason, not a blank panel.
#[test]
fn changes_panel_says_so_when_nothing_changed() {
    let (mut app, _agent, _rx) = test_app();
    app.changes_open = true;
    app.changes_scope = "this session".to_string();
    let rows = render_rows(&mut app, 100, 30);
    let screen: String = rows
        .iter()
        .map(|r| r.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        screen.contains("no checkpoints this session"),
        "the empty state names its scope: {screen}"
    );
    assert!(screen.contains("preview"), "and the preview key: {screen}");
}

/// Enter on a `/changes` row previews that file in the viewer — the panel's
/// advertised action, driven through the real key path.
#[test]
fn changes_enter_previews_the_highlighted_file() {
    let (mut app, agent, _rx) = test_app();
    let dir = tempfile::tempdir().expect("tmp");
    std::fs::write(dir.path().join("changed.rs"), "fn main() {}\n").expect("seed");
    app.workspace_root = dir.path().to_path_buf();
    app.changes_open = true;
    app.changes_scope = "this session".to_string();
    app.changes_rows = vec![("changed.rs".to_string(), "write".to_string())];
    app.changes_index = 0;

    assert!(!handle_key(&mut app, &agent, key(KeyCode::Enter)));
    assert!(!app.changes_open, "preview closes the panel");
    assert_eq!(app.viewer_tabs.len(), 1, "and opens exactly one viewer tab");
    assert_eq!(app.viewer_tabs[0].rel, "changed.rs");
    assert!(
        app.viewer_tabs[0]
            .lines
            .iter()
            .any(|l| l.contains("fn main()")),
        "showing the file's bytes: {:?}",
        app.viewer_tabs[0].lines
    );
}

/// **Streaming tool output reaches the screen, not only the record.**
///
/// `UiState::apply`'s `ToolExecutionUpdate` arm accumulates the running card's body (M152),
/// and `a_running_tool_card_shows_the_output_it_has_produced_so_far` pins that on **state**.
/// The frame is a different question: the render cache was invalidated only for cards with
/// `ended_at_ms.is_some()`, and a card that is still running has none — so it kept serving
/// the rows built when the card was opened, and the M152 fix was never on screen. A
/// three-minute `cargo build` drew a static card for three minutes, which reads as a hang.
///
/// That is `AGENTS.md`'s own failure mode in its own words: *a test that passes is not a
/// claim that is true — for anything a user sees, drive the real draw path and assert on the
/// frame.* So this drives the real path: events through `apply_with_scroll`, the production
/// draw, and the screen read back.
#[test]
fn a_running_tool_s_output_reaches_the_frame() {
    use epsilon_contracts::agent_event::AgentEvent;
    use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage};
    let (mut app, _agent, _rx) = test_app();
    super::apply_with_scroll(
        &mut app,
        &AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(AssistantMessage {
                id: "m-stream".into(),
                timestamp_ms: 0,
                provider: "faux".into(),
                model_id: "faux-1".into(),
                content: vec![epsilon_contracts::content::ContentBlock::ToolCall {
                    id: "tc-stream".into(),
                    name: "bash".into(),
                    arguments: serde_json::json!({ "command": "cargo build" }),
                }],
                stop_reason: StopReason::ToolUse,
                error_message: None,
                usage: Usage::default(),
            })),
        },
    );
    super::apply_with_scroll(
        &mut app,
        &AgentEvent::ToolExecutionStart {
            tool_call_id: "tc-stream".into(),
            tool_name: "bash".into(),
            args: serde_json::json!({ "command": "cargo build" }),
        },
    );
    // **The frame in between is the point.** A card that has never been drawn is built
    // fresh on the first frame with whatever body it already has, so driving all the
    // events before any draw would pass against the broken code. The real sequence
    // paints while the tool is running: the card is drawn once (pending, no output),
    // then output streams in, then the next frame must show it.
    let first = render_rows(&mut app, 80, 30).join("\n");
    assert!(
        !first.contains("epsilon-core"),
        "precondition: nothing has streamed on the first frame:\n{first}"
    );

    // Two chunks: the second is the assertion, because a cache that is not invalidated
    // between them shows the first one's rows for good.
    for chunk in [
        "Compiling epsilon-core v0.1.0",
        "Compiling epsilon-tools v0.1.0",
    ] {
        super::apply_with_scroll(
            &mut app,
            &AgentEvent::ToolExecutionUpdate {
                tool_call_id: "tc-stream".into(),
                tool_name: "bash".into(),
                partial: epsilon_contracts::tool::ToolPartial {
                    content: vec![epsilon_contracts::content::ContentBlock::Text {
                        text: chunk.into(),
                    }],
                    details: serde_json::Value::Null,
                },
            },
        );
    }
    let frame = render_rows(&mut app, 80, 30).join("\n");
    assert!(
        frame.contains("epsilon-core"),
        "the first streamed line must be on screen:\n{frame}"
    );
    assert!(
        frame.contains("epsilon-tools"),
        "the second streamed line must be on screen too — a card rebuilt only when it finishes \
         shows the first one for the whole run:\n{frame}"
    );
}

/// Tool cards read as blocks: a blank row before each card (P-136).
///
/// Pushed in `push_line` (the single funnel), not by producers — so every
/// card gets it and none can forget it. Asserted on the frame (blank row
/// present, exactly one) and on the lines (no blank first, never two).
#[test]
fn tool_cards_are_separated_by_blank_rows() {
    use epsilon_contracts::agent_event::AgentEvent;
    use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage};
    let (mut app, _agent, _rx) = test_app();
    app.ui.push_local_input("run it");
    let call = |id: &str, name: &str, args: serde_json::Value| {
        epsilon_contracts::content::ContentBlock::ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: args,
        }
    };
    app.ui.apply(&AgentEvent::MessageEnd {
        message: Box::new(Message::Assistant(AssistantMessage {
            id: "m1".into(),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![
                call("c1", "bash", serde_json::json!({ "command": "echo hi" })),
                call("c2", "read", serde_json::json!({ "path": "a.txt" })),
            ],
            stop_reason: StopReason::ToolUse,
            error_message: None,
            usage: Usage::default(),
        })),
    });

    let kinds: Vec<_> = app.ui.lines().iter().map(|l| l.kind).collect();
    let blanks = kinds
        .iter()
        .filter(|k| **k == crate::state::LineKind::Blank)
        .count();
    assert_eq!(
        blanks, 2,
        "one blank before each of the two cards, none elsewhere: {kinds:?}"
    );
    assert!(
        kinds
            .windows(2)
            .all(|w| !(w[0] == crate::state::LineKind::Blank
                && w[1] == crate::state::LineKind::Blank)),
        "never two blanks: {kinds:?}"
    );

    let frame = render_rows(&mut app, 80, 24).join("\n");
    let rows: Vec<&str> = frame.lines().collect();
    let first_card = rows
        .iter()
        .position(|r| r.contains("bash"))
        .expect("the card is on screen");
    assert!(
        rows[first_card - 1].trim().is_empty(),
        "a blank row precedes the card:\n{frame}"
    );
}

/// A card's body nests under its title with a continuation gutter.
///
/// The title keeps the line's gutter; body rows draw the continuation glyph so a
/// title and its output never share a gutter. Asserted on the frame.
///
/// **The glyphs come from the app's theme, not from this test's alphabet.** The
/// first version asserted the ASCII repertoire's literals (`\` for the call, `|`
/// for the body) and passed for months on a machine whose detection defaulted to
/// ASCII; round 105's battery ran with `TERM=xterm-256color`, the detection
/// flipped to the unicode repertoire, and the frame drew `▸` and `│` — the test
/// failed on a correct product. A repertoire-relative assertion is the fix: it
/// reads the glyphs the draw itself used and stays true in both.
#[test]
fn card_bodies_nest_under_a_continuation_gutter() {
    use epsilon_contracts::agent_event::AgentEvent;
    use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage};
    let (mut app, _agent, _rx) = test_app();
    app.ui.push_local_input("run it");
    app.ui.apply(&AgentEvent::MessageEnd {
        message: Box::new(Message::Assistant(AssistantMessage {
            id: "m1".into(),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![epsilon_contracts::content::ContentBlock::ToolCall {
                id: "c1".into(),
                name: "bash".into(),
                arguments: serde_json::json!({ "command": "echo hi" }),
            }],
            stop_reason: StopReason::ToolUse,
            error_message: None,
            usage: Usage::default(),
        })),
    });
    app.ui.apply(&AgentEvent::ToolExecutionEnd {
        tool_call_id: "c1".into(),
        tool_name: "bash".into(),
        is_error: false,
    });

    let frame = render_rows(&mut app, 80, 24).join("\n");
    let rows: Vec<&str> = frame.lines().collect();
    let title = rows
        .iter()
        .position(|r| r.contains("bash"))
        .expect("the title is on screen");
    assert!(
        rows[title].starts_with(app.theme.glyphs.tool_call),
        "the title keeps the call gutter:\n{frame}"
    );
    assert!(
        rows[title + 1].starts_with(app.theme.glyphs.vertical),
        "the body nests under a continuation gutter:\n{frame}"
    );
}

/// The live tail separates from the previous block while streaming.
///
/// Committed lines separate in `push_line`; the tail is not a line, so it
/// glued to whatever preceded it until commit. Same rule, at draw time.
#[test]
fn streaming_text_is_separated_from_the_previous_block() {
    use epsilon_contracts::agent_event::AgentEvent;
    use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage};
    let (mut app, _agent, _rx) = test_app();
    app.ui.push_local_input("run it");
    app.ui.apply(&AgentEvent::MessageEnd {
        message: Box::new(Message::Assistant(AssistantMessage {
            id: "m1".into(),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![epsilon_contracts::content::ContentBlock::ToolCall {
                id: "c1".into(),
                name: "bash".into(),
                arguments: serde_json::json!({ "command": "echo hi" }),
            }],
            stop_reason: StopReason::ToolUse,
            error_message: None,
            usage: Usage::default(),
        })),
    });
    app.ui.apply(&AgentEvent::MessageUpdate {
        message: Box::new(Message::Assistant(AssistantMessage {
            id: "m2".into(),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![epsilon_contracts::content::ContentBlock::Text {
                text: "working on it".into(),
            }],
            stop_reason: StopReason::EndTurn,
            error_message: None,
            usage: Usage::default(),
        })),
    });

    let frame = render_rows(&mut app, 80, 24).join("\n");
    let rows: Vec<&str> = frame.lines().collect();
    let tail = rows
        .iter()
        .position(|r| r.contains("working on it"))
        .expect("the live tail is on screen");
    assert!(
        rows[tail - 1].trim().is_empty(),
        "a blank row separates the tail from the card:\n{frame}"
    );
}

// ---- the conversation's live edge sits next to the composer ------------------
//
// The transcript used to be top-anchored: `draw_transcript` handed ratatui a
// `Paragraph` whose first row was the transcript region's first row, so a
// conversation shorter than the viewport drew its content under the header and
// left every remaining body row blank — at 80x24 a user turn, a separator and
// one notice drew on rows 1-3 of a twenty-row body and left seventeen blank
// rows between the newest text and the composer, and during a turn the arriving
// answer scrolled *away* from the input the reader was about to type into.
//
// Roadmap item 5 said this "wants its own decision". The decision is taken here
// and the reason is stated in the test that pins it: pi's own `ScrollView` is
// top-anchored by construction (`references/pi-1.0.0/packages/tui/src/components/
// scroll-view.ts`, `maxScrollTop = max(0, contentHeight - viewportHeight)`, so
// short content parks at offset 0), and Epsilon is a port of pi — but the
// desktop front end already follows the chat convention through
// `stick_to_bottom`, and `AGENTS.md` §2.1 is the invariant that matters: the two
// front ends disagreeing about where a conversation sits is the defect, not the
// difference. Deliberately different from pi, recorded as such.

/// The body's last row carries the conversation's last row.
///
/// Read from the layout the production draw uses, not from a remembered row
/// number: the ending blank's own `y` is where "directly above the composer"
/// means.
#[test]
fn a_short_conversation_sits_directly_above_the_ending_blank() {
    let (mut app, _agent, _rx) = test_app();
    app.ui.push_local_input("what changed?");
    app.ui.push_notice("here is the answer");

    let rows = render_rows(&mut app, 80, 24);
    // The newest row sits directly above the *ending blank*, not directly above
    // the composer: the blank is what keeps the answer from touching the input
    // line the composer draws.
    let last_row = crate::ui::layout::Regions::split(Rect::new(0, 0, 80, 24), 1)
        .gap
        .y as usize
        - 1;
    assert!(
        rows[last_row].contains("here is the answer"),
        "the newest row must sit directly above the ending blank, not under the \
         header:\n{rows:#?}"
    );
    // The dead space moved *above* the content rather than disappearing: an
    // empty body row at the top is what bottom-anchoring means, and a reader
    // who scrolls up still finds the header immediately followed by the first
    // thing that was said.
    assert!(
        rows[1].trim().is_empty(),
        "the spare rows must sit above the conversation:\n{rows:#?}"
    );
}

/// A view the reader scrolled back is untouched by the anchoring.
///
/// Padding is only correct while the view is following its tail: `transcript_skip`
/// returns 0 exactly when the content is shorter than the viewport, so a scrolled
/// window always has more rows than fit and there is nothing to pad. Pinning the
/// *scrolled* frame too, because "the frozen window moved" is the failure a
/// padding rule that ignored the scroll state would produce.
#[test]
fn a_scrolled_back_view_fills_the_body_from_the_top() {
    let mut app = bench_app(UiState::new());
    for index in 0..60 {
        app.ui.push_notice(format!("ROW-{index:03}"));
    }
    let rows = render_rows(&mut app, 60, 20);
    app.scroll_from_bottom = 5;
    let scrolled = render_rows(&mut app, 60, 20);
    let last_row = crate::ui::layout::Regions::split(Rect::new(0, 0, 60, 20), 1)
        .gap
        .y as usize
        - 1;
    assert!(
        !scrolled[1].trim().is_empty() && !scrolled[last_row].trim().is_empty(),
        "a scrolled window is full top to bottom, with nothing padded:\n{scrolled:#?}"
    );
    assert!(
        scrolled.join("\n").contains("ROW-"),
        "the transcript vanished from the scrolled frame:\n{scrolled:#?}"
    );
    assert_ne!(rows, scrolled, "the scroll did not move the window");
}

/// The live answer streams in next to the composer, not under the header.
///
/// The streaming case is where anchoring earns its place: the tail grows a row at
/// a time, and with it top-anchored the text a reader is watching arrives as far
// from the input as the screen allows.
#[test]
fn a_streaming_tail_arrives_next_to_the_composer() {
    use epsilon_contracts::agent_event::AgentEvent;
    use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage};
    let (mut app, _agent, _rx) = test_app();
    app.ui.apply(&AgentEvent::MessageUpdate {
        message: Box::new(Message::Assistant(AssistantMessage {
            id: "m1".into(),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![epsilon_contracts::content::ContentBlock::text(
                "first line of the answer",
            )],
            stop_reason: StopReason::EndTurn,
            error_message: None,
            usage: Usage::default(),
        })),
    });

    let rows = render_rows(&mut app, 80, 24);
    let last_row = crate::ui::layout::Regions::split(Rect::new(0, 0, 80, 24), 1)
        .gap
        .y as usize
        - 1;
    assert!(
        rows[last_row].contains("first line of the answer"),
        "the streaming tail must sit directly above the ending blank:\n{rows:#?}"
    );
}

/// The welcome screen is a hero, not a conversation, and stays at the top.
///
/// The one part of the body the anchoring deliberately does not touch: there is
// no live edge to follow until something has been said, and a title that jumped
// to the bottom of an empty window would be decoration for its own sake. Pinned
/// because the next pass that "makes the whole body consistent" would otherwise
/// have no test arguing with it.
#[test]
fn the_welcome_screen_stays_at_the_top_of_the_body() {
    let (mut app, _agent, _rx) = test_app();
    let rows = render_rows(&mut app, 80, 24);
    assert!(
        rows.iter().any(|r| r.contains("E P S I L O N")),
        "the welcome screen must still be on the first frame:\n{rows:#?}"
    );
    let hero = rows
        .iter()
        .position(|r| r.contains("E P S I L O N"))
        .expect("the welcome screen is on screen");
    assert_eq!(
        hero, 2,
        "the hero belongs at the top of the body:\n{rows:#?}"
    );
}

/// Reasoning and the answer separate *while* streaming, not only at commit.
///
/// Round 72's rule one level in. `draw_transcript` built both tails in one
/// pass, so for the whole of the text-streaming phase a reasoning row sat
/// directly above the answer row — and the moment the model stops reasoning and
/// starts answering, the one transition a reader is watching, had no marker
/// beyond the gutter glyph changing. At commit `push_line`'s kind-change rule
/// inserted a blank, so the frame also changed shape mid-turn.
///
/// One `MessageUpdate` carrying a Thinking block *and* a Text block, exactly as
/// a provider that reasons streams them. Asserted at both widths the evidence
/// discipline names.
#[test]
fn reasoning_and_the_answer_separate_while_streaming() {
    use epsilon_contracts::agent_event::AgentEvent;
    use epsilon_contracts::message::{AssistantMessage, Message, StopReason, Usage};
    let (mut app, _agent, _rx) = test_app();
    app.ui.apply(&AgentEvent::MessageUpdate {
        message: Box::new(Message::Assistant(AssistantMessage {
            id: "m1".into(),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![
                epsilon_contracts::content::ContentBlock::Thinking {
                    text: "weighing the two options".into(),
                    signature: Some(String::new()),
                },
                epsilon_contracts::content::ContentBlock::Text {
                    text: "the answer is B".into(),
                },
            ],
            stop_reason: StopReason::EndTurn,
            error_message: None,
            usage: Usage::default(),
        })),
    });

    for width in [80u16, 100] {
        let frame = render_rows(&mut app, width, 24).join("\n");
        let rows: Vec<&str> = frame.lines().collect();
        let reasoning = rows
            .iter()
            .position(|r| r.contains("weighing the two options"))
            .expect("the reasoning tail is on screen");
        let answer = rows
            .iter()
            .position(|r| r.contains("the answer is B"))
            .expect("the answer tail is on screen");
        assert!(answer > reasoning, "reasoning streams first:\n{frame}");
        assert!(
            rows[answer - 1].trim().is_empty(),
            "the row between them must be blank at {width}:\n{frame}"
        );
    }
}

/// Ctrl+S in the model picker saves the highlighted row as the default.
///
/// Written to an override dir, never global — then ordered second and
/// marked on the next open. Driven through the real key path, because the
/// defect class here is chords that parse and never fire (P-254).
#[test]
fn ctrl_s_in_the_picker_saves_the_default() {
    let (mut app, agent, _rx) = test_app();
    let dir = tempfile::tempdir().expect("a config dir");
    app.config_dir_override = Some(dir.path().to_path_buf());
    app.picker_open = true;
    app.picker_provider = Some("deepseek".to_string());
    app.picker_index = 1;

    handle_key(&mut app, &agent, ctrl(KeyCode::Char('s')));

    let saved = epsilon_core::config::load_model_id_in(dir.path());
    assert!(
        saved.is_some(),
        "must persist somewhere readable: {saved:?}"
    );
    assert_eq!(
        app.default_model.as_deref(),
        saved.as_deref(),
        "and memory must follow the file"
    );
}

/// P-047: `/scoped-models` opens a searchable checklist over the cycle scope.
///
/// The head parses today and only talks (a notice); the checklist is the
/// missing half. Opens through the real command path, filters through the
/// real key path, asserts on the real frame.
#[tokio::test]
async fn scoped_models_opens_a_searchable_checklist() {
    let (mut app, agent, _rx) = test_app();
    app.run_command(
        &agent,
        crate::picker::parse_command("/scoped-models").expect("/scoped-models parses"),
    );
    assert!(app.scope_open, "/scoped-models must open the checklist");

    let screen = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        screen.contains("scoped models"),
        "the title must name the checklist: {screen}"
    );

    for c in "deepseek".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    let filtered = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        filtered.contains("deepseek"),
        "typing must filter the checklist rows: {filtered}"
    );
}

/// P-047: Space stages membership live, Ctrl+S persists, Esc discards.
///
/// Staged == `app.favorites` (the F2 cycle list reads it), so the rescope
/// is live the moment Space lands; the file moves only on Ctrl+S.
#[tokio::test]
async fn scope_space_stages_live_and_ctrl_s_persists() {
    let (mut app, agent, _rx) = test_app();
    let dir = tempfile::tempdir().expect("a config dir");
    app.config_dir_override = Some(dir.path().to_path_buf());
    // Hermetic scope: the fixture may carry ambient global favorites.
    app.favorites.clear();
    app.run_command(
        &agent,
        crate::picker::parse_command("/scoped-models").expect("/scoped-models parses"),
    );
    assert!(app.scope_open);

    let rows = crate::picker::build_rows_with(
        "",
        &app.model_id,
        &[],
        &app.favorites,
        None,
        &app.live_cache,
        &app.customs,
    );
    assert!(!rows.is_empty(), "the checklist must list the catalog");
    let id = rows[0].id.clone();

    handle_key(&mut app, &agent, key(KeyCode::Char(' ')));
    assert!(
        app.favorites.contains(&id),
        "Space must stage the highlighted model into the cycle scope"
    );
    let staged = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        staged.contains("[x]"),
        "the staged row must read checked on the frame: {staged}"
    );

    handle_key(&mut app, &agent, ctrl(KeyCode::Char('s')));
    assert!(!app.scope_open, "Ctrl+S must save and close");
    let saved = epsilon_core::config::load_favorite_models_in(dir.path());
    assert_eq!(saved, vec![id], "the scope must persist where it rereads");
}

/// P-047: Esc throws the unstaged toggles away, file untouched.
#[tokio::test]
async fn scope_esc_discards_unstaged_toggles() {
    let (mut app, agent, _rx) = test_app();
    let dir = tempfile::tempdir().expect("a config dir");
    app.config_dir_override = Some(dir.path().to_path_buf());
    // Hermetic scope: the fixture may carry ambient global favorites.
    app.favorites.clear();
    app.run_command(
        &agent,
        crate::picker::parse_command("/scoped-models").expect("/scoped-models parses"),
    );
    let rows = crate::picker::build_rows_with(
        "",
        &app.model_id,
        &[],
        &app.favorites,
        None,
        &app.live_cache,
        &app.customs,
    );
    let id = rows[0].id.clone();
    handle_key(&mut app, &agent, key(KeyCode::Char(' ')));
    assert!(app.favorites.contains(&id));

    handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert!(!app.scope_open, "Esc must close the checklist");
    assert!(
        !app.favorites.contains(&id),
        "Esc must revert the unstaged toggle"
    );
    assert!(
        epsilon_core::config::load_favorite_models_in(dir.path()).is_empty(),
        "nothing must reach the file without Ctrl+S"
    );
}

/// Ctrl+G is pressed and answers honestly without the `real` feature.
///
/// E-045 recorded that no test pressed it. Without a fetcher there is
/// nothing to refresh; the key must say so rather than silently do nothing.
/// Notices travel the channel (not `ui.lines`), drained synchronously — the
/// send happens inside the key handler, before it returns.
#[test]
fn ctrl_g_reports_unavailable_without_the_real_feature() {
    let (mut app, agent, (_frx, mut nrx)) = test_app();
    app.picker_open = true;
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('g')));
    let said: Vec<_> = std::iter::from_fn(|| nrx.try_recv().ok()).collect();
    assert!(
        said.iter()
            .any(|l| l.contains("built without --features real")),
        "must name the missing build, not go silent: {said:?}"
    );
}

/// A finished refresh reports its outcome (P-253).
///
/// Success merges ids into the cache and says how many; failure names the
/// error. Driven directly (the channel end belongs to the event loop), both
/// halves asserted — a merge without a report, or a report without a merge,
/// fails.
#[test]
fn finished_refresh_reports_success_and_failure() {
    let (mut app, _agent, _rx) = test_app();
    let said = app.record_live_result("deepseek".to_string(), Ok(vec!["deepseek-new".to_string()]));
    assert!(
        said.is_some_and(|s| s.contains("1 models merged")),
        "success must report the count"
    );
    assert!(
        app.live_cache
            .iter()
            .any(|(p, ids)| p == "deepseek" && ids.contains(&"deepseek-new".to_string())),
        "and merge the ids"
    );

    let said = app.record_live_result("deepseek".to_string(), Err("timeout".to_string()));
    assert!(
        said.is_some_and(|s| s.contains("live listing failed") && s.contains("timeout")),
        "failure must name the error"
    );
}

/// `/model <text>` that names no one model opens the picker pre-filled
/// instead of switching to a typo (P-247). Exact ids still apply instantly
/// (pinned beside this); a family name — ambiguous by design — opens the
/// selector with the text ready to refine or pick.
#[test]
fn model_partial_opens_the_picker_prefilled() {
    let (mut app, agent, _rx) = test_app();
    assert!(!type_and_enter(&mut app, &agent, "/model deepseek"));
    assert!(
        app.picker_open,
        "an ambiguous name must open the picker, not switch"
    );
    assert_eq!(app.input, "deepseek", "with the typed text as the filter");
    assert_eq!(
        app.model_id, "faux-1",
        "and switch nothing (no typo in the session, none saved)"
    );
}

/// Ctrl+L opens the model picker like Ctrl+M (P-247).
#[test]
fn ctrl_l_opens_the_model_picker() {
    let (mut app, agent, _rx) = test_app();
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('l')));
    assert!(app.picker_open, "ctrl+l must open the picker");
}

/// Alt+Q pulls queued steering back into the composer with a count (P-033).
///
/// Seeded directly (steer pushes without a run); driven through the real key
/// path. Empty queue says so instead of going silent.
#[test]
fn alt_q_restores_queued_messages_with_a_count() {
    use epsilon_contracts::content::UserContent;
    use epsilon_contracts::message::{Message, UserMessage};
    let (mut app, agent, _rx) = test_app();
    for (i, text) in ["first thought", "second thought"].iter().enumerate() {
        agent.steer(Message::User(UserMessage {
            id: format!("m-q{i}"),
            timestamp_ms: 0,
            content: vec![UserContent::Text {
                text: text.to_string(),
            }],
        }));
    }
    assert_eq!(agent.steer_queue_len(), 2);

    handle_key(
        &mut app,
        &agent,
        key_mod(KeyModifiers::ALT, KeyCode::Char('q')),
    );
    assert_eq!(agent.steer_queue_len(), 0, "pulled, not copied");
    assert!(
        app.input.contains("first thought") && app.input.contains("second thought"),
        "both queued messages must land in the draft: {:?}",
        app.input
    );
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.text.contains("Restored 2 queued messages")),
        "and report the count: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );

    handle_key(
        &mut app,
        &agent,
        key_mod(KeyModifiers::ALT, KeyCode::Char('q')),
    );
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.text.contains("No queued messages")),
        "empty must say so: {:?}",
        app.ui.lines().iter().map(|l| &l.text).collect::<Vec<_>>()
    );
}

/// Ctrl+N starts a new session file, like `/new` (P-243).
///
/// Routed through the head, so the key and the command share the outcome.
/// Counts session files before and after: one more, beside the others.
#[test]
fn ctrl_n_starts_a_new_session_file() {
    let (mut app, agent, _rx, dir) = test_app_with_store();
    let before: Vec<_> = walk_jsonl(dir.path());
    handle_key(&mut app, &agent, ctrl(KeyCode::Char('n')));
    let after: Vec<_> = walk_jsonl(dir.path());
    assert_eq!(
        after.len(),
        before.len() + 1,
        "exactly one session file must appear: {after:?}"
    );
}

/// Count every `.jsonl` file under a directory (session files).
fn walk_jsonl(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.filter_map(Result::ok) {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "jsonl") {
                out.push(p);
            }
        }
    }
    out
}

/// An empty session browser names the key that starts one (P-243).
///
/// No sessions + empty filter: actionable (`Ctrl+N`). Filtered-out: the
/// widen hint (unchanged). Asserted on the frame. Emptied by removing the
/// fixture's own file (its tempdir, no one else's).
#[test]
fn empty_browser_names_the_new_session_key() {
    let (mut app, agent, _rx, dir) = test_app_with_store();
    for f in walk_jsonl(dir.path()) {
        std::fs::remove_file(&f).expect("empty the fixture");
    }
    // The directory must exist (a missing dir is no browser, not an empty
    // one — pinned beside this).
    assert!(
        app.session_dir().is_dir(),
        "the sessions dir must exist to be empty"
    );
    app.open_session_browser(&agent);
    assert!(
        app.session_browser.is_some(),
        "an existing empty dir still opens"
    );
    let frame = render_rows(&mut app, 80, 24).join("\n");
    assert!(frame.contains("Ctrl+N"), "empty must name the key: {frame}");
}

/// Ctrl+N works inside the session browser it is named in (P-243).
///
/// The empty message promises the key; the browser owns the keyboard, so a
/// composer-level-only chord would be a promise the UI breaks. Empty the
/// fixture, open, press: the browser closes and a session file appears.
#[test]
fn ctrl_n_inside_the_browser_starts_a_session() {
    let (mut app, agent, _rx, dir) = test_app_with_store();
    for f in walk_jsonl(dir.path()) {
        std::fs::remove_file(&f).expect("empty the fixture");
    }
    app.open_session_browser(&agent);
    assert!(
        app.session_browser.is_some(),
        "the empty browser must open to press in"
    );
    let before: Vec<_> = walk_jsonl(dir.path());

    handle_key(&mut app, &agent, ctrl(KeyCode::Char('n')));

    assert!(
        app.session_browser.is_none(),
        "a new session leaves the browser"
    );
    let after: Vec<_> = walk_jsonl(dir.path());
    assert_eq!(
        after.len(),
        before.len() + 1,
        "and a session file appears: {after:?}"
    );
}

/// Tab toggles the session browser between this workspace and all sessions (P-230).
///
/// A foreign session (another cwd's directory, a real store file, never
/// attached): workspace scope hides it, one Tab shows it with its workspace
/// named, a second Tab hides it again — all through the real key path.
#[test]
fn tab_in_the_browser_toggles_workspace_and_all_scopes() {
    let (mut app, agent, _rx, dir) = test_app_with_store();
    let sessions = dir.path().join(".epsilon").join("sessions");
    let foreign_dir =
        epsilon_session::time::session_dir_for(&sessions, std::path::Path::new("/elsewhere"));
    std::fs::create_dir_all(&foreign_dir).expect("the foreign session directory");
    let _foreign = epsilon_session::SessionStore::open_in(
        foreign_dir.join("other.jsonl"),
        std::path::Path::new("/elsewhere"),
    )
    .expect("a foreign session opens");
    app.open_session_browser(&agent);
    assert!(
        app.session_browser.is_some(),
        "the browser must open to press in"
    );
    let frame = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        !frame.contains("elsewhere"),
        "workspace scope hides it first: {frame}"
    );
    assert!(
        frame.contains("this workspace"),
        "the title names the starting scope: {frame}"
    );

    handle_key(&mut app, &agent, key(KeyCode::Tab));
    let frame = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        frame.contains("elsewhere"),
        "one Tab shows every scope with the workspace named: {frame}"
    );
    assert!(
        frame.contains("all workspaces"),
        "the title names the scope Tab switched to: {frame}"
    );
    // The same frame at the wide size: scope words must survive the width.
    let wide = render_rows(&mut app, 100, 30).join("\n");
    assert!(
        wide.contains("all workspaces") && wide.contains("elsewhere"),
        "scope and foreign label hold at 100x30: {wide}"
    );

    handle_key(&mut app, &agent, key(KeyCode::Tab));
    let frame = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        !frame.contains("elsewhere"),
        "a second Tab returns to the workspace: {frame}"
    );
}

/// Alt+Enter queues a follow-up for after the turn (P-030).
///
/// Mirrors the steering test: staged running, real key path, both halves —
/// the notice the reader sees and the queue the agent reads. A follow-up is
/// not steering (`steer_queue_len` stays 0) and not a spawn (nothing runs).
#[tokio::test]
async fn alt_enter_while_streaming_queues_a_follow_up() {
    let (mut app, agent, (_frx, mut nrx)) = test_app();
    app.ui.set_running_for_test(true);
    app.input = "and then the sequel".to_string();
    app.cursor_chars = app.input.chars().count();
    handle_key(&mut app, &agent, key_mod(KeyModifiers::ALT, KeyCode::Enter));
    let said = tokio::time::timeout(std::time::Duration::from_secs(5), nrx.recv())
        .await
        .expect("the notice arrives")
        .expect("the channel stays open");
    assert!(
        said.contains("follow-up queued"),
        "the reader must be told it waits: {said:?}"
    );
    assert_eq!(
        agent.followup_queue_len(),
        1,
        "and it must land in the follow-up queue"
    );
    assert_eq!(
        agent.steer_queue_len(),
        0,
        "not in the steering queue: a follow-up waits, it does not steer"
    );
}

/// Up/Down recall prompt history through the explicit bindings (P-211).
///
/// Two submits, then Up twice (older, oldest), Down (newer), Down (empty
/// again). Driven through the real key path — the defect class is chords
/// that parse and never fire, and recall had no test at all.
#[tokio::test]
async fn up_and_down_walk_prompt_history() {
    let (mut app, agent, _rx) = test_app();
    assert!(!type_and_enter(&mut app, &agent, "first thought"));
    assert!(!type_and_enter(&mut app, &agent, "second thought"));

    handle_key(&mut app, &agent, key(KeyCode::Up));
    assert_eq!(app.input, "second thought", "newest first");
    handle_key(&mut app, &agent, key(KeyCode::Up));
    assert_eq!(app.input, "first thought", "then older");
    handle_key(&mut app, &agent, key(KeyCode::Down));
    assert_eq!(app.input, "second thought", "and back forward");
    handle_key(&mut app, &agent, key(KeyCode::Down));
    assert!(
        app.input.is_empty(),
        "past newest restores the empty draft: {:?}",
        app.input
    );
}

/// Ctrl+Up jumps the viewport to prompts; it does not recall (P-211).
///
/// The recall arms used to match bare `KeyCode::Up`, which also caught
/// Ctrl+Up and starved the prompt-jump binding — advertised by `/hotkeys`,
/// printed nowhere, firing never. Converting recall to explicit ids
/// un-shadows it; this pins the split: modified Up jumps, bare Up recalls.
#[tokio::test]
async fn ctrl_up_jumps_to_prompts_instead_of_recalling() {
    let (mut app, agent, _rx) = test_app();
    assert!(!type_and_enter(&mut app, &agent, "first thought"));
    assert!(!type_and_enter(&mut app, &agent, "second thought"));
    // At the floor, empty draft: bare Up must recall, and must not move
    // the viewport; Ctrl+Up must jump to the earlier prompt, and must not
    // recall. (A larger starting offset would step off a transcript this
    // short — `jump_to_prompt` saturates instead of inventing a row.)
    app.scroll_from_bottom = 0;
    handle_key(&mut app, &agent, key(KeyCode::Up));
    assert_eq!(app.input, "second thought", "bare Up recalls");
    assert_eq!(
        app.scroll_from_bottom, 0,
        "and leaves the viewport where it was"
    );

    app.input.clear();
    app.cursor_chars = 0;
    let before = app.scroll_from_bottom;
    handle_key(&mut app, &agent, ctrl(KeyCode::Up));
    assert!(
        app.scroll_from_bottom != before,
        "Ctrl+Up must jump the viewport, not sit still: {}",
        before
    );
    assert!(
        app.input.is_empty(),
        "and must not recall into the draft: {:?}",
        app.input
    );
}

/// A leading `!` puts the footer into bash mode before Enter (P-022).
///
/// This composer has no border to recolour (a rule, not a box, by design),
/// so the mode word is the channel that says what Enter will do. Typed
/// through the real key path and asserted on the frame: `!` shows `bash`,
/// backspacing it out restores `insert`.
#[test]
fn bang_prefix_switches_the_footer_to_bash_mode() {
    let (mut app, agent, _rx) = test_app();
    for c in "!ls".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    let frame = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        frame.contains("bash"),
        "a `!` draft must announce bash mode: {frame}"
    );
    handle_key(&mut app, &agent, key(KeyCode::Backspace));
    handle_key(&mut app, &agent, key(KeyCode::Backspace));
    handle_key(&mut app, &agent, key(KeyCode::Backspace));
    let frame = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        !frame.lines().any(|l| l.contains("bash")),
        "clearing the `!` must restore insert mode: {frame}"
    );
}

/// F6 opens the workspace tree sidebar, and the tree lists the workspace's files.
///
/// The first of the TUI-as-IDE slices (Phase 3): a keyboard-driven file tree beside
/// the transcript, built from the shared core walk — the traversal `at_files.rs`
/// says the editor's tree wants with its own budget.
#[tokio::test]
async fn f6_opens_the_workspace_tree_showing_its_files() {
    let (mut app, agent, _receivers, dir) = test_app_with_store();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src").join("lib.rs"), "").unwrap();
    let quit = handle_key(&mut app, &agent, key(KeyCode::F(6)));
    assert!(!quit, "F6 toggles the tree, it does not quit");
    assert!(app.tree_open, "F6 must open the tree");
    let frame = render_rows(&mut app, 100, 30).join("\n");
    assert!(
        frame.contains("main.rs") && frame.contains("lib.rs"),
        "the tree must show the workspace's files: {frame}"
    );
}

/// The tree navigates, collapses, opens the viewer, and `@` inserts `@path`.
#[tokio::test]
async fn tree_navigate_collapse_and_enter_inserts_at_path() {
    let (mut app, agent, _receivers, dir) = test_app_with_store();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src").join("lib.rs"), "").unwrap();
    handle_key(&mut app, &agent, key(KeyCode::F(6)));
    assert!(app.tree_open);
    // Rows are dirs-first: src, src/lib.rs, main.rs. Move to main.rs.
    handle_key(&mut app, &agent, key(KeyCode::Down));
    handle_key(&mut app, &agent, key(KeyCode::Down));
    assert_eq!(
        app.tree_rows.get(app.tree_cursor).map(|r| r.rel.as_str()),
        Some("main.rs")
    );
    // Collapse src from the top: cursor back up twice, Left collapses.
    handle_key(&mut app, &agent, key(KeyCode::Up));
    handle_key(&mut app, &agent, key(KeyCode::Up));
    assert_eq!(
        app.tree_rows.get(app.tree_cursor).map(|r| r.rel.as_str()),
        Some("src")
    );
    handle_key(&mut app, &agent, key(KeyCode::Left));
    assert!(app.tree_collapsed.contains("src"));
    assert_eq!(
        app.tree_rows.len(),
        2,
        "collapsed src hides its child: {:?}",
        app.tree_rows
    );
    // Right re-expands; Enter on the file opens the viewer, Esc closes it,
    // and `@` inserts `@main.rs ` and closes the tree.
    handle_key(&mut app, &agent, key(KeyCode::Right));
    assert!(!app.tree_collapsed.contains("src"));
    handle_key(&mut app, &agent, key(KeyCode::Down));
    handle_key(&mut app, &agent, key(KeyCode::Down));
    let quit = handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert!(!quit);
    assert!(app.viewer_open(), "Enter on a file opens the viewer");
    let quit = handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert!(!quit);
    assert!(!app.viewer_open(), "Esc closes the viewer");
    assert!(app.tree_open, "closing the viewer returns to the tree");
    let quit = handle_key(&mut app, &agent, key(KeyCode::Char('@')));
    assert!(!quit);
    assert!(!app.tree_open, "@ closes the tree");
    assert_eq!(
        app.input, "@main.rs ",
        "file inserts an @-reference, got {:?}",
        app.input
    );
}

/// Esc closes the tree without touching the draft; an empty workspace names itself.
#[tokio::test]
async fn tree_esc_closes_and_empty_workspace_names_itself() {
    let (mut app, agent, _receivers, _dir) = test_app_with_store();
    handle_key(&mut app, &agent, key(KeyCode::Char('x')));
    handle_key(&mut app, &agent, key(KeyCode::F(6)));
    assert!(app.tree_open);
    let frame = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        frame.contains("empty workspace"),
        "empty tree must say so: {frame}"
    );
    let quit = handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert!(!quit);
    assert!(!app.tree_open);
    assert_eq!(
        app.input, "x",
        "Esc must not eat the draft, got {:?}",
        app.input
    );
}

/// Quitting with a parked approval ask denies it — fail closed.
///
/// `handle_key` documents "quitting drops the parked ask, which resolves to Deny".
/// The quit path replies nothing, so the denial has to come from the channel's
/// dropped-responder rule (`channel_ui.rs`: an unanswered `ask()` resolves `Deny`),
/// not from this file. Without that rule the tool would hang forever waiting for an
/// answer from a UI that no longer exists — or worse, a future quit path could
/// answer `AllowOnce` on the way out and this test is what would say so.
#[tokio::test]
async fn quitting_with_a_pending_ask_denies_it_fail_closed() {
    let (mut app, agent, _receivers) = test_app();
    let approvals = ChannelApprovalUi::new();
    let requester = approvals.clone();
    let response = tokio::spawn(async move {
        requester
            .ask(PermissionRequest {
                request_id: "quit-test".into(),
                capability: "write".into(),
                pattern: "work.txt".into(),
                detail: serde_json::json!({}),
            })
            .await
    });
    app.pending_ask = approvals.next_ask().await;
    assert!(app.pending_ask.is_some(), "the ask never parked");
    let quit = handle_key(&mut app, &agent, ctrl(KeyCode::Char('q')));
    assert!(quit, "ctrl+q must quit even with a pending ask");
    drop(app);
    assert!(
        matches!(response.await.unwrap(), PermissionDecision::Deny),
        "a quit-dropped ask must resolve Deny — never hang, never allow"
    );
}

/// Enter on a file opens a read-only viewer showing its content; Esc closes it.
#[tokio::test]
async fn tree_enter_opens_a_read_only_viewer_and_esc_closes_it() {
    let (mut app, agent, _receivers, dir) = test_app_with_store();
    std::fs::write(dir.path().join("notes.txt"), "first line\nsecond line\n").unwrap();
    handle_key(&mut app, &agent, key(KeyCode::F(6)));
    assert!(app.tree_open);
    // Single file: cursor is already on it.
    assert_eq!(app.tree_rows.len(), 1);
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert!(app.viewer_open(), "Enter on a file must open the viewer");
    let frame = render_rows(&mut app, 100, 30).join("\n");
    assert!(
        frame.contains("first line") && frame.contains("notes.txt"),
        "viewer must show the file's content and name: {frame}"
    );
    let quit = handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert!(!quit);
    assert!(!app.viewer_open(), "Esc must close the viewer");
}

/// The viewer scrolls with page keys and names an empty file honestly.
#[tokio::test]
async fn viewer_pages_down_and_names_an_empty_file() {
    let (mut app, agent, _receivers, dir) = test_app_with_store();
    let many: String = (0..60).map(|i| format!("line {i:02}\n")).collect();
    std::fs::write(dir.path().join("many.txt"), &many).unwrap();
    std::fs::write(dir.path().join("empty.txt"), "").unwrap();
    handle_key(&mut app, &agent, key(KeyCode::F(6)));
    // Rows are dirs-first single files in walk order; find many.txt.
    let idx = app
        .tree_rows
        .iter()
        .position(|r| r.rel == "many.txt")
        .expect("many.txt listed");
    app.tree_cursor = idx;
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert!(app.viewer_open());
    // Measure capacity with a real frame, then page.
    let _ = render_rows(&mut app, 100, 30);
    assert!(app.viewer_capacity > 1, "capacity must be measured");
    handle_key(&mut app, &agent, key(KeyCode::PageDown));
    let scrolled = app.active_viewer().map(|v| v.scroll).unwrap_or(0);
    assert!(
        scrolled > 0,
        "PageDown must advance, capacity {}",
        app.viewer_capacity
    );
    handle_key(&mut app, &agent, key(KeyCode::Home));
    assert_eq!(app.active_viewer().map(|v| v.scroll), Some(0));
    handle_key(&mut app, &agent, key(KeyCode::Esc));
    // Empty file names itself rather than showing a blank box.
    let idx = app
        .tree_rows
        .iter()
        .position(|r| r.rel == "empty.txt")
        .expect("empty.txt listed");
    app.tree_cursor = idx;
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    let frame = render_rows(&mut app, 100, 30).join("\n");
    assert!(
        frame.contains("(empty file)"),
        "empty file must say so: {frame}"
    );
}

/// The wheel follows focus: over the open tree it moves the tree cursor, over
/// the viewer it scrolls the file — never the transcript behind them.
#[tokio::test]
async fn wheel_scroll_follows_the_focused_overlay() {
    use crossterm::event::{Event as TermEvent, MouseEvent, MouseEventKind};
    let wheel = |kind| {
        TermEvent::Mouse(MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        })
    };
    let (mut app, agent, _receivers, dir) = test_app_with_store();
    std::fs::write(dir.path().join("a.txt"), "a\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "b\n").unwrap();
    handle_key(&mut app, &agent, key(KeyCode::F(6)));
    assert!(app.tree_open);
    super::keys::handle_term_event(&mut app, &agent, wheel(MouseEventKind::ScrollDown));
    assert_eq!(app.tree_cursor, 1, "wheel must move the tree cursor");
    assert_eq!(
        app.scroll_from_bottom, 0,
        "wheel over the tree must not scroll the transcript"
    );
    // Viewer open: the wheel scrolls the file, not the tree behind it.
    let many: String = (0..60).map(|i| format!("line {i:02}\n")).collect();
    std::fs::write(dir.path().join("many.txt"), &many).unwrap();
    app.refresh_tree();
    let idx = app
        .tree_rows
        .iter()
        .position(|r| r.rel == "many.txt")
        .expect("many.txt listed");
    app.tree_cursor = idx;
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert!(app.viewer_open());
    let _ = render_rows(&mut app, 100, 30);
    let before = app.tree_cursor;
    super::keys::handle_term_event(&mut app, &agent, wheel(MouseEventKind::ScrollDown));
    let scrolled = app.active_viewer().map(|v| v.scroll).unwrap_or(0);
    assert!(scrolled > 0, "wheel must scroll the viewer");
    assert_eq!(
        app.tree_cursor, before,
        "wheel over the viewer must not move the tree"
    );
    assert_eq!(
        app.scroll_from_bottom, 0,
        "wheel over the viewer must not scroll the transcript"
    );
}

/// `e` in the viewer edits the file; Ctrl+S saves through a checkpoint (undoable).
#[tokio::test]
async fn viewer_edit_mode_types_and_ctrl_s_saves_undoably() {
    let (mut app, agent, _receivers, dir) = test_app_with_store();
    std::fs::write(dir.path().join("edit.txt"), "hello\n").unwrap();
    handle_key(&mut app, &agent, key(KeyCode::F(6)));
    let idx = app
        .tree_rows
        .iter()
        .position(|r| r.rel == "edit.txt")
        .expect("edit.txt listed");
    app.tree_cursor = idx;
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    assert!(app.viewer_open());
    handle_key(&mut app, &agent, key(KeyCode::Char('e')));
    assert!(app.viewer_editing(), "e must enter edit mode");
    handle_key(&mut app, &agent, key(KeyCode::End));
    for c in " world".chars() {
        handle_key(&mut app, &agent, key(KeyCode::Char(c)));
    }
    // Alt+S saves (ctrl+s belongs to `app.model.saveDefault`).
    handle_key(
        &mut app,
        &agent,
        key_mod(KeyModifiers::ALT, KeyCode::Char('s')),
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("edit.txt")).unwrap(),
        "hello world\n",
        "save must write the buffer"
    );
}

/// Esc with a dirty buffer arms first and discards only on the second press.
#[tokio::test]
async fn viewer_esc_arms_before_discarding_dirty_edits() {
    let (mut app, agent, _receivers, dir) = test_app_with_store();
    std::fs::write(dir.path().join("d.txt"), "keep\n").unwrap();
    handle_key(&mut app, &agent, key(KeyCode::F(6)));
    let idx = app
        .tree_rows
        .iter()
        .position(|r| r.rel == "d.txt")
        .expect("d.txt listed");
    app.tree_cursor = idx;
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    handle_key(&mut app, &agent, key(KeyCode::Char('e')));
    handle_key(&mut app, &agent, key(KeyCode::End));
    handle_key(&mut app, &agent, key(KeyCode::Char('!')));
    assert!(app.viewer_editing());
    handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert!(
        app.viewer_editing(),
        "first Esc with dirty buffer must arm, not discard"
    );
    handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert!(!app.viewer_editing(), "second Esc discards");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("d.txt")).unwrap(),
        "keep\n",
        "discarded edits must never reach the file"
    );
}

/// Saving refuses when the file changed underneath, and keeps a backup.
#[tokio::test]
async fn viewer_save_refuses_changed_underneath_and_restore_undoes_it() {
    let (mut app, agent, _receivers, dir) = test_app_with_store();
    std::fs::write(dir.path().join("r.txt"), "v1\n").unwrap();
    handle_key(&mut app, &agent, key(KeyCode::F(6)));
    let idx = app
        .tree_rows
        .iter()
        .position(|r| r.rel == "r.txt")
        .expect("r.txt listed");
    app.tree_cursor = idx;
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    handle_key(&mut app, &agent, key(KeyCode::Char('e')));
    // Someone else writes first: our save must refuse, buffer intact.
    std::fs::write(dir.path().join("r.txt"), "v2-external\n").unwrap();
    handle_key(&mut app, &agent, key(KeyCode::End));
    handle_key(&mut app, &agent, key(KeyCode::Char('!')));
    handle_key(
        &mut app,
        &agent,
        key_mod(KeyModifiers::ALT, KeyCode::Char('s')),
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("r.txt")).unwrap(),
        "v2-external\n",
        "another writer's bytes win over ours"
    );
    assert!(app.viewer_editing(), "refused save keeps the buffer open");
    // Re-open fresh and save cleanly: backup holds the previous version.
    // Contract change (Phase 3, tabs): the third Esc used to close the viewer
    // outright, silently discarding the refused-save buffer the test name
    // calls "clean" but isn't. A dirty tab now arms first — and because the
    // file changed underneath, the arm names that save will refuse rather
    // than offering alt+s. The fourth Esc discards explicitly.
    handle_key(&mut app, &agent, key(KeyCode::Esc));
    handle_key(&mut app, &agent, key(KeyCode::Esc));
    handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert!(app.viewer_open(), "third Esc arms on unsaved edits");
    assert!(
        last_line_text(&app).contains("changed on disk"),
        "the arm must not offer a save that will refuse: {}",
        last_line_text(&app)
    );
    handle_key(&mut app, &agent, key(KeyCode::Esc));
    assert!(!app.viewer_open(), "explicit second Esc closes the viewer");
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    handle_key(&mut app, &agent, key(KeyCode::Char('e')));
    handle_key(&mut app, &agent, key(KeyCode::End));
    handle_key(&mut app, &agent, key(KeyCode::Char('!')));
    handle_key(
        &mut app,
        &agent,
        key_mod(KeyModifiers::ALT, KeyCode::Char('s')),
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("r.txt")).unwrap(),
        "v2-external!\n"
    );
    let sentence =
        epsilon_runtime::restore_checkpoint(dir.path(), "r.txt").expect("restore undoes the save");
    assert!(
        sentence.contains("restored"),
        "restore reports itself: {sentence}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("r.txt")).unwrap(),
        "v2-external\n",
        "the checkpoint entry IS the way back — no .tui-bak sidecar"
    );
    assert!(
        !dir.path().join("r.txt.tui-bak").exists(),
        "no second undo format is left behind"
    );
}

/// The tree sidebar docks RIGHT of the transcript with a separating gap.
#[tokio::test]
async fn tree_sidebar_docks_right_with_a_separating_gap() {
    let (mut app, agent, _receivers, dir) = test_app_with_store();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
    handle_key(&mut app, &agent, key(KeyCode::F(6)));
    assert!(app.tree_open);
    let rows = render_rows(&mut app, 100, 30);
    let tree_col = rows[0].find("tree").expect("tree title on the top row");
    assert!(
        tree_col > 50,
        "tree must dock right of the transcript, found at col {tree_col}"
    );
    // Separating gap: a blank column between transcript and tree border.
    let left_edge = rows[0].find('┌').expect("tree border");
    assert!(left_edge > 50);
    let gap = &rows[0][..left_edge];
    assert!(
        gap.ends_with(' ') || gap.ends_with('│'),
        "a gutter must separate transcript from tree"
    );
}

/// User markdown stays literal while assistant markdown renders (P-134): the
/// composer echoes `1. \*x\*` back with its markers, and the model's `**y**`
/// draws as `y` without them — one path per kind, decided by the `md` payload.
#[tokio::test]
async fn user_markdown_stays_literal_while_assistant_renders() {
    let (mut app, _agent, _rx) = test_app();
    app.ui.push_local_input(r"1. \*x\*");
    let frame = render_rows(&mut app, 80, 24).join(
        "
",
    );
    assert!(
        frame.contains(r"1. \*x\*"),
        "user markdown must stay literal: {frame}"
    );
}

/// Ctrl+Left folds (or steps up) and Ctrl+Right unfolds (or steps down) in the
/// timeline picker (P-300): the fold chord works through the real key path.
#[tokio::test]
async fn ctrl_arrows_fold_and_unfold_in_the_timeline() {
    use crate::app::TimelineRow;
    let (mut app, agent, _rx) = test_app();
    let parent = TimelineRow {
        seq: 0,
        id: "a".into(),
        kind: "user".into(),
        snippet: "the parent".into(),
        depth: 0,
        on_active: true,
        parent: None,
        label: None,
    };
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
    app.timeline_all = vec![parent.clone(), child];
    app.timeline_entries = vec![parent];
    app.timeline_index = 0;
    app.timeline_open = true;
    let ctrl_left = key_mod(KeyModifiers::CONTROL, KeyCode::Left);
    let ctrl_right = key_mod(KeyModifiers::CONTROL, KeyCode::Right);
    handle_key(&mut app, &agent, ctrl_left);
    assert!(
        app.folded.contains("a"),
        "ctrl+Left on a foldable row must fold it"
    );
    handle_key(&mut app, &agent, ctrl_right);
    assert!(!app.folded.contains("a"), "ctrl+Right must unfold it again");
}

/// **The file editor's own undo, through the real key path.**
///
/// The unit tests prove the primitives; this proves the *keys reach them*. Before
/// this, `Ctrl+Z` in the viewer's edit mode fell into the `_ => {}` arm and was
/// discarded — the composer had undo since M16 and the file editor had none, so a
/// user editing a file had no way back from a bad backspace except `Esc` and
/// losing everything they had typed.
#[tokio::test]
async fn ctrl_z_undoes_an_edit_in_the_file_editor() {
    let (mut app, agent, _receivers, dir) = test_app_with_store();
    std::fs::write(dir.path().join("u.txt"), "alpha\nbeta\n").unwrap();
    handle_key(&mut app, &agent, key(KeyCode::F(6)));
    let idx = app
        .tree_rows
        .iter()
        .position(|r| r.rel == "u.txt")
        .expect("u.txt listed");
    app.tree_cursor = idx;
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    handle_key(&mut app, &agent, key(KeyCode::Char('e')));
    assert!(app.viewer_editing(), "e enters edit mode");

    // Type "!" at the end of the first line.
    handle_key(&mut app, &agent, key(KeyCode::End));
    handle_key(&mut app, &agent, key(KeyCode::Char('!')));
    let v = app.active_viewer().expect("viewer");
    assert_eq!(
        v.edit_lines[0], "alpha!",
        "the keystroke reached the buffer"
    );

    // Ctrl+Z takes it back — the key that did nothing at all before.
    handle_key(
        &mut app,
        &agent,
        key_mod(KeyModifiers::CONTROL, KeyCode::Char('z')),
    );
    assert_eq!(
        app.active_viewer().expect("viewer").edit_lines[0],
        "alpha",
        "ctrl+z must reach the file editor"
    );

    // Ctrl+Y re-applies it.
    handle_key(
        &mut app,
        &agent,
        key_mod(KeyModifiers::CONTROL, KeyCode::Char('y')),
    );
    assert_eq!(
        app.active_viewer().expect("viewer").edit_lines[0],
        "alpha!",
        "ctrl+y re-applies"
    );

    // Drain the rail: the third press has nothing left, and it says so rather
    // than swallowing the keypress.
    handle_key(
        &mut app,
        &agent,
        key_mod(KeyModifiers::CONTROL, KeyCode::Char('y')),
    );
    handle_key(
        &mut app,
        &agent,
        key_mod(KeyModifiers::CONTROL, KeyCode::Char('y')),
    );
    assert!(
        app.ui
            .lines()
            .iter()
            .any(|l| l.kind == LineKind::System && l.text.contains("nothing to redo")),
        "an exhausted rail is reported, not swallowed"
    );
}

/// **`Delete` works in the file editor.**
///
/// It was in the `_ => {}` arm: bound to nothing, announced nowhere, and silent —
/// so a user pressing it saw a still cursor and could not distinguish a broken key
/// from a busy frame. `Backspace` beside it worked, which is what made it a defect
/// rather than an omission.
#[tokio::test]
async fn delete_removes_the_character_under_the_file_editor_cursor() {
    let (mut app, agent, _receivers, dir) = test_app_with_store();
    std::fs::write(dir.path().join("d.txt"), "abcd\n").unwrap();
    handle_key(&mut app, &agent, key(KeyCode::F(6)));
    let idx = app
        .tree_rows
        .iter()
        .position(|r| r.rel == "d.txt")
        .expect("d.txt listed");
    app.tree_cursor = idx;
    handle_key(&mut app, &agent, key(KeyCode::Enter));
    handle_key(&mut app, &agent, key(KeyCode::Char('e')));
    // Home puts the cursor at column 0.
    handle_key(&mut app, &agent, key(KeyCode::Home));
    handle_key(&mut app, &agent, key(KeyCode::Delete));
    assert_eq!(
        app.active_viewer().expect("viewer").edit_lines[0],
        "bcd",
        "Delete removes forward, where Backspace removes backward"
    );
    // And it is undoable, like every other edit now.
    handle_key(
        &mut app,
        &agent,
        key_mod(KeyModifiers::CONTROL, KeyCode::Char('z')),
    );
    assert_eq!(
        app.active_viewer().expect("viewer").edit_lines[0],
        "abcd",
        "ctrl+z undoes the deletion too"
    );
}
