//! Golden frames: the whole screen, drawn by the real draw path.
//!
//! # Why this file exists
//!
//! Every other TUI test here checks one widget in isolation, which is the right
//! unit for most of them and wrong for exactly one question: **what does the
//! screen actually look like**. A composer that renders perfectly on its own and
//! a footer that renders perfectly on its own can still be assembled into a frame
//! where the two overlap, where the composer has no room, or where the transcript
//! is squeezed to nothing — and every unit test would stay green.
//!
//! So these tests drive [`draw`] itself, through the same `TestBackend` a unit
//! test uses, and assert on the assembled screen. They are deliberately about
//! *structure* rather than pixel-exact text: a golden test that pins every
//! character fails on every deliberate copy change and gets rewritten without
//! being read, which is worse than no test. What is pinned here is the set of
//! properties a frame must have at every size — and the one property this module
//! exists for, which is that a row is never wider than the terminal.
//!
//! `render_rows` calls `draw(app)`, the production entry point. A test that
//! hand-assembled the shell would be testing the test.

use super::tests_support::{park_ask, render_buffer, render_rows, test_app};
use super::*;
use crate::ui::theme::{ColorMode as ThemeColorMode, Theme};

/// Pin the look so a golden frame can assert exact colours and glyphs.
///
/// `Theme::detect()` reads `NO_COLOR`, `COLORTERM` and `EPSILON_ASCII` from
/// whatever environment the test runner happens to have, which is right for the
/// product and wrong for a test that names a colour. The theme is a field of
/// `App`, so this sets it rather than re-detecting it.
fn pin_theme(app: &mut App, mode: ThemeColorMode) {
    app.theme = Theme::new(mode, crate::ui::theme::UNICODE);
}

/// One screen cell, for assertions that are about style rather than text.
fn cell(buffer: &ratatui::buffer::Buffer, x: u16, y: u16) -> &ratatui::buffer::Cell {
    &buffer[(x, y)]
}

/// A frame with a user turn, a tool call, a result and a few notices.
///
/// Built through the public state API on purpose: `push_line` is private to
/// `state`, and a test that reached into it would stop exercising the paths a
/// real session uses.
fn busy_app() -> App {
    let mut app = bench_app(UiState::new());
    app.ui.push_local_input("explain the layout module");
    for text in [
        "The layout module owns the spatial model.",
        "It splits the frame into a header, a transcript, a composer and a footer,",
        "and it decides what happens when the terminal is too small.",
    ] {
        app.ui.push_notice(text);
    }
    app.input = "and the composer?".into();
    app.cursor_chars = app.input.chars().count();
    app
}

/// **On a plain terminal the model's answer and a harness notice are not the same row.**
///
/// This is the owner's 2026-10-04 report — *"the output of the model is not shown"* — and the
/// defect behind it was not a missing event or a renderer that dropped a row. The answer **was**
/// on the screen; it drew the **same gutter character** as a status line, so it read as one. On a
/// banded truecolor terminal a colour band separates them, which is why it survived for a month
/// and why the report was filed as "nothing is rendered" rather than "two roles look alike".
///
/// Asserted on the rendered **rows**, and on the band's absence, because that is the only
/// configuration where the distinction has to come from the glyph. The control below re-runs the
/// same comparison with a band, which is what proves this failure is the glyph and not the
/// harness: that half passes either way.
#[test]
fn the_answer_is_not_a_notice_when_there_is_no_band() {
    let mut app = bench_app(UiState::new());
    // Ansi16 + ASCII is what `detect()` returns in a plain Windows console without
    // COLORTERM or WT_SESSION, and it is the one mode with no band to carry the role.
    app.theme = Theme::new(ThemeColorMode::Ansi16, crate::ui::theme::ASCII);
    assert!(
        !app.theme.has_surfaces(),
        "this test is about the bandless case; a band would carry the distinction instead"
    );
    app.ui
        .apply(&epsilon_contracts::agent_event::AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(
                epsilon_contracts::message::AssistantMessage {
                    id: "msg-gutters".into(),
                    timestamp_ms: 0,
                    provider: "faux".into(),
                    model_id: "faux-1".into(),
                    content: vec![epsilon_contracts::content::ContentBlock::text(
                        "Run complete.",
                    )],
                    stop_reason: epsilon_contracts::message::StopReason::EndTurn,
                    error_message: None,
                    usage: Default::default(),
                },
            )),
        });
    app.ui.push_notice("permission allowed");

    let rows = render_rows(&mut app, 80, 24);
    let answer = rows
        .iter()
        .position(|r| r.contains("Run complete."))
        .unwrap_or_else(|| panic!("the answer never reached the screen: {rows:#?}"));
    let notice = rows
        .iter()
        .position(|r| r.contains("permission allowed"))
        .unwrap_or_else(|| panic!("the notice never reached the screen: {rows:#?}"));
    assert_ne!(
        rows[answer].chars().next(),
        rows[notice].chars().next(),
        "the model's answer and a harness notice are the same pixels without colour: \
         {:?} / {:?}",
        rows[answer],
        rows[notice]
    );

    // The control. A banded terminal separated these before the fix and still does, so this
    // half cannot be what failed — which is what makes the assertion above a claim about the
    // glyph rather than about the harness.
    let mut banded = bench_app(UiState::new());
    pin_theme(&mut banded, ThemeColorMode::Truecolor);
    assert!(banded.theme.has_surfaces());
    banded
        .ui
        .apply(&epsilon_contracts::agent_event::AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(
                epsilon_contracts::message::AssistantMessage {
                    id: "msg-gutters".into(),
                    timestamp_ms: 0,
                    provider: "faux".into(),
                    model_id: "faux-1".into(),
                    content: vec![epsilon_contracts::content::ContentBlock::text(
                        "Run complete.",
                    )],
                    stop_reason: epsilon_contracts::message::StopReason::EndTurn,
                    error_message: None,
                    usage: Default::default(),
                },
            )),
        });
    banded.ui.push_notice("permission allowed");
    let rows = render_rows(&mut banded, 80, 24);
    let answer = rows
        .iter()
        .position(|r| r.contains("Run complete."))
        .expect("the answer is drawn");
    let notice = rows
        .iter()
        .position(|r| r.contains("permission allowed"))
        .expect("the notice is drawn");
    assert_ne!(
        rows[answer].chars().next(),
        rows[notice].chars().next(),
        "a banded terminal separates these today, and must keep to"
    );
}

/// **The bandless case is a *pair* of glyph sets, and the first version of this test checked
/// only one.**
///
/// The test above pins `Theme::new(Ansi16, ASCII)` — which is what let a half-finished fix
/// pass. `Glyphs::detect` returns `UNICODE` on every ordinary POSIX terminal and on a conhost
/// with a UTF-8 code page (`theme.rs:392-394`), while `ColorMode::detect` returns `Ansi16`
/// unless `COLORTERM` says truecolor (`theme.rs:84-96`). So **UNICODE + Ansi16 is the far more
/// common bandless configuration**, and it is a different table with its own `bullet` and
/// `assistant`. An independent review on 2026-10-04 found the answer and a notice still drawing
/// the same `•` there.
///
/// Asserted over both tables for that reason, and as a property — *no glyph set may give these
/// two roles the same character* — rather than as the two literal characters, because a
/// property survives the next character change and a literal does not.
#[test]
fn no_glyph_set_gives_an_answer_and_a_notice_the_same_character() {
    for (name, glyphs) in [
        ("ASCII", crate::ui::theme::ASCII),
        ("UNICODE", crate::ui::theme::UNICODE),
    ] {
        // Both tables, bandless and banded, because the choice of table and the choice of
        // colour mode are independent and a reader can land in any combination.
        for (mode_name, mode) in [
            ("Ansi16", ThemeColorMode::Ansi16),
            ("None", ThemeColorMode::None),
            ("Truecolor", ThemeColorMode::Truecolor),
        ] {
            let theme = Theme::new(mode, glyphs);
            assert_ne!(
                glyphs.assistant, glyphs.bullet,
                "{name}/{mode_name}: the model's answer and a harness notice draw the same \
                 character, so a reply reads as a status line without colour"
            );
            // And the painted row, for the case that actually produces the report.
            let mut app = bench_app(UiState::new());
            app.theme = theme;
            app.ui
                .apply(&epsilon_contracts::agent_event::AgentEvent::MessageEnd {
                    message: Box::new(Message::Assistant(
                        epsilon_contracts::message::AssistantMessage {
                            id: "msg-pair".into(),
                            timestamp_ms: 0,
                            provider: "faux".into(),
                            model_id: "faux-1".into(),
                            content: vec![epsilon_contracts::content::ContentBlock::text(
                                "Run complete.",
                            )],
                            stop_reason: epsilon_contracts::message::StopReason::EndTurn,
                            error_message: None,
                            usage: Default::default(),
                        },
                    )),
                });
            app.ui.push_notice("permission allowed");
            let rows = render_rows(&mut app, 80, 24);
            let answer = rows
                .iter()
                .position(|r| r.contains("Run complete."))
                .unwrap_or_else(|| {
                    panic!("{name}/{mode_name}: the answer was not drawn: {rows:#?}")
                });
            let notice = rows
                .iter()
                .position(|r| r.contains("permission allowed"))
                .unwrap_or_else(|| panic!("{name}/{mode_name}: the notice was not drawn"));
            assert_ne!(
                rows[answer].chars().next(),
                rows[notice].chars().next(),
                "{name}/{mode_name}: same pixels for the answer and the notice"
            );
        }
    }
}

/// The gutter glyph appears **once** per row, on every row kind.
///
/// This is the M198 defect, stated as a property rather than as three corrected
/// string literals, because the three literal assertions that used to pin it
/// (`▸ ▸ bash`, `• • run finished`, `✓ ✓ …`) each had to be edited by hand and
/// none of them noticed the fourth and fifth rows. Four of the seven row kinds
/// drew the same character twice — once in the gutter, once as a body marker or
/// a card title — and it read as a rendering fault, not as a marker.
///
/// The check is precise rather than a string comparison: take the glyph the
/// gutter actually painted (from the **buffer cell at column 0**, not from a
/// literal), and assert the body does not open with that same glyph. Reading the
/// cell is the point — it is what makes the test about the drawn frame rather
/// than about a set of characters a future theme change would invalidate.
/// Code page 437's **low** range, from `0x01` to `0x1F` — the control slots DOS
/// reused for symbols, which is why `↑` (0x18) and `↓` (0x19) are drawable even
/// though neither is ASCII.
const CP437_MARKS: &str = "☺☻♥♦♣♠•◘○◙♂♀♪♫☼►◄↕‼¶§▬↨↑↓→←∟↔▲▼";

/// Code page 437's upper half, from `0x80` to `0xFF`.
const CP437_HIGH: &str = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■\u{A0}";

/// Whether a console on code page 437 can draw this character.
///
/// **`…` is not drawable**, which is the surprise worth writing down: it is at 0x85 in
/// Windows-1252, where every modern terminal learned it, and DOS put `à` there instead. A
/// console that falls back to a font substitution will show it; a console that does not —
/// which is the case this test exists for — shows a box. `·` (0xFA) and `—` (0x97) *are*
/// in CP437, so prose may keep them.
fn cp437_can_draw(c: char) -> bool {
    c.is_ascii() || CP437_MARKS.contains(c) || CP437_HIGH.contains(c)
}

#[test]
fn an_ascii_frame_draws_no_character_a_legacy_console_cannot() {
    // M219/A22: the owner's report was "boxes with `?`" in `cmd.exe` and PowerShell. The cause was
    // a *drawn* character the repertoire could not substitute, and the reason nothing caught it is
    // structural: a `TestBackend` holds the characters, and `console-probe.py` reads the console
    // **buffer** — so every frame artifact in this repository would have looked perfect on a CP437
    // console. This test is the closest thing a test can see: paint the frame with the ASCII table
    // and read back what was actually drawn.
    //
    // **The assertion is over the drawn rows, not over the glyph table**, so a character typed as a
    // literal in the chrome fails here while `ascii_glyphs_are_seven_bit_only` stays green — that
    // check only knows the table's own fields.
    //
    // The frames hold **no user or model prose**, because that text is not the harness's to change:
    // a message in Greek is a message in Greek. Every character below therefore belongs to the UI.
    let mut offenders: Vec<String> = Vec::new();
    let mut frames = 0usize;
    for (w, h) in [(80u16, 24u16), (100, 30), (120, 40)] {
        for panel in 0..4 {
            let mut app = bench_app(UiState::with_glyphs(crate::ui::theme::ASCII));
            app.theme = Theme::new(ThemeColorMode::Truecolor, crate::ui::theme::ASCII);
            match panel {
                // 0: the plain frame. 1: the model picker. 2: the command palette. 3: the slash
                // popup — the three overlays are where every separator, arrow and marker is drawn.
                1 => app.picker_open = true,
                2 => {
                    app.palette_open = true;
                    app.input = "go".into();
                }
                3 => {
                    app.input = "/se".into();
                    app.slash_open = true;
                }
                _ => {}
            }
            frames += 1;
            for row in render_rows(&mut app, w, h) {
                let bad: Vec<char> = row
                    .chars()
                    .filter(|c| !cp437_can_draw(*c))
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                if !bad.is_empty() {
                    offenders.push(format!("{w}x{h} panel {panel}: {bad:?} in {row:?}"));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "the ASCII repertoire drew {} row(s) a CP437 console cannot render, across {} frames:\n{}\n\
         Each of these is a character typed as a literal rather than taken from `Glyphs`. The fix is \
         the glyph field: `●`, `★`, `⟳` and anything astral-plane are not in CP437 at all, so a \
         console renders them as a box or a `?` -- the report this test exists for.",
        offenders.len(),
        frames,
        offenders.join("\n")
    );
}

#[test]
fn no_transcript_row_repeats_its_gutter_glyph() {
    let mut app = bench_app(UiState::new());
    app.ui.push_local_input("a user turn");
    app.ui.push_notice("a lifecycle notice");
    app.ui
        .apply(&crate::state::agent_event_tool_result("bash", "boom", true));

    let (buffer, rows) = render_buffer(&mut app, 80, 24);
    // Only the transcript region. The composer's rule is a row of `─`, so a
    // whole-frame sweep reads its own divider as a gutter glyph repeated in its
    // body — the rule is not a transcript row and has no gutter.
    let transcript = App::transcript_window(ratatui::layout::Rect::new(0, 0, 80, 24), 1);
    let mut checked = 0usize;
    for index in transcript.y..transcript.bottom() {
        let row = &rows[index as usize];
        let glyph = cell(&buffer, 0, index).symbol().to_string();
        if glyph.trim().is_empty() {
            continue; // a row with no marker cannot repeat one
        }
        checked += 1;
        // The gutter is two columns: the glyph and its pad. The body starts at
        // column 2, and "repeats it" means the body opens with the same glyph.
        let body: String = row.chars().skip(crate::ui::transcript::GUTTER).collect();
        assert!(
            !body.starts_with(&glyph),
            "row {index} draws its gutter glyph {glyph:?} twice — once in the \
             gutter and again in the body: {row:?}"
        );
    }
    // The frame has to have exercised the property, or this test proves nothing
    // about a frame that happened to be empty.
    assert!(
        checked >= 3,
        "only {checked} marker-bearing rows were checked; the fixture is not \
         building the transcript this test is about"
    );
}

#[test]
fn a_full_frame_is_exactly_the_terminal_size_at_every_supported_width() {
    // The property this module exists for. A row wider than the terminal wraps,
    // which turns a one-line bar into two and shifts everything below it — the
    // class of bug that makes a TUI look broken rather than merely plain.
    for (width, height) in [(44u16, 10u16), (60, 20), (80, 24), (100, 30), (160, 48)] {
        let mut app = busy_app();
        let rows = render_rows(&mut app, width, height);
        assert_eq!(
            rows.len(),
            height as usize,
            "{width}x{height}: wrong row count"
        );
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(
                row.chars().count(),
                width as usize,
                "{width}x{height} row {index} is {} columns: {row:?}",
                row.chars().count()
            );
        }
    }
}

#[test]
fn the_frame_shows_the_four_things_it_is_for() {
    // Header, transcript, composer, footer — each present and identifiable at a
    // size a developer actually uses. If a change squeezes one out at 80x24, that
    // is a regression no unit test would have caught.
    let mut app = busy_app();
    let rows = render_rows(&mut app, 80, 24);
    let screen = rows.join("\n");

    assert!(
        rows[0].contains("idle"),
        "no header on row 0: {:?}",
        rows[0]
    );
    assert!(
        screen.contains("The layout module owns the spatial model."),
        "the transcript is missing its content: {screen}"
    );
    assert!(
        screen.contains("and the composer?"),
        "the composer lost the draft: {screen}"
    );
    assert!(
        rows[rows.len() - 2..].iter().any(|r| r.contains("enter")),
        "no key hints at the bottom: {:?}",
        &rows[rows.len() - 2..]
    );
    // The rule that separates the transcript from the composer, and only that
    // one. There used to be two: a second full-width rule sat between the header
    // and the transcript, drawn by the same function in the same style, so
    // neither line said what it divided — and at 80x24 the pair was 2 of 24 rows.
    // The count below is pinned deliberately: moving it from 2 to 1 is the
    // reviewable evidence that cutting the rule was intended, and that the row it
    // held went to the transcript rather than going missing. The glyph comes from
    // the theme the app resolved rather than from a literal, because the
    // repertoire is a runtime decision (and a console that cannot render the
    // Unicode rule gets the ASCII one).
    let rule = app
        .theme()
        .glyphs
        .horizontal
        .chars()
        .next()
        .expect("a horizontal glyph");
    let rules = rows
        .iter()
        .filter(|r| !r.is_empty() && r.chars().all(|c| c == rule))
        .count();
    assert_eq!(
        rules, 1,
        "expected one full-width rule made of {rule:?}, found {rules}: {screen}"
    );
}

#[test]
fn a_long_transcript_never_pushes_the_composer_off_the_screen() {
    // A long conversation scrolls; it does not grow. If the transcript claimed
    // more rows than it has, the composer and footer would be laid out past the
    // bottom of the frame and the user could not type.
    let mut app = bench_app(UiState::new());
    for index in 0..200 {
        app.ui.push_notice(format!("ROW-{index:03}"));
    }
    let rows = render_rows(&mut app, 60, 20);
    let screen = rows.join("\n");
    assert!(screen.contains("ROW-199"), "the tail must be visible");
    // The composer and the footer are the last two rows whatever the transcript
    // does. Asserting on the footer's hints rather than on the composer's marker
    // is deliberate: the marker scrolls with the draft, and a test that pinned
    // its row would fail on a one-row change to the composer.
    assert!(
        rows[rows.len() - 2..]
            .iter()
            .any(|r| r.contains("enter") || r.contains("ask anything")),
        "the composer and footer were pushed off the frame: {:?}",
        &rows[rows.len() - 4..]
    );
}

#[test]
fn below_the_floor_the_frame_says_so_instead_of_drawing_a_broken_layout() {
    // A 40x8 window cannot hold a conversation. The failure mode to avoid is not
    // "ugly" — it is a frame with a composer but no room for the transcript, so
    // the user types into something they cannot read.
    let mut app = busy_app();
    let rows = render_rows(&mut app, 40, 8);
    let screen = rows.join("\n");
    assert!(screen.contains("too small"), "{screen}");
    assert!(
        !screen.contains("The layout module owns"),
        "a frame below the floor must not draw a partial transcript: {screen}"
    );
}

#[test]
fn resizing_mid_session_keeps_every_row_inside_the_new_width() {
    // Resize is the case where cached coordinates go stale. Drawing wide, then
    // narrow, then wide again must produce well-formed frames each time — the
    // renderer holds no width of its own.
    let mut app = busy_app();
    for (width, height) in [(120u16, 30u16), (50, 14), (120, 30), (44, 10)] {
        let rows = render_rows(&mut app, width, height);
        assert_eq!(rows.len(), height as usize);
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(
                row.chars().count(),
                width as usize,
                "after resizing to {width}x{height}, row {index} is {} columns",
                row.chars().count()
            );
        }
    }
}

#[test]
fn an_empty_session_gets_the_first_run_screen_inside_a_well_formed_frame() {
    // The first frame a new user sees. It must be the welcome screen — a blank
    // body reads as a hung program — and it must still be a frame, not a wall of
    // text with no composer.
    let mut app = bench_app(UiState::new());
    let rows = render_rows(&mut app, 100, 30);
    let screen = rows.join("\n");
    assert!(screen.contains("E P S I L O N"), "{screen}");
    assert!(
        screen.contains("ask anything"),
        "the composer placeholder is missing: {screen}"
    );
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.chars().count(), 100, "row {index} is malformed");
    }
}

#[tokio::test]
async fn a_pending_approval_is_visible_in_a_well_formed_frame() {
    // The one frame where being wrong has consequences. An ask parked on the
    // shared queue must reach the screen: the user is being asked to authorize
    // work, and a dialog that is laid out but not drawn would still answer keys.
    let (mut app, _agent, _receivers) = test_app();
    let approvals = ChannelApprovalUi::new();
    let _response = park_ask(&mut app, &approvals, "req-golden").await;

    let rows = render_rows(&mut app, 90, 26);
    let screen = rows.join("\n");
    assert!(screen.contains("permission required"), "{screen}");
    assert!(screen.contains("rm -rf build"), "{screen}");
    assert!(screen.contains('y') && screen.contains('n'), "{screen}");
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(
            row.chars().count(),
            90,
            "the approval frame is malformed on row {index}: {row:?}"
        );
    }
}

// ---- the transcript's decoration, seen on a real frame -----------------------
//
// The tests in `ui::transcript` call `decorate` directly, and that is precisely
// how the whole decoration system came to be dead code: they proved the function
// worked while nothing on screen called it. The two tests below go through
// `draw(app)` — the production entry point — and read the *cells*, because a band
// and a gutter glyph's colour are invisible to a text-only assertion.

#[test]
fn a_user_turn_carries_its_gutter_and_band_on_a_real_frame() {
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Truecolor);
    let surface = app.theme.surface_user().bg;
    let user_style = app.theme.user();
    app.ui
        .push_local_input("what does the gutter look like now?");

    let (buffer, rows) = render_buffer(&mut app, 80, 24);
    let row_index = rows
        .iter()
        .position(|r| r.contains("what does the gutter look like now?"))
        .unwrap_or_else(|| panic!("the user turn never reached the screen: {rows:#?}"));
    let row = &rows[row_index];

    // 1. The two-column gutter, and the `▎` that says "this is the person
    //    talking" in the leftmost cell.
    assert!(
        row.starts_with("▎ "),
        "user row {row_index} has no gutter marker: {row:?}"
    );
    assert_eq!(cell(&buffer, 0, row_index as u16).symbol(), "▎");
    assert_eq!(
        cell(&buffer, 0, row_index as u16).style().fg,
        user_style.fg,
        "the gutter glyph must carry the user role's colour"
    );
    // 2. The band, at the glyph, behind the text, and out to the terminal edge.
    for x in [0u16, 2, 20, 79] {
        assert_eq!(
            cell(&buffer, x, row_index as u16).style().bg,
            surface,
            "column {x} of the user row is not on the user surface"
        );
    }
    // 3. Exactly one marker. `❯` from the render cache; the `"> "` that
    //    `push_local_input` used to store is gone, so the row reads
    //    `▎ ❯ what does …` and not `▎ ❯ > what does …`.
    assert!(
        row.contains("▎ ❯ what does"),
        "the user row must carry one marker, not two: {row:?}"
    );
    // 4. The band belongs to that row and nothing else: a blank line above it is
    //    not painted as part of the message.
    assert_ne!(
        cell(&buffer, 20, row_index as u16 - 1).style().bg,
        surface,
        "the band leaked onto the row above the message"
    );
    // 5. And the frame is still exactly the terminal's width — a band that
    //    overflows is a wrapped row, which is what the one column of slack used
    //    to be buying.
    for (index, r) in rows.iter().enumerate() {
        assert_eq!(
            r.chars().count(),
            80,
            "row {index} is {} columns: {r:?}",
            r.chars().count()
        );
    }
}

#[test]
fn a_wrapped_banded_message_keeps_every_row_inside_the_frame() {
    // The width trap the audit named: the cache wraps the BODY to
    // `frame - GUTTER` and `decorate` pads to the FRAME width. Wrap to the frame
    // width instead and every banded row is two cells over the edge — the row
    // wraps, one message becomes two rows and everything below it shifts. This is
    // the production path, at a realistic narrow width.
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Truecolor);
    let surface = app.theme.surface_user().bg;
    app.ui.push_local_input(
        "the user turn is the one row that must never wrap its band past the terminal \
         edge, because a band that overflows turns one message into two rows and moves \
         everything under it",
    );

    let (buffer, rows) = render_buffer(&mut app, 60, 20);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(
            row.chars().count(),
            60,
            "row {index} is {} columns: {row:?}",
            row.chars().count()
        );
    }
    let banded: Vec<usize> = (0..rows.len())
        .filter(|&y| cell(&buffer, 59, y as u16).style().bg == surface)
        .collect();
    assert!(
        banded.len() >= 3,
        "a wrapped user turn must band every one of its rows, got {banded:?}: {rows:#?}"
    );
    assert_eq!(
        banded,
        (banded[0]..banded[0] + banded.len()).collect::<Vec<_>>(),
        "the banded rows must be contiguous"
    );
    // One marker, on the first row only: a `❯` repeated on every continuation row
    // would read as a new message per line.
    assert!(
        rows[banded[0]].starts_with("▎ ❯ the user turn"),
        "the first banded row must carry the gutter and the one marker: {:?}",
        rows[banded[0]]
    );
    for index in banded.iter().skip(1) {
        assert!(
            !rows[*index].contains('❯'),
            "continuation row {index} repeated the marker: {:?}",
            rows[*index]
        );
    }
}

#[test]
fn a_tool_call_and_a_notice_are_distinguishable_on_a_real_frame_without_colour() {
    // `NO_COLOR`: no color at all, and no filled bands — `surface_*` returns the
    // default style on anything but truecolor — so glyph and position are all
    // that is left. A pending call and a lifecycle notice used to share
    // `glyphs.bullet`, in every colour mode: `• …` and `• …`, the same pixels.
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::None);
    // Built through `apply`, the event reducer a real session feeds: a tool-call
    // row only exists because an assistant message asked for a tool.
    app.ui
        .apply(&epsilon_contracts::agent_event::AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(
                epsilon_contracts::message::AssistantMessage {
                    id: "msg_golden_call".into(),
                    timestamp_ms: 0,
                    provider: "faux".into(),
                    model_id: "faux-1".into(),
                    content: vec![epsilon_contracts::content::ContentBlock::ToolCall {
                        id: "tc_golden".into(),
                        name: "bash".into(),
                        arguments: serde_json::json!({ "command": "cargo test -p epsilon-tui" }),
                    }],
                    stop_reason: epsilon_contracts::message::StopReason::ToolUse,
                    error_message: None,
                    usage: epsilon_contracts::message::Usage::default(),
                },
            )),
        });
    app.ui.push_notice("run finished · 3 files changed");

    let (buffer, rows) = render_buffer(&mut app, 80, 24);
    assert!(
        !app.theme.has_surfaces(),
        "NO_COLOR must not claim filled bands"
    );
    // M126: the call is a **card** now, so the row to find is its title line —
    // `▸ bash $ cargo test -p epsilon-tui · pending` — and the command it is about
    // to run is both the headline (M153) and the body under
    // it. Mutated from `tool-call bash cargo test`, which is the flat row this
    // slice replaced.
    let call_row = rows
        .iter()
        .position(|r| r.contains("bash $ cargo test -p epsilon-tui · pending"))
        .unwrap_or_else(|| panic!("the tool card never reached the screen: {rows:#?}"));
    let notice_row = rows
        .iter()
        .position(|r| r.contains("run finished"))
        .unwrap_or_else(|| panic!("the notice never reached the screen: {rows:#?}"));

    // The gutter glyph, then the card's own title. **One** glyph, not two: the
    // marker used to repeat the gutter's character in the body, so every one of
    // these rows read `▸ ▸ bash` and `• • run finished` — the same character
    // twice, which looks like a rendering fault rather than a marker. The role is
    // still unambiguous, because it is the *gutter* that differs between a pending
    // call and a notice, and that is the cell the next two assertions compare.
    assert!(
        rows[call_row].starts_with("▸ bash $ cargo test -p epsilon-tui · pending"),
        "tool-card row {call_row} must carry the call gutter and the card title: {:?}",
        rows[call_row]
    );
    assert!(
        rows[notice_row].starts_with("• run finished"),
        "notice row {notice_row} must carry the notice gutter and its text: {:?}",
        rows[notice_row]
    );
    assert_ne!(
        cell(&buffer, 0, call_row as u16).symbol(),
        cell(&buffer, 0, notice_row as u16).symbol(),
        "with colour off, two roles that share a gutter glyph are the same pixels"
    );
    assert_eq!(
        cell(&buffer, 0, call_row as u16).style().fg,
        Some(ratatui::style::Color::Reset),
        "NO_COLOR must not emit colour: {:?}",
        cell(&buffer, 0, call_row as u16).style()
    );
    // The band is withheld, as it must be, and the row still says what it is: the
    // marker is in the body text, not only in a colour that is gone.
    assert_eq!(
        cell(&buffer, 0, call_row as u16).style().bg,
        Some(ratatui::style::Color::Reset),
        "a banded call on a no-surface theme would be an approximation"
    );
    // The card's body is the command, so a pending call still says what it will do.
    assert!(
        rows[call_row + 1].contains("cargo test -p epsilon-tui"),
        "the pending card's body must name the call: {:?}",
        rows[call_row + 1]
    );
}

#[test]
fn an_answer_and_its_reasoning_are_distinguishable_on_a_real_frame_without_colour() {
    // The other half of the `NO_COLOR` story, on the production draw path. A
    // filled band is truecolor-only, so off truecolor the assistant row and the
    // thinking row had nothing to tell them apart: `Emphasis::Plain` collapses to
    // `Style::default()` under `NO_COLOR` as well, leaving two blank gutter cells
    // and two runs of identical plain prose. `Theme::has_surfaces` promises a
    // gutter-glyph fallback for exactly this; this is the frame that promise is
    // about, because a `row_style` assertion alone cannot see the cache, the
    // decoration and the paragraph agreeing.
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::None);
    let assistant = "the answer is B because the first option wraps at 60 columns";
    let thinking = "weighing the two options before answering";
    // The live path: `streaming_thinking` is what a `MessageUpdate` fills, and
    // `MessageEnd` is what turns it into a Thinking row. Present in every real
    // session that shows reasoning at all.
    let message = Message::Assistant(epsilon_contracts::message::AssistantMessage {
        id: "msg_golden_roles".into(),
        timestamp_ms: 0,
        provider: "faux".into(),
        model_id: "faux-1".into(),
        content: vec![
            epsilon_contracts::content::ContentBlock::Thinking {
                text: thinking.into(),
                signature: None,
            },
            epsilon_contracts::content::ContentBlock::text(assistant.to_string()),
        ],
        stop_reason: epsilon_contracts::message::StopReason::EndTurn,
        error_message: None,
        usage: epsilon_contracts::message::Usage::default(),
    });
    // Two apps fed the identical events, so the only difference between the two
    // frames below is the theme.
    let mut rich = bench_app(UiState::new());
    pin_theme(&mut rich, ThemeColorMode::Truecolor);
    for target in [&mut app, &mut rich] {
        target
            .ui
            .apply(&epsilon_contracts::agent_event::AgentEvent::MessageUpdate {
                message: Box::new(message.clone()),
            });
        target
            .ui
            .apply(&epsilon_contracts::agent_event::AgentEvent::MessageEnd {
                message: Box::new(message.clone()),
            });
    }
    assert!(app.ui.lines().iter().any(|l| l.kind == LineKind::Thinking));

    let (buffer, rows) = render_buffer(&mut app, 80, 24);
    assert!(!app.theme.has_surfaces(), "NO_COLOR claims filled bands");
    let answer_row = rows
        .iter()
        .position(|r| r.contains("the answer is B"))
        .unwrap_or_else(|| panic!("the answer never reached the screen: {rows:#?}"));
    let reasoning_row = rows
        .iter()
        .position(|r| r.contains("weighing the two options"))
        .unwrap_or_else(|| panic!("the reasoning never reached the screen: {rows:#?}"));

    // The fallback markers, named rather than merely "different": `pin_theme`
    // pins the Unicode repertoire, so these are the glyphs a real terminal gets.
    //
    // **Read from the repertoire, not typed into the assertion.** This line had `•`
    // written into it, and that literal was the *defect*: `UNICODE.assistant` was `•`,
    // the same character as `UNICODE.bullet`, so on a bandless terminal the model's
    // answer drew as a harness notice. M303 gave the answer its own `▪` and this
    // assertion failed — for the right reason, at the wrong moment, and only because
    // a golden frame test happened to name the character.
    //
    // Asserting against the table states what the test is for: the answer row carries
    // *this repertoire's* answer marker, and the reasoning row its reasoning marker.
    // The collision those two roles once shared is pinned as a property over both
    // repertoires by `no_glyph_set_gives_an_answer_and_a_notice_the_same_character`,
    // so it does not need restating here.
    let glyphs = crate::ui::theme::UNICODE;
    assert!(
        rows[answer_row].starts_with(&format!("{} the answer", glyphs.assistant)),
        "the answer's row must carry its own gutter marker: {:?}",
        rows[answer_row]
    );
    assert!(
        rows[reasoning_row].starts_with(&format!("{} weighing", glyphs.thinking)),
        "the reasoning row must carry its own gutter marker: {:?}",
        rows[reasoning_row]
    );
    assert_ne!(
        cell(&buffer, 0, answer_row as u16).symbol(),
        cell(&buffer, 0, reasoning_row as u16).symbol(),
        "with colour off, the answer and its reasoning are the same pixels"
    );
    // And the same distinction holds on a terminal that *can* show bands — the
    // two roles are separated by the band there, and must still not collide.
    let rich_rows = render_rows(&mut rich, 80, 24);
    let rich_answer = rich_rows
        .iter()
        .position(|r| r.contains("the answer is B"))
        .unwrap_or_else(|| panic!("the answer never reached the screen: {rich_rows:#?}"));
    let rich_reasoning = rich_rows
        .iter()
        .position(|r| r.contains("weighing the two options"))
        .unwrap_or_else(|| panic!("the reasoning never reached the screen: {rich_rows:#?}"));
    assert!(
        rich_rows[rich_answer].starts_with("  the answer"),
        "a banded answer needs no marker: {:?}",
        rich_rows[rich_answer]
    );
    assert!(
        rich_rows[rich_reasoning].starts_with("│ weighing"),
        "the reasoning keeps its rule: {:?}",
        rich_rows[rich_reasoning]
    );
}

#[test]
fn a_pending_tool_call_gets_the_pending_band_where_the_terminal_has_one() {
    // The other half of the same decoration: on a terminal that can show filled
    // bands, the call is on `surface_pending()`. This is the negative control for
    // the `LineKind` part of the change — drop the kind on the way out of the
    // cache and the row loses its band here, while the marker assertions in
    // `render_cache` (which read the cache directly) stay green.
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Truecolor);
    let pending = app.theme.surface_pending().bg;
    let ok = app.theme.surface_ok().bg;
    assert_ne!(pending, ok, "pending and ok must not share a surface");
    app.ui
        .apply(&epsilon_contracts::agent_event::AgentEvent::MessageEnd {
            message: Box::new(Message::Assistant(
                epsilon_contracts::message::AssistantMessage {
                    id: "msg_golden_pending".into(),
                    timestamp_ms: 0,
                    provider: "faux".into(),
                    model_id: "faux-1".into(),
                    content: vec![epsilon_contracts::content::ContentBlock::ToolCall {
                        id: "tc_pending".into(),
                        name: "edit".into(),
                        arguments: serde_json::json!({ "path": "src/lib.rs" }),
                    }],
                    stop_reason: epsilon_contracts::message::StopReason::ToolUse,
                    error_message: None,
                    usage: epsilon_contracts::message::Usage::default(),
                },
            )),
        });

    let (buffer, rows) = render_buffer(&mut app, 80, 24);
    let call_row = rows
        .iter()
        .position(|r| r.contains("edit src/lib.rs · pending"))
        .unwrap_or_else(|| panic!("the tool card never reached the screen: {rows:#?}"));
    assert!(
        rows[call_row].starts_with("▸ edit src/lib.rs · pending"),
        "the card must carry its gutter marker and its title: {:?}",
        rows[call_row]
    );
    // The title carries the card's band; the body nests under a
    // continuation gutter instead. This used to assert both rows banded
    // ("a band that stopped under the title would read as two") — written
    // before bodies had any gutter of their own, when the band was the only
    // thing joining them. The continuation now does that joining, so the
    // band marks state (on the title) rather than painting the whole card.
    for x in [0u16, 2, 40, 79] {
        assert_eq!(
            cell(&buffer, x, call_row as u16).style().bg,
            pending,
            "column {x} of the title row is not on the pending surface: {:?}",
            cell(&buffer, x, call_row as u16).style()
        );
    }
    for x in [0u16, 2, 40, 79] {
        assert_eq!(
            cell(&buffer, x, (call_row + 1) as u16).style().bg,
            Some(ratatui::style::Color::Reset),
            "column {x} of the body row must not carry the band: {:?}",
            cell(&buffer, x, (call_row + 1) as u16).style()
        );
    }
    // Bands belong to the rows that carry them: the blank row below the card is
    // not painted as part of it.
    assert_ne!(
        cell(&buffer, 40, call_row as u16 + 2).style().bg,
        pending,
        "the band leaked onto the row below the card"
    );
}

#[test]
fn the_advertised_scroll_key_scrolls_the_real_frame() {
    // The footer now advertises PageUp/PageDown. A hint is a promise about what
    // a key *does*, so the promise is checked through `handle_key` and the
    // production draw path rather than against the handler's own source: if the
    // footer naming the key were the only change, this fails with the transcript
    // still showing its tail.
    let (mut app, agent, _receivers) = test_app();
    // The chord comes from the binding table, which is where the footer got it.
    let pageup = app
        .keybindings
        .key_text("app.transcript.pageUp")
        .expect("the scroll key must resolve, or the footer is advertising nothing");
    assert_eq!(pageup, "pageup", "the default scroll chord moved");

    for index in 0..60 {
        app.ui.push_notice(format!("ROW-{index:03}"));
    }
    let before = render_rows(&mut app, 60, 20);
    assert!(before.join("\n").contains("ROW-059"), "{before:#?}");
    assert_eq!(app.scroll_from_bottom, 0, "a fresh frame follows the tail");

    assert!(!super::keys::handle_key(
        &mut app,
        &agent,
        KeyEvent {
            code: KeyCode::PageUp,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    ));
    let screen = render_rows(&mut app, 60, 20).join("\n");
    assert!(
        !screen.contains("ROW-059"),
        "the frame still shows the tail after {pageup}: the advertised key did not scroll"
    );
    assert!(screen.contains("ROW-"), "the transcript vanished: {screen}");
}

// ---- D11: the frozen view, on the drawn frame --------------------------------
//
// "Scrolling up is silent, `G` does nothing, and there is no scroll position
// indicator anywhere" was the audit's finding. All three are properties of the
// screen, so all three are read back from the screen: the frozen state is the
// footer row's own glyphs, and the return to live is asserted by which transcript
// rows came back, not by a flag the key handler set.

/// A frame with `count` numbered notices, one per row.
fn numbered_app(count: usize) -> App {
    let mut app = bench_app(UiState::new());
    for index in 0..count {
        app.ui.push_notice(format!("ROW-{index:03}"));
    }
    app
}

/// A key event of the kind crossterm delivers for a real press.
fn press(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    }
}

#[test]
fn paging_up_a_running_view_shows_it_is_frozen_and_where() {
    // 60×12 leaves the transcript eight rows of the ten numbered ones, so one
    // PageUp is enough to lose the tail — which is what the reader has to be
    // able to tell at a glance from the footer.
    let (mut app, agent, _receivers) = test_app();
    // ASCII glyphs on purpose: the badge's marker is a repertoire decision, and
    // the assertion below reads the marker out of the theme rather than writing
    // it down here.
    app.theme = Theme::with_palette(
        ThemeColorMode::Ansi16,
        crate::ui::theme::ASCII,
        crate::ui::theme::ThemeMode::Dark,
    );
    for index in 0..10 {
        app.ui.push_notice(format!("ROW-{index:03}"));
    }
    let before = render_rows(&mut app, 60, 12);
    assert!(before.join("\n").contains("ROW-009"), "{before:#?}");

    assert!(!super::keys::handle_key(
        &mut app,
        &agent,
        press(KeyCode::PageUp)
    ));
    let frozen = render_rows(&mut app, 60, 12);
    let screen = frozen.join("\n");
    let footer = frozen.last().expect("footer row");

    // The state is on the screen: a marker and a count of what is below the
    // window, in the theme's own glyph repertoire.
    let back = crate::ui::theme::ASCII.back;
    assert!(
        footer.contains(back),
        "nothing on the footer says the view stopped following: {footer:?}"
    );
    assert!(
        footer.contains("jump to live"),
        "the footer does not name the way back: {footer:?}"
    );
    // And the position is a count of rows below the window, not a word.
    let below = app.geometry.rows_below();
    assert!(below > 0, "the frozen window is at the bottom: {below}");
    assert!(
        footer.contains(&format!("{back} {below}/")),
        "the badge does not report {below} rows below the window: {footer:?}"
    );
    // The window really moved: the tail is out of view.
    assert!(
        !screen.contains("ROW-009"),
        "the page key did not move the viewport: {screen}"
    );
}

#[test]
fn the_jump_to_live_key_returns_the_view_to_the_tail() {
    // The exit, asserted through the production draw path twice over: the rows
    // that come back are the ones a live view shows, and the frozen marker that
    // was on the footer is gone. A test that read `scroll_from_bottom` would pass
    // on a setter with no draw path behind it — the M123 lesson.
    let (mut app, agent, _receivers) = test_app();
    app.theme = Theme::with_palette(
        ThemeColorMode::Ansi16,
        crate::ui::theme::ASCII,
        crate::ui::theme::ThemeMode::Dark,
    );
    for index in 0..10 {
        app.ui.push_notice(format!("ROW-{index:03}"));
    }
    assert!(!super::keys::handle_key(
        &mut app,
        &agent,
        press(KeyCode::PageUp)
    ));
    let frozen = render_rows(&mut app, 60, 12).join("\n");
    assert!(!frozen.contains("ROW-009"), "{frozen}");

    // The chord comes from the table the footer prints, and the letter is as the
    // key label spells it — `g`, matching `G` because the matcher compares
    // characters case-insensitively.
    let key = app
        .keybindings
        .key_text("app.transcript.live")
        .expect("the jump key must resolve, or the footer is advertising nothing");
    assert_eq!(key, "g", "the default jump chord moved");
    assert!(!super::keys::handle_key(
        &mut app,
        &agent,
        press(KeyCode::Char('g'))
    ));

    let live = render_rows(&mut app, 60, 12);
    let screen = live.join("\n");
    assert!(
        screen.contains("ROW-009"),
        "the view did not come back to the tail after {key}: {screen}"
    );
    let footer = live.last().expect("footer row");
    assert!(
        !footer.contains(crate::ui::theme::ASCII.back),
        "the frozen marker survived the jump: {footer:?}"
    );
    assert!(footer.contains("enter send"), "{footer:?}");
}

#[test]
fn a_live_frame_never_advertises_the_frozen_position() {
    // The negative half: the badge is not a permanent fixture of the row. A
    // footer that always carried a position would be the same defect in the
    // other direction — chrome that says something it does not mean.
    let mut app = numbered_app(40);
    app.theme = Theme::with_palette(
        ThemeColorMode::Ansi16,
        crate::ui::theme::ASCII,
        crate::ui::theme::ThemeMode::Dark,
    );
    let rows = render_rows(&mut app, 60, 20);
    let footer = rows.last().expect("footer row");
    assert!(
        !footer.contains(crate::ui::theme::ASCII.back),
        "a following view reported a position: {footer:?}"
    );
    assert_eq!(app.scroll_from_bottom, 0);
}

// ---- the approval dialog: its size, and the colour it says it uses -----------
//
// `draw_dialog`'s own doc comment claimed a danger treatment while the code
// passed `focused: true` and got the same accent every other overlay uses, and
// the box was eleven rows for a yes/no where the layout reference asks 5–7. Both
// are properties of the drawn frame, so both are read back from the drawn frame:
// a text-only assertion cannot see a border colour, and a test that asked
// `draw_dialog` for its height would be asking the code under test to grade
// itself.

// ---- D8: the light palette, on the drawn frame -------------------------------
//
// The audit ranked "no light theme" as a body-text defect: `Color::White` as
// prose is invisible on a light terminal. The palette tests in `ui::theme` hold
// the arithmetic; these hold the promise that the *frame* changes when the
// palette does, which is the half a token table cannot make on its own.

/// The foreground colour of the first cell of `needle` in a drawn frame.
///
/// Found by scanning the rendered rows rather than assuming a row and column: the
/// question is what colour the reader's eye lands on, and that has to come from
/// the same cells the reader sees. Every needle used with this helper is ASCII,
/// so the row's byte offset is also its column.
///
/// `None` — a cell nothing styled — is reported as [`ratatui::style::Color::Reset`]
/// because every caller compares two *drawn* frames, where the only question is
/// whether the two values differ. A caller asking whether a cell was left
/// untouched has to read `Cell::style().fg` itself, as the `NO_COLOR` loops do:
/// collapsed here, "nothing was emitted" and "`Reset` was emitted" are the same
/// answer, and they are different facts about the code under test.
fn fg_of(buffer: &ratatui::buffer::Buffer, needle: &str) -> ratatui::style::Color {
    let rows = crate::ui::layout::screen_text(buffer);
    for (y, row) in rows.split('\n').enumerate() {
        if let Some(x) = row.find(needle) {
            let cell = &buffer[(x as u16, y as u16)];
            return cell.style().fg.unwrap_or(ratatui::style::Color::Reset);
        }
    }
    panic!("{needle:?} never reached the screen: {rows}");
}

#[test]
fn the_light_palette_changes_the_body_text_the_frame_draws() {
    // One frame, one palette, one asserted colour: a notice is drawn in the
    // system role, which is the quiet-but-readable one the audit found invisible
    // on a light terminal. Both values come out of the palette tables through
    // `Theme`, never from a literal in a widget.
    let prose = "BODYTEXT";
    let draw_with = |mode: crate::ui::theme::ThemeMode| {
        let mut app = bench_app(UiState::new());
        app.theme = Theme::with_palette(ThemeColorMode::Truecolor, crate::ui::theme::UNICODE, mode);
        app.ui.push_notice(prose);
        let (buffer, _rows) = render_buffer(&mut app, 60, 12);
        fg_of(&buffer, prose)
    };
    let light = draw_with(crate::ui::theme::ThemeMode::Light);
    let dark = draw_with(crate::ui::theme::ThemeMode::Dark);
    assert_ne!(
        light, dark,
        "the two palettes drew the same body text: the switch does nothing on screen"
    );
    // The light palette's system role, and the dark one's, exactly: an
    // assertion on "dark-ish" would pass for the *timestamps* and prove nothing
    // about the prose.
    assert_eq!(
        light,
        ratatui::style::Color::Rgb(0x4a, 0x53, 0x64),
        "light body text is not the light palette's system role"
    );
    assert_eq!(
        dark,
        ratatui::style::Color::Rgb(0x8a, 0x92, 0xa3),
        "dark body text is not the dark palette's system role"
    );
    // `Color::White` is what the audit found and what the skill forbids as a
    // default; the light palette must not resolve to it.
    assert_ne!(
        light,
        ratatui::style::Color::White,
        "light body text is still white — invisible on a light terminal"
    );
}

#[test]
fn no_color_keeps_the_roles_apart_in_the_light_palette_too() {
    // `NO_COLOR` is orthogonal to the palette, and the failure it guards against
    // is the one the skill names: a theme that collapses under it makes reasoning
    // traces indistinguishable from answers. Read as *attributes on the drawn
    // cells*, not from the theme object.
    let mut app = bench_app(UiState::new());
    app.theme = Theme::with_palette(
        ThemeColorMode::None,
        crate::ui::theme::UNICODE,
        crate::ui::theme::ThemeMode::Light,
    );
    app.ui.push_local_input("ASKED");
    app.ui.push_notice("NOTICED");
    let (buffer, rows) = render_buffer(&mut app, 60, 12);
    let screen = rows.join("\n");
    assert!(
        screen.contains("ASKED") && screen.contains("NOTICED"),
        "{screen}"
    );

    let asked_row = row_of(&rows, "ASKED");
    let asked_x = rows[asked_row].find("ASKED").expect("found above");
    let asked = cell(&buffer, asked_x as u16, asked_row as u16).style();
    // The contract is "no colour", not "no foreground": the frame is composed of
    // `Style::default()`s and the terminal supplies the rest, so the only
    // assertion that cannot break on an unrelated default is that nothing here
    // names a *hue*.
    assert!(
        matches!(asked.fg, None | Some(ratatui::style::Color::Reset)),
        "NO_COLOR must leave the foreground to the terminal, found {:?}",
        asked.fg
    );

    // Selection is the other role the theme promises to keep visible without
    // colour; it is read as an *attribute*, which is what a no-colour terminal
    // has left.
    let theme = Theme::with_palette(
        ThemeColorMode::None,
        crate::ui::theme::UNICODE,
        crate::ui::theme::ThemeMode::Light,
    );
    assert!(
        theme
            .selected()
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD),
        "selection lost its attribute in the light palette"
    );
    assert!(
        theme
            .thinking()
            .add_modifier
            .contains(ratatui::style::Modifier::DIM),
        "reasoning lost its attribute in the light palette"
    );
}

/// The columns the dialog's frame occupies, across every row it is drawn on.
///
/// Found by looking for the colour the frame is supposed to have. Scanning a
/// single row for "the first styled cell" finds the *transcript* beside the
/// dialog instead — the panel is centred, so the columns around it are not
/// blank, they are the screen it is covering — and the top row is interrupted by
/// the title, which is drawn in the title's style. Taking the extremes over all
/// the frame's rows gives the rectangle's edges whatever the title does.
fn frame_columns(
    buffer: &ratatui::buffer::Buffer,
    rows: &[u16],
    width: u16,
    colour: ratatui::style::Color,
) -> (u16, u16) {
    let mut left = u16::MAX;
    let mut right = 0u16;
    for &row in rows {
        for x in 0..width {
            if buffer[(x, row)].style().fg == Some(colour) {
                left = left.min(x);
                right = right.max(x);
            }
        }
    }
    assert!(
        left != u16::MAX,
        "none of the rows {rows:?} carries the dialog's colour: it is not drawn in it"
    );
    assert!(
        left < right,
        "the frame's edges must be apart: {left}..={right}"
    );
    (left, right)
}

#[tokio::test]
async fn the_approval_dialog_is_six_rows_and_wears_the_danger_border() {
    let (mut app, _agent, _receivers) = test_app();
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    let approvals = ChannelApprovalUi::new();
    let _response = park_ask(&mut app, &approvals, "req-shape").await;

    let (buffer, rows) = render_buffer(&mut app, 100, 24);
    let screen = rows.join("\n");
    let danger = app.theme.danger().fg.expect("the danger role has a colour");
    let focus = app
        .theme
        .border_focus()
        .fg
        .expect("the focus role has a colour");
    assert_ne!(
        danger, focus,
        "the two accents must differ for this test to mean anything"
    );

    // 1. The frame is the danger colour and not the focus accent, on every one
    //    of its edges. This is the assertion the pre-fix dialog failed: it drew
    //    `border_focus()` and claimed otherwise in its doc comment.
    let danger_rows: Vec<u16> = (0..24)
        .filter(|&y| (0..100u16).any(|x| buffer[(x, y)].style().fg == Some(danger)))
        .collect();
    assert_eq!(
        danger_rows.len(),
        6,
        "the permission dialog must be six rows tall — two frame edges and four \
         of body — all of them in the danger accent. Found {danger_rows:?}; one \
         row means the frame is drawn in some other colour and only the `n deny` \
         key is danger, which is what the border_focus() version looked like:\n{screen}"
    );
    assert_eq!(
        danger_rows,
        (danger_rows[0]..danger_rows[0] + 6).collect::<Vec<u16>>(),
        "the dialog's rows must be contiguous: {danger_rows:?}"
    );
    let (left, right) = frame_columns(&buffer, &danger_rows, 100, danger);
    let top = danger_rows[0];
    let bottom = danger_rows[5];
    // The corners are the frame, one at each end of each edge.
    for (row, name) in [(top, "top"), (bottom, "bottom")] {
        for x in [left, right] {
            assert_eq!(
                buffer[(x, row)].style().fg,
                Some(danger),
                "the {name} edge's corner at column {x} is not in the danger accent: {:?}",
                rows[row as usize]
            );
        }
    }
    // The side rails too: a frame is four edges, and a test that read only the
    // top and bottom would pass for a dialog drawn with the wrong sides.
    for row in &danger_rows[1..5] {
        for x in [left, right] {
            assert_eq!(
                buffer[(x, *row)].style().fg,
                Some(danger),
                "column {x} of row {row} must be a danger-coloured rail: {:?}",
                rows[*row as usize]
            );
        }
    }
    // And the rectangle is the width the prompt layout asks for, not a stray
    // band: `Width::PROMPT` is three fifths of the screen, floored at 46 — at 100
    // columns the share wins; the floor is what a narrow terminal keeps.
    assert_eq!(
        right - left + 1,
        100 * 3 / 5,
        "the dialog's width must be the prompt width: {left}..={right}"
    );

    // 2. The body still says everything a decision needs, on those six rows.
    assert!(screen.contains("permission required"), "{screen}");
    assert!(screen.contains("cap"), "{screen}");
    assert!(screen.contains("bash"), "{screen}");
    assert!(
        screen.contains("rm -rf build"),
        "the detail line must show what will run: {screen}"
    );
    assert!(
        !screen.contains(r#"{"command""#),
        "the detail line must be the command, not the JSON object holding it: {screen}"
    );
    for key in [
        "y allow once",
        "a always (session)",
        "n deny",
        "e deny + guidance",
    ] {
        assert!(
            screen.contains(key),
            "the dialog must offer `{key}`: {screen}"
        );
    }
    // 3. Nothing below the dialog is the dialog: the box does not bleed.
    assert!(
        rows[danger_rows[5] as usize + 1..]
            .iter()
            .all(|r| !r.contains("permission required")),
        "the dialog leaked past its bottom edge: {screen}"
    );
}

#[test]
fn a_detail_object_prints_the_field_that_decides_the_question() {
    // The dialog's one row of detail is worth more as the command than as the
    // object containing it: on the PROMPT width the JSON's punctuation is most of
    // what fits.
    assert_eq!(
        summarize_detail(&serde_json::json!({ "command": "rm -rf build" })),
        "rm -rf build"
    );
    assert_eq!(
        summarize_detail(&serde_json::json!({ "path": "src/lib.rs", "offset": 40 })),
        "src/lib.rs"
    );
    assert_eq!(
        summarize_detail(&serde_json::json!({ "query": "openai" })),
        "openai"
    );
    assert_eq!(
        summarize_detail(&serde_json::json!("plain text")),
        "plain text"
    );
    // No known field: the object itself, truncated, rather than a guess.
    let other = summarize_detail(&serde_json::json!({ "count": 3 }));
    assert!(other.contains("count"), "{other}");
    let long = summarize_detail(&serde_json::json!({ "unknown": "x".repeat(400) }));
    assert!(
        long.ends_with('…'),
        "a long detail must be truncated: {long}"
    );
    assert!(long.chars().count() <= 97, "{} chars", long.chars().count());
}

#[test]
fn the_one_rule_sits_between_the_transcript_and_the_composer() {
    // Where the surviving rule lives, on a frame whose transcript is full. The
    // old layout drew this row AND a second rule under the header; the point of
    // cutting one is that this one keeps its job — "everything below this line
    // takes keystrokes" — and the row goes to the conversation.
    let mut app = bench_app(UiState::new());
    for index in 0..40 {
        app.ui.push_notice(format!("ROW-{index:03}"));
    }
    let rows = render_rows(&mut app, 80, 24);
    let rule: char = app
        .theme()
        .glyphs
        .horizontal
        .chars()
        .next()
        .expect("a horizontal glyph");
    let is_rule = |r: &String| !r.is_empty() && r.chars().all(|c| c == rule);
    let rule_rows: Vec<usize> = (0..rows.len()).filter(|&i| is_rule(&rows[i])).collect();
    assert_eq!(
        rule_rows.len(),
        1,
        "there must be one rule, not two: {rows:#?}"
    );
    let rule_row = rule_rows[0];
    // The rule is the line under the composer, and the transcript runs INTO the
    // composer with nothing between them: under the old layout a second rule sat
    // there, at 80x24 on row 19, and this frame's transcript ended above it.
    let composer_row = rule_row - 1;
    assert!(
        rows[composer_row].contains("ask anything"),
        "the one rule must sit directly below the composer, found it on row \
         {rule_row} with {:?} above it: {rows:#?}",
        rows[composer_row]
    );
    assert!(
        rows[composer_row - 1].contains("ROW-"),
        "the transcript must reach the composer with no rule between them: {:?}",
        rows[composer_row - 1]
    );
}

// ---- M126: tool cards and the diff, seen on a real frame ----------------------
//
// The audit's finding was literal: a diff arrived on screen as **prose**. An
// `edit` result was `format!("{name} → diff:\n{diff}")` pushed through the
// markdown parser, so a `-` line became a bullet, a `+` line became a bullet, and
// `---` became a horizontal rule drawn across the card. Nothing about that was
// visible to a test that read the render cache, and a test that called the diff
// renderer directly would have proved only that the renderer worked — the same
// blind spot that hid the transcript's dead decoration for months. Everything
// below drives `draw(app)` and reads the **cells**.

/// One `edit` result with a diff, applied the way a session applies it.
fn edit_app(diff: &str, mode: ThemeColorMode) -> App {
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, mode);
    app.ui
        .apply(&crate::state::agent_event_edit_result(diff, false));
    app
}

/// A diff of `n` removed lines and `n` added ones, longer than the card's cap.
fn long_diff(n: usize) -> String {
    let mut out = format!("@@ -1,{n} +1,{n} @@\n");
    for i in 0..n {
        out.push_str(&format!("-old line {i}\n"));
    }
    for i in 0..n {
        out.push_str(&format!("+new line {i}\n"));
    }
    out
}

/// The row of the frame holding `needle`, or a panic naming what was on screen.
fn row_of(rows: &[String], needle: &str) -> usize {
    rows.iter()
        .position(|r| r.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} never reached the screen: {rows:#?}"))
}

/// The column one of a card body's own characters starts at.
///
/// Found in the rendered row rather than assumed, so these assertions stay about
/// colour instead of about arithmetic that a copy change would silently move.
fn card_text_origin(row: &str, needle: &str) -> usize {
    row.find(needle)
        .unwrap_or_else(|| panic!("{needle:?} is not on this row: {row:?}"))
}

/// The columns of one card body row: where its sign is, and where its text is.
///
/// Both are found from a distinctive word in the row's body rather than from the
/// row's first `+`/`-`: a card row begins with the *card's own* status glyph, and
/// in the ASCII repertoire that glyph is `+`. Searching for the sign from column
/// zero therefore finds the card's marker and reports the number column as the
/// sign. Measuring the body first and walking back to its sign avoids that
/// entirely, and does not depend on how wide the number column happens to be.
///
/// The answer is in **display columns**, not bytes: `✓` is one cell and three
/// bytes, so a byte offset taken after it sits two columns to the right of the
/// cell it names, and `Buffer::cell(x, y)` indexes by cell.
fn sign_and_body(row: &str, body: &str) -> (usize, usize) {
    let body_byte = card_text_origin(row, body);
    let sign_byte = row[..body_byte]
        .rfind(['+', '-'])
        .unwrap_or_else(|| panic!("no sign before {body:?} on this row: {row:?}"));
    (
        crate::ui::layout::display_width(&row[..sign_byte]),
        crate::ui::layout::display_width(&row[..body_byte]),
    )
}

/// Press `ctrl+x`.
///
/// The chord comes from the binding table, and the row under test is the newest
/// card — which is the row `App::selected_card_id` picks and therefore the row the
/// key acts on. `handle_key` is the production entry point, so a test cannot open a
/// card by calling the state mutator directly and then claim the key works.
fn press_expand(app: &mut App, agent: &std::sync::Arc<Agent>) {
    let chord = app
        .keybindings
        .key_text("app.tools.expand")
        .expect("app.tools.expand must resolve");
    assert_eq!(chord, "ctrl+x", "the default expand chord moved");
    assert!(
        !super::keys::handle_key(
            app,
            agent,
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)
        ),
        "`{chord}` must not quit the app"
    );
}

/// The frame test for M155, and the one that should have existed before it.
///
/// Every assertion about a tool call until now was about *state* — `cards.len()`,
/// `card.status`, `card.head`. None of them could see that the call was drawn **twice**,
/// because two cards and one card are equally valid to a state assertion; only the screen
/// shows that one tool call occupies two blocks of rows. Three defects in a row (M152,
/// M153, M155) were invisible to the state tests and obvious here, so this asserts the
/// thing a reader would actually see: the call appears once, and its body is under it
/// rather than beside a copy of it.
/// M157, and the assertion that makes this different from the old `/search`.
///
/// The old command searched the session log and printed its hits as transcript notices, so
/// the honest frame test for "does search work" was the frame test for "are the notices
/// there" — which passed while the thing it was supposed to do, *move the view to the
/// match*, did not exist.
///
/// This asserts the two things a user would notice: the panel says what is being searched
/// and where you are in the matches, and stepping through actually changes which part of
/// the transcript is on screen.
#[test]
fn an_open_search_reports_itself_and_the_matches_it_found() {
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    for text in [
        "alpha",
        "the needle is here",
        "beta",
        "gamma",
        "another needle",
        "delta",
    ] {
        app.ui.push_notice(text);
    }
    app.search_own_transcript("needle");

    let search = app.search.as_ref().expect("a match opens the search");
    assert_eq!(search.matches, vec![1, 4], "both needles, in order");

    let rows = render_rows(&mut app, 80, 24);
    let screen = rows.join("\n");
    assert!(
        screen.contains("search"),
        "the panel must say what it is: {screen}"
    );
    assert!(
        screen.contains("needle"),
        "the query must be on screen: {screen}"
    );
    assert!(
        screen.contains("1 of 2"),
        "the reader must know which match they are on: {screen}"
    );
}

/// The part the old one could not do.
#[test]
fn stepping_through_matches_moves_what_is_on_screen() {
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    for i in 0..80 {
        app.ui.push_notice(format!("row {i}"));
    }
    app.ui.push_notice("findme at the top");
    for i in 80..160 {
        app.ui.push_notice(format!("row {i}"));
    }
    app.search_own_transcript("findme");
    let before = render_rows(&mut app, 80, 24).join("\n");
    assert!(before.contains("1 of 1"), "{before}");

    // A second match far below, so stepping must relocate the viewport.
    app.search_close();
    for i in 160..240 {
        app.ui.push_notice(format!("row {i}"));
    }
    app.ui.push_notice("findme at the bottom");
    app.search_own_transcript("findme");
    let opened = render_rows(&mut app, 80, 24).join("\n");
    // The *first* match specifically. Asserting `contains("findme")` was not enough and
    // the control proved it: with the jump disabled the view sits at the tail, where the
    // **second** match happens to be, so the assertion passed with the feature broken. A
    // test that a control cannot fail is not evidence, which is the same rule as everything
    // else in this file — it just needed saying here too.
    assert!(
        opened.contains("findme at the top"),
        "opening a search must land on its FIRST match, not merely report one: {opened}"
    );
    assert!(
        !opened.contains("row 0"),
        "and it must have *moved the view* to do it — this is the whole point: {opened}"
    );
}

/// Closing puts the view back, so search is not itself the thing you have to undo.
#[test]
fn closing_a_search_restores_the_view_it_started_from() {
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    for i in 0..200 {
        app.ui.push_notice(format!("row {i}"));
    }
    app.scroll_from_bottom = 0;
    app.search_own_transcript("row 12");
    // The jump is *deferred* to the next measurement, because the line-to-row mapping only
    // exists once the render cache has been reconciled for the current transcript. So a frame
    // has to be drawn before there is anything to assert — which is also the real sequence
    // in the app: a key is handled, then the loop draws.
    let _ = render_rows(&mut app, 80, 24);
    let moved = app.scroll_from_bottom;
    assert!(
        moved > 0,
        "the search moved the view, or there is nothing to restore"
    );
    app.search_close();
    assert_eq!(
        app.scroll_from_bottom, 0,
        "closing restores where the reader was"
    );
}

#[test]
fn a_completed_tool_call_is_drawn_once_on_a_real_frame() {
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    // Drive the public event stream, in the order the loop emits it: the call is
    // committed with the assistant message, then it starts, then the result lands.
    app.ui.apply(&AgentEvent::MessageEnd {
        message: Box::new(Message::Assistant(crate::state::assistant_calling(
            "running the check",
            "tc_frame",
            "bash",
            serde_json::json!({"command": "echo epsilon-ready"}),
        ))),
    });
    app.ui.apply(&AgentEvent::ToolExecutionStart {
        tool_call_id: "tc_frame".into(),
        tool_name: "bash".into(),
        args: serde_json::json!({"command": "echo epsilon-ready"}),
    });
    app.ui.apply(&crate::state::agent_event_tool_result_named(
        "tc_frame",
        "bash",
        "epsilon-ready",
        false,
    ));

    let rows = render_rows(&mut app, 80, 24);
    let screen = rows.join("\n");

    // The headline carries the command, the status and the duration (M153).
    let title = row_of(&rows, "bash $ echo epsilon-ready");
    assert!(
        rows[title].contains("· ok ·"),
        "a settled call must say so on its own row: {:?}",
        rows[title]
    );

    // The regression, stated as a count. Before M155 the call card *and* the result
    // card both rendered, so the command appeared twice and the output appeared twice.
    let command_rows = rows
        .iter()
        .filter(|r| r.contains("echo epsilon-ready"))
        .count();
    let output_rows = rows.iter().filter(|r| r.contains("epsilon-ready")).count();
    assert_eq!(
        command_rows, 1,
        "the tool call is drawn once; found it on {command_rows} rows:\n{screen}"
    );
    // `epsilon-ready` is also the command's argument, so the command row matches this
    // pattern too. The output must add exactly one row beyond it — the result body,
    // under its own title, not beside a second copy of the title.
    assert_eq!(
        output_rows, 2,
        "one title row plus one body row, not two of each:\n{screen}"
    );
}

/// A result with no call in the transcript must still reach the screen. The counterpart
/// to M155's in-place branch: if the frame silently dropped an orphaned result, a replay
/// or a branch switch would lose the tool's output with nothing to show for it.
#[test]
fn an_orphaned_tool_result_still_reaches_the_frame() {
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    app.ui.apply(&crate::state::agent_event_tool_result(
        "read",
        "fn main() {}",
        false,
    ));
    let rows = render_rows(&mut app, 80, 24);
    let screen = rows.join("\n");
    assert!(
        screen.contains("fn main() {}"),
        "an orphaned result must not be swallowed:\n{screen}"
    );
}

#[test]
fn a_tool_result_is_a_card_whose_title_carries_name_status_and_duration() {
    // The card contract: `name · status · duration` on the title line, so the
    // reader knows what happened without reading the body. Read from the frame
    // because a title is a property of what was drawn, not of what was built.
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    app.ui.apply(&crate::state::agent_event_tool_result(
        "bash",
        "all tests pass",
        false,
    ));

    let rows = render_rows(&mut app, 80, 24);
    let title_row = row_of(&rows, "bash all tests pass · ok");
    let title = &rows[title_row];
    // Gutter glyph, then the title, which carries the status and duration. One
    // tick, not two: the body marker used to repeat the gutter's own character.
    assert!(
        title.starts_with("✓ bash all tests pass · ok · "),
        "the title line must carry name, status and duration: {title:?}"
    );
    // The duration is a real number with a unit, not a placeholder. A card that
    // printed `0s` for every fast call would look broken, which is why the unit is
    // milliseconds at this scale.
    let tail = title
        .trim_end()
        .trim_start_matches("✓ bash all tests pass · ok · ");
    assert!(
        tail.ends_with("ms"),
        "the duration must be milliseconds for a fast call: {tail:?}"
    );
    assert!(
        tail[..tail.len() - 2].chars().all(|c| c.is_ascii_digit()),
        "the duration must be a number: {tail:?}"
    );
    // The body is the tool's own output, under the title.
    assert!(
        rows[title_row + 1].contains("all tests pass"),
        "the card body must follow its title: {:?}",
        rows[title_row + 1]
    );
}

#[test]
fn a_failed_tool_result_says_failed_on_its_title() {
    // Status is a word as well as a colour: `NO_COLOR` and a 16-colour terminal
    // both drop the colour, and a card that then said nothing about the failure
    // would be the audit's "colour alone carries meaning" defect.
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::None);
    app.ui
        .apply(&crate::state::agent_event_tool_result("bash", "boom", true));

    let rows = render_rows(&mut app, 80, 24);
    let title = &rows[row_of(&rows, "bash boom · failed")];
    assert!(
        title.starts_with("✗ bash boom · failed"),
        "a failed call must say so in words and glyphs: {title:?}"
    );
}

/// A failed shell card's title carries the exit **number** (P-158).
///
/// The frame, not a unit assertion on the renderer: this is the one property of
/// the change that the unit test cannot see — that the number reaches the row a
/// reader actually reads, under the real reducer and the real draw. `bash boom`
/// above already proves the word; this proves the code beside it.
///
/// The event is hand-built rather than taken from `agent_event_tool_result`,
/// because that helper hard-codes `details: {}` and the exit code lives in
/// `details` — exactly the case `state.rs:1684`'s note anticipates. Both widths
/// are drawn: the headline is the longest string a title carries, and a card
/// whose suffix pushed its command off an 80-column screen would pass at 100.
#[test]
fn a_failed_shell_card_shows_its_exit_code_on_the_frame() {
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    app.ui.apply(&AgentEvent::MessageEnd {
        message: Box::new(Message::Assistant(crate::state::assistant_calling(
            "checking the build",
            "tc_exit",
            "bash",
            serde_json::json!({"command": "cargo test"}),
        ))),
    });
    app.ui.apply(&AgentEvent::MessageEnd {
        message: Box::new(Message::ToolResult(
            epsilon_contracts::message::ToolResultMessage {
                id: epsilon_contracts::ids::new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_exit".into(),
                tool_name: "bash".into(),
                content: vec![epsilon_contracts::content::ContentBlock::text(
                    "error: process didn't exit successfully",
                )],
                details: serde_json::json!({ "exit_code": 101 }),
                usage: None,
                is_error: true,
            },
        )),
    });

    for width in [80u16, 100] {
        let rows = render_rows(&mut app, width, 24);
        let needle = "$ cargo test · exit 101";
        let title = &rows[row_of(&rows, needle)];
        assert!(
            title.starts_with("✗ bash $ cargo test · exit 101 · failed"),
            "the title must read as a command that exited 101, at {width}: {title:?}"
        );
        // The command is still on the row — the suffix added a fact, it did not
        // push the command off the line.
        assert!(
            title.contains("$ cargo test · exit 101 · failed · "),
            "status and duration must survive beside the exit code at {width}: {title:?}"
        );
    }
}

/// A `todo` result settles its call's card instead of drawing a second row.
///
/// The composite, on the frame, because each half alone passes while the defect
/// stands: the call card's title must stop saying `pending`, no second row may
/// appear, and the checklist must nest under a continuation gutter. Before the
/// fix the frame carried `▸ todo · pending` above `✓ todo [x] …` with a raw JSON
/// body, on every multi-step turn.
#[test]
fn a_todo_result_settles_its_call_and_nests_its_checklist() {
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    let items = serde_json::json!({"todo": [
        {"content": "refactor the parser", "status": "completed"},
        {"content": "run the tests", "status": "in_progress"},
        {"content": "write the changelog", "status": "pending"}
    ]});
    // The order a real turn emits: the call is committed with the assistant
    // message, it starts, then the result lands.
    app.ui.apply(&AgentEvent::MessageEnd {
        message: Box::new(Message::Assistant(crate::state::assistant_calling(
            "planning the work",
            "tc_todo",
            "todo",
            items.clone(),
        ))),
    });
    app.ui.apply(&AgentEvent::ToolExecutionStart {
        tool_call_id: "tc_todo".into(),
        tool_name: "todo".into(),
        args: items,
    });
    app.ui.apply(&AgentEvent::MessageEnd {
        message: Box::new(Message::ToolResult(
            epsilon_contracts::message::ToolResultMessage {
                id: epsilon_contracts::ids::new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_todo".into(),
                tool_name: "todo".into(),
                content: vec![epsilon_contracts::content::ContentBlock::text(
                    "[x] refactor the parser\n[~] run the tests\n[ ] write the changelog",
                )],
                details: serde_json::json!({"todo": [
                    {"content": "refactor the parser", "status": "completed"},
                    {"content": "run the tests", "status": "in_progress"},
                    {"content": "write the changelog", "status": "pending"}
                ]}),
                usage: None,
                is_error: false,
            },
        )),
    });

    for width in [80u16, 100] {
        let rows = render_rows(&mut app, width, 24);
        let screen = rows.join("\n");
        let todo_rows: Vec<&String> = rows.iter().filter(|r| r.contains("todo")).collect();
        assert_eq!(
            todo_rows.len(),
            1,
            "one row names todo — a second is the double-draw:\n{screen}"
        );
        let title = &rows[row_of(&rows, "✓ todo (1/3 done) · ok · ")];
        assert!(
            title.starts_with("✓ todo (1/3 done) · ok · "),
            "the call's own card is settled, not still pending: {title:?}"
        );
        assert!(
            !screen.contains("pending"),
            "no pending row survives the result:\n{screen}"
        );
        // The checklist nests under a continuation gutter, and no row is raw JSON.
        let item_row = &rows[row_of(&rows, "[x] refactor the parser")];
        assert!(
            item_row.starts_with('│') || item_row.starts_with('|'),
            "the checklist nests under the gutter: {item_row:?}"
        );
        assert!(
            !screen.contains("{\"todo\""),
            "a JSON dump must not reach the screen:\n{screen}"
        );
    }
}

/// A capped **text** body keeps its tail, and says which end is hidden.
///
/// The most-read long card in a coding agent is a build or test log, and its
/// verdict is at the bottom. Pi keeps the last twenty wrapped lines of a bash
/// card (`bash-execution.ts:131`, `.slice(-PREVIEW_LINES)`) and its bash tool
/// preview keeps `keep: "end"` (`renderers/bash.ts:74`), so Pi is deliberately
/// non-uniform: the generic fallback keeps the **first** ten
/// (`tool-execution.ts:166`) and the bash card keeps the **last** twenty.
/// Epsilon has one card, so it takes the bash card's rule — ours *is* Pi's bash
/// card.
///
/// A diff keeps the head: a change reads from its first hunk down, and a diff
/// whose top is hidden cannot be reviewed at all. That is why this is per body
/// kind rather than one rule for the card.
#[test]
fn a_capped_text_body_keeps_its_tail_and_says_so() {
    // 30 lines of compile output whose verdict is the last one.
    let mut body = String::new();
    for i in 0..29 {
        body.push_str(&format!("   Compiling crate-{i} v0.1.0\n"));
    }
    body.push_str("test result: FAILED. 3 passed; 1 failed;");
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    app.ui.apply(&AgentEvent::MessageEnd {
        message: Box::new(Message::Assistant(crate::state::assistant_calling(
            "building",
            "tc_build",
            "bash",
            serde_json::json!({"command": "cargo test"}),
        ))),
    });
    app.ui.apply(&AgentEvent::MessageEnd {
        message: Box::new(Message::ToolResult(
            epsilon_contracts::message::ToolResultMessage {
                id: epsilon_contracts::ids::new_id("msg"),
                timestamp_ms: 0,
                tool_call_id: "tc_build".into(),
                tool_name: "bash".into(),
                content: vec![epsilon_contracts::content::ContentBlock::text(&body)],
                details: serde_json::json!({ "exit_code": 101 }),
                usage: None,
                is_error: true,
            },
        )),
    });

    for width in [80u16, 100] {
        let rows = render_rows(&mut app, width, 40);
        let screen = rows.join("\n");
        assert!(
            screen.contains("test result: FAILED. 3 passed; 1 failed;"),
            "the verdict is the tail and must be on screen at {width}:\n{screen}"
        );
        assert!(
            !screen.contains("Compiling crate-0 "),
            "the head is what the cap hid, and must be gone at {width}:\n{screen}"
        );
        // The affordance must say the head is hidden, not the tail: a reader who
        // is told "N more lines" with no direction will assume the missing ones
        // are below.
        let affordance = rows
            .iter()
            .find(|r| r.contains("lines"))
            .filter(|r| r.contains("earlier") || r.contains("more lines"))
            .unwrap_or_else(|| panic!("the card was not capped at {width}:\n{screen}"));
        assert!(
            affordance.contains("earlier lines"),
            "the affordance must name which end is hidden: {affordance:?}"
        );
    }
}

/// A capped **diff** keeps its head (the tail rule does not apply to it).
///
/// The control for the row above. A diff reads from its first hunk down, so
/// hiding the top would make an unreviewable card, and this is the assertion
/// that stops the tail rule being widened to every body kind by someone who
/// finds the text rule and generalises it.
#[test]
fn a_capped_diff_still_keeps_its_head() {
    let mut app = edit_app(&long_diff(20), ThemeColorMode::Ansi16);
    // Tall enough to show the cap: at 80×24 the viewport itself hides the body,
    // and asserting on a row the window cut off is the mistake the neighbouring
    // diff test documents. `BODY_ROWS` caps the card at 12, so 40 rows is ample.
    let rows = render_rows(&mut app, 100, 40);
    assert!(
        !rows.iter().any(|r| r.contains("new line 19")),
        "the diff head is kept, so its last row is still hidden: {rows:#?}"
    );
    // The first diff line is on screen. The needle is `old line 0` rather than
    // `new line 0` because at this width the diff is SPLIT and both halves carry
    // the same number, so the removed side is the one that cannot be confused
    // with a later row the way `new line 1` can be with `new line 19`.
    assert!(
        rows.iter().any(|r| r.contains("old line 0")),
        "and its first row is on screen: {rows:#?}"
    );
    let affordance = rows
        .iter()
        .find(|r| r.contains("more lines"))
        .expect("the diff is capped");
    assert!(
        !affordance.contains("earlier"),
        "a diff hides its tail, not its head: {affordance:?}"
    );
}

/// Filtering round-trips through the cache without serving stale rows.
///
/// The trap the filter's own design warns about, and the reason the filter is in
/// `RenderState` and `CacheKey` rather than applied at draw time: a row cached
/// under one filter and served under another is the exact failure the cache key
/// exists to prevent.
///
/// **The comparison is on the cached row, not the whole frame.** Every press
/// writes `transcript filter: …`, so the frame's row count changes with the
/// number of presses and two frames taken at different press counts can never be
/// equal. The first two versions of this test asserted `assert_eq!` on frames
/// and on filtered frames, and both failed for that reason alone — test bugs,
/// not the defect they were looking for. The card row is the row whose rendering
/// depends on the filter, so it is the row to compare.
#[test]
fn a_filter_round_trip_through_the_cache_restores_every_row() {
    let (mut app, agent, _receivers) = test_app();
    app.ui.push_local_input("the user's question");
    app.ui.apply(&crate::state::agent_event_tool_result(
        "bash",
        "the tool's output",
        false,
    ));
    let ctrl_o = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
    let shift_ctrl_o = KeyEvent::new(
        KeyCode::Char('O'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );

    /// The rows the card occupies, which is what the filter decides about.
    fn card_rows(app: &mut App) -> Vec<String> {
        render_rows(app, 80, 24)
            .into_iter()
            .filter(|r| r.contains("bash") || r.contains("the tool's output"))
            .collect()
    }

    let original = card_rows(&mut app);
    assert_eq!(original.len(), 2, "title plus one body row: {original:?}");

    // Away: two forwards (no-tools → user-only), then back two (no-tools →
    // default). Every press changes the cache key, so every one invalidates.
    assert!(!super::keys::handle_key(&mut app, &agent, ctrl_o));
    assert!(!super::keys::handle_key(&mut app, &agent, ctrl_o));
    assert!(card_rows(&mut app).is_empty(), "the card is really gone");
    assert!(!super::keys::handle_key(&mut app, &agent, shift_ctrl_o));
    assert!(!super::keys::handle_key(&mut app, &agent, shift_ctrl_o));
    let restored = card_rows(&mut app);

    assert_eq!(
        restored, original,
        "a filter that comes back must bring the card back byte for byte — a \
         stale cache would serve rows rendered under the filter, or lose the \
         ones it dropped"
    );
}

/// `ctrl+o` filters the transcript on a real frame (P-293).
///
/// The whole chain through the real key path: a seeded transcript of user rows,
/// prose and a tool card; one `ctrl+o` press; the frame redrawn. Asserted on the
/// screen because a filter that changed a field and not the frame is the defect
/// class this repository has already been bitten by twice.
///
/// **Why three states and not Pi's five** is written in `TranscriptFilter`: two of
/// Pi's modes need an entry id and a label the live transcript does not carry.
#[test]
fn ctrl_o_filters_the_transcript_on_a_real_frame() {
    let (mut app, agent, _receivers) = test_app();
    app.ui.push_local_input("the user's question");
    app.ui.apply(&AgentEvent::MessageEnd {
        message: Box::new(Message::Assistant(crate::state::assistant_calling(
            "checking",
            "tc_filter",
            "bash",
            serde_json::json!({"command": "echo hi"}),
        ))),
    });
    app.ui.apply(&AgentEvent::ToolExecutionStart {
        tool_call_id: "tc_filter".into(),
        tool_name: "bash".into(),
        args: serde_json::json!({"command": "echo hi"}),
    });
    app.ui.apply(&crate::state::agent_event_tool_result_named(
        "tc_filter",
        "bash",
        "the tool's output",
        false,
    ));
    app.ui
        .push_notice("a notice that is neither prose nor a user turn".to_string());

    let before = render_rows(&mut app, 80, 24).join("\n");
    assert!(before.contains("the tool's output"), "seeded: {before}");
    assert!(before.contains("a notice"), "seeded: {before}");

    // Press 1: no tools. Pi's mode keeps tool **calls** and drops results
    // (`tree-selector.ts:372-375`), and Epsilon's `NoTools` does the same — but
    // the observable difference is smaller than the rule sounds, for a reason
    // worth stating: a call and its result are ONE line here. The result
    // settles the call's own card in place (M155/P-050), so there is no separate
    // call row for the mode to keep. What `no-tools` therefore shows is prose,
    // user turns and notices without any card at all.
    let ctrl_o = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
    assert!(!super::keys::handle_key(&mut app, &agent, ctrl_o));
    let after = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        !after.contains("the tool's output"),
        "no-tools hides tool results: {after}"
    );
    assert!(
        after.contains("the user's question"),
        "and keeps the user's turn: {after}"
    );
    assert!(
        after.contains("a notice"),
        "a notice is not a tool result and must survive: {after}"
    );

    // Press 2: user only. Everything but user rows goes.
    assert!(!super::keys::handle_key(&mut app, &agent, ctrl_o));
    let after = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        after.contains("the user's question"),
        "user-only keeps user rows: {after}"
    );
    assert!(
        !after.contains("a notice"),
        "user-only drops notices: {after}"
    );

    // Press 3: wraps to the start, and the frame is the original again.
    assert!(!super::keys::handle_key(&mut app, &agent, ctrl_o));
    let wrapped = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        wrapped.contains("the tool's output") && wrapped.contains("a notice"),
        "the third press must restore everything, not stop at a dead state: {wrapped}"
    );
}

/// `shift+ctrl+o` steps back, and a filter that is set is not lost by Escape.
///
/// Two halves, both parity: Pi's backward cycle
/// (`tree-selector.ts:1072-1078`) and its rule that Escape clears the search and
/// the folds but **not** the filter mode (`:1039-1046`). A filter that silently
/// reset would make Esc an unpredictable destructive key.
#[test]
fn shift_ctrl_o_steps_back_and_escape_keeps_the_filter() {
    let (mut app, agent, _receivers) = test_app();
    app.ui.push_local_input("the user's question");
    app.ui.apply(&crate::state::agent_event_tool_result(
        "bash",
        "the tool's output",
        false,
    ));

    let ctrl_o = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
    let shift_ctrl_o = KeyEvent::new(
        KeyCode::Char('O'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    // Forward once, back once: the frame must be the unfiltered one.
    assert!(!super::keys::handle_key(&mut app, &agent, ctrl_o));
    let filtered = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        !filtered.contains("the tool's output"),
        "forward filtered: {filtered}"
    );
    assert!(!super::keys::handle_key(&mut app, &agent, shift_ctrl_o));
    let back = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        back.contains("the tool's output"),
        "one press back must restore: {back}"
    );

    // Forward again, then Escape — which is the app's interrupt, and must not
    // quietly change what the transcript shows.
    assert!(!super::keys::handle_key(&mut app, &agent, ctrl_o));
    super::keys::handle_key(
        &mut app,
        &agent,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    );
    let after_esc = render_rows(&mut app, 80, 24).join("\n");
    assert!(
        !after_esc.contains("the tool's output"),
        "Escape must not reset the filter: {after_esc}"
    );
}

#[test]
fn an_edits_diff_is_drawn_with_a_coloured_gutter_not_as_prose() {
    // The heart of the slice. Each of these assertions fails on the old code for a
    // different reason: the colours did not exist (everything was one foreground
    // through the markdown parser), the sign gutter did not exist, and the line
    // numbers did not exist.
    let diff = "@@ -4,6 +4,7 @@\n fn draw() {\n-    let rows = cache.rows();\n+    let rows = cache.rows(width);\n";
    let mut app = edit_app(diff, ThemeColorMode::Ansi16);
    let ok = app.theme.success().fg.expect("a success colour");
    let danger = app.theme.danger().fg.expect("a danger colour");
    let accent = app.theme.accent().fg.expect("a hunk-header colour");

    let (buffer, rows) = render_buffer(&mut app, 80, 24);
    let added_row = row_of(&rows, "let rows = cache.rows(width);");
    let removed_row = row_of(&rows, "let rows = cache.rows();");
    let hunk_row = row_of(&rows, "@@ -4,6 +4,7 @@");
    let context_row = row_of(&rows, "fn draw() {");
    // 1. The sign gutter, on the row it belongs to, before the line number.
    //
    //    Both sides are numbered 5, and that is the format working: the hunk says
    //    `-4,6 +4,7`, so the old file's fifth line is the changed one and the new
    //    file's fifth line is its replacement. The two files stay level until an
    //    addition or a removal makes them diverge — which is what the numbers are
    //    *for*, and why a one-for-one replacement agrees on both sides rather than
    //    showing two different numbers. A diff where the counts do not match its
    //    header (an earlier version of this fixture) is numbered wrongly, which is
    //    why the header is derived here rather than made up.
    assert!(
        rows[added_row].contains("+ 5 "),
        "an added line needs its `+` gutter and the new file's number: {:?}",
        rows[added_row]
    );
    assert!(
        rows[removed_row].contains("- 5 "),
        "a removed line needs its `-` gutter and the old file's number: {:?}",
        rows[removed_row]
    );
    assert!(
        !rows[context_row].contains("+ ") && !rows[context_row].contains("- "),
        "a context line takes neither sign: {:?}",
        rows[context_row]
    );

    // 2. Per-line colour, on the **frame**: the sign and the line are the side's
    //    colour together, which is the skill's rule (a colourblind reader needs
    //    the character to survive; a sighted one needs the whole line marked).
    //
    //    The number column between the sign and the body is drawn faint on
    //    purpose — it is a coordinate, not part of the change — so the columns
    //    asserted are the sign's and the first column of the line itself.
    let (sign_col, text_col) = sign_and_body(&rows[added_row], "let rows");
    let number_col = sign_col + 2;
    assert_eq!(
        (sign_col, text_col),
        (2, 6),
        "the sign is the third cell and the body follows `+ 5 `: {:?}",
        rows[added_row]
    );
    for (column, what) in [(sign_col, "the sign"), (text_col, "the line")] {
        assert_eq!(
            cell(&buffer, column as u16, added_row as u16).style().fg,
            Some(ok),
            "column {column} ({what}) of the added line is not in the success colour: {:?}",
            rows[added_row]
        );
    }
    let (removed_sign, removed_text) = sign_and_body(&rows[removed_row], "let rows");
    for (column, what) in [(removed_sign, "the sign"), (removed_text, "the line")] {
        assert_eq!(
            cell(&buffer, column as u16, removed_row as u16).style().fg,
            Some(danger),
            "column {column} ({what}) of the removed line is not in the danger colour: {:?}",
            rows[removed_row]
        );
    }
    // The number is the metadata column and keeps the faint colour on both sides.
    assert_ne!(
        cell(&buffer, number_col as u16, added_row as u16)
            .style()
            .fg,
        Some(ok),
        "the line number must stay metadata, not take the added line's colour"
    );
    // The context line is neither: it is the dim "unchanged" style, so a diff does
    // not read as two colours fighting down the whole card.
    let context_col = card_text_origin(&rows[context_row], "fn draw");
    assert_ne!(
        cell(&buffer, context_col as u16, context_row as u16)
            .style()
            .fg,
        Some(ok)
    );
    assert_ne!(
        cell(&buffer, context_col as u16, context_row as u16)
            .style()
            .fg,
        Some(danger)
    );
    // The hunk header has its own role, so a reader can find where a hunk starts
    // without reading the numbers.
    let hunk_col = card_text_origin(&rows[hunk_row], "@@");
    assert_eq!(
        cell(&buffer, hunk_col as u16, hunk_row as u16).style().fg,
        Some(accent),
        "the hunk header must be drawn in the accent: {:?}",
        rows[hunk_row]
    );

    // 3. The changed span is marked as well, in inverse video, so a one-word edit
    //    on a long line does not read as a whole new line.
    let reversed_at = |row: usize| {
        (0..80u16)
            .filter(|&x| {
                cell(&buffer, x, row as u16)
                    .style()
                    .add_modifier
                    .contains(ratatui::style::Modifier::REVERSED)
            })
            .count()
    };
    assert!(
        reversed_at(added_row) > 0 && reversed_at(removed_row) > 0,
        "the changed span of a one-for-one edit must be highlighted on both sides"
    );
    // …and only over the part that changed: `rows(width)` is a suffix, so the
    // prefix `let rows = cache.rows` must not be reversed. Counting the reversed
    // cells against the line's own length is what makes that checkable.
    let line_len = "    let rows = cache.rows(width);".chars().count();
    assert!(
        reversed_at(added_row) < line_len,
        "the highlight covered the whole line rather than the change"
    );
}

#[test]
fn the_diff_is_not_run_through_the_markdown_parser() {
    // The defect, stated positively. Marks that only the markdown parser produces:
    // a `-`/`+` line becomes a bullet (`  • `), and `---` becomes a horizontal
    // rule drawn across the card. Neither may appear in a diff card.
    //
    // The file headers here sit *before* the hunk, which is where they really are
    // in a git diff — that is the `DiffKind::Meta` path, not the add/remove path,
    // and the earlier version of this fixture put them inside the hunk by mistake.
    let diff = "--- a/f.rs\n+++ b/f.rs\n@@ -1,3 +1,3 @@\n unchanged\n-old\n+new\n---\n-and another\n+replacement\n";
    let mut app = edit_app(diff, ThemeColorMode::Ansi16);
    let rows = render_rows(&mut app, 80, 24);
    let joined = rows.join("\n");
    // The transcript's exact width is not assumed here: only the rows of *this*
    // card are examined — the title carrying its ✓ gutter and the body rows
    // nested under the continuation gutter (presentation pass: they no longer
    // share one). The welcome screen's bullet is a different role entirely.
    //
    // The card's last row is the `… N more lines` affordance, which carries the
    // `•` of a *system* row because that is what a capped card's tail is. So the
    // slice stops before it: this test is about how a diff's own rows are drawn.
    let title_row = row_of(&rows, "edit edited a.rs");
    let card = rows[title_row..]
        .iter()
        .take_while(|r| {
            (r.starts_with('✓') || r.starts_with('│') || r.starts_with('|'))
                && !r.contains("more lines")
        })
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        card.contains("@@ -1,3 +1,3 @@"),
        "the card's rows were not found: {joined}"
    );
    assert!(
        !card.contains('•'),
        "a diff line became a markdown bullet: {card}"
    );
    // A `---` line becomes a markdown *horizontal rule* — and the rule glyph is
    // the theme's box-drawing character, not an ASCII dash. So the question is
    // whether the card contains any box-drawing line at all; the literal `---`
    // text is the file header being shown as it arrived, which is the point.
    let rule = app.theme.glyphs.horizontal.chars().next().expect("a glyph");
    assert_ne!(rule, '-', "an ASCII repertoire cannot carry this assertion");
    assert!(
        !card.contains(rule),
        "a `---` line was drawn as a markdown rule: {card}"
    );
    assert!(
        card.contains("--- a/f.rs"),
        "the file header must be shown as it arrived: {card}"
    );
    assert!(
        rows.iter().any(|r| r.contains("--- a/f.rs"))
            && rows.iter().any(|r| r.contains("+++ b/f.rs")),
        "the file header lines are part of the diff and must be shown as they came: {joined}"
    );
    // The second `---` is *inside* the hunk and is a removed line, so it takes the
    // sign gutter rather than being mistaken for a file header. Its number is the
    // old file's third line: the context line advanced both files to 2, the `-old`
    // took 2 and advanced the old file to 3.
    assert!(
        rows.iter().any(|r| r.contains("- 3 --")),
        "a `---` line inside a hunk is a removal and must be drawn as one: {joined}"
    );
    // And the hunk line is on its own row rather than joined to its neighbour. The
    // card's rows are padded out to the body width, so the comparison trims.
    // A body row, so the continuation gutter — not the title's ✓.
    assert!(
        rows[row_of(&rows, "@@ -1,3 +1,3 @@")].starts_with("│ @@ -1,3 +1,3 @@")
            || rows[row_of(&rows, "@@ -1,3 +1,3 @@")].starts_with("| @@ -1,3 +1,3 @@"),
        "the hunk header must be its own row, prefixed by the continuation gutter: {:?}",
        rows[row_of(&rows, "@@ -1,3 +1,3 @@")]
    );
}

#[test]
fn a_diff_still_reads_with_no_colour_and_ascii_glyphs() {
    // The property the whole card design rests on: colour reinforces the diff, it
    // never carries it. With `NO_COLOR` **and** the ASCII repertoire — the least
    // capable terminal the product supports — added and removed lines must still
    // be told apart, and told apart by the gutter rather than by position.
    //
    // The theme is pinned to `None` *and* to `ASCII` here, which is the console
    // that `NO_COLOR=1 EPSILON_ASCII=1` gets.
    let diff = "@@ -4,6 +4,7 @@\n fn draw() {\n-    let rows = cache.rows();\n+    let rows = cache.rows(width);\n";
    let mut app = bench_app(UiState::new());
    app.theme = Theme::new(ThemeColorMode::None, crate::ui::theme::ASCII);
    app.ui
        .apply(&crate::state::agent_event_edit_result(diff, false));

    let (buffer, rows) = render_buffer(&mut app, 80, 24);
    let added_row = row_of(&rows, "cache.rows(width)");
    let removed_row = row_of(&rows, "cache.rows();");
    assert_ne!(
        added_row, removed_row,
        "the two sides must be separate rows"
    );

    // The signs, and only the signs, say which side is which. In ASCII the card's
    // own status glyph is `+` too, which is why the body sign is located from the
    // body rather than from column zero.
    let (added_sign, added_text) = sign_and_body(&rows[added_row], "let rows");
    let (removed_sign, removed_text) = sign_and_body(&rows[removed_row], "let rows");
    assert_eq!(
        rows[added_row].chars().nth(added_sign),
        Some('+'),
        "an added line keeps its `+` without colour: {:?}",
        rows[added_row]
    );
    assert_eq!(
        rows[removed_row].chars().nth(removed_sign),
        Some('-'),
        "a removed line keeps its `-` without colour: {:?}",
        rows[removed_row]
    );
    // And the text is still there to read, at the column the sign introduced.
    assert!(added_text > added_sign && removed_text > removed_sign);

    // No colour anywhere on either side — including the changed span's inverse
    // video, which is deliberately not emitted here: on a terminal with no colour
    // a reversed block is just a different-looking background, and the sign already
    // carries the meaning on its own.
    //
    // The assertion cites `None | Some(Reset)` the way the `NO_COLOR` role test
    // above does, and for a reason worth stating: the contract is "no *hue*". The
    // same loop used to demand `Some(Color::Reset)` literally, which is a claim
    // about ratatui as much as about us — `Buffer::empty` already fills every cell
    // with `Reset`, so a cell this code never styled satisfies it. Reading both
    // values is what makes the assertion about the drawing.
    for row in [added_row, removed_row] {
        for x in 0..80u16 {
            let fg = cell(&buffer, x, row as u16).style().fg;
            assert!(
                matches!(fg, None | Some(ratatui::style::Color::Reset)),
                "row {row} column {x} emitted a hue under NO_COLOR: {fg:?}"
            );
        }
    }
    // The line numbers survive with the signs, so a reader whose eye runs down the
    // number column rather than across the signs still sees two different files.
    assert!(
        rows[added_row].contains("+ 5 ") && rows[removed_row].contains("- 5 "),
        "the numbers must survive with the signs: {:?} / {:?}",
        rows[added_row],
        rows[removed_row]
    );
}

#[test]
fn the_split_diff_draws_both_sides_at_a_wide_frame() {
    // B5, and the reason it is a *coverage* blocker rather than a bug: every diff
    // test in this file renders at 80 columns and the cache test at 60, so
    // `split_rows` — reached at `SPLIT_MIN_WIDTH` = 120 body columns (`card.rs`)
    // and live in production at every wider terminal — had never been drawn by
    // anything. The geometry checks out by hand; "I read it and it looks right" is
    // what B1 also looked like.
    //
    // The body width at 140 columns is 138 (the frame minus the 2-cell gutter), so
    // each half is 69. Every assertion below is a **cell**, read from the same
    // buffer the reader would see.
    const FRAME_WIDTH: u16 = 140;
    const BODY_WIDTH: usize = FRAME_WIDTH as usize - 2;
    let diff = "@@ -1,2 +1,2 @@\n unchanged = true;\n-old = 1;\n+new = 2;\n+inserted = 3;\n";
    let mut app = edit_app(diff, ThemeColorMode::Truecolor);
    let (buffer, rows) = render_buffer(&mut app, FRAME_WIDTH, 24);
    let ok = app.theme.success().fg.expect("a success colour");
    let danger = app.theme.danger().fg.expect("a danger colour");
    let faint = app.theme.faint().fg.expect("a faint colour");

    let unchanged = row_of(&rows, "unchanged = true;");
    let replaced = row_of(&rows, "old = 1;");
    let added = row_of(&rows, "new = 2;");
    let inserted = row_of(&rows, "inserted = 3;");

    // 1. The row is one row, not two: a split diff exists to show a replacement
    //    *beside* itself, and a parser or renderer that put the halves on
    //    consecutive rows would satisfy every 80-column test in this file.
    assert_eq!(
        replaced, added,
        "a one-for-one replacement must share one row: {:?} then {:?}",
        rows[replaced], rows[added]
    );
    assert!(
        rows[replaced].contains("old = 1;") && rows[replaced].contains("new = 2;"),
        "both sides of the change must be on that row: {:?}",
        rows[replaced]
    );

    // 2. Both halves' positions and both halves' sign columns. The *absolute*
    //    column is not asserted where it would be a fact about chrome — every
    //    column below comes from a word in the row, via the buffer — but the
    //    relation between the halves is the half-width arithmetic this coverage gap
    //    was about, so that is asserted exactly.
    //
    //    Each half is `- 2 ` then the body: the sign is three cells before its
    //    body, the number one cell after the sign.
    let locate = |needle: &str| {
        column_of(&buffer, replaced as u16, FRAME_WIDTH, needle)
            .unwrap_or_else(|| panic!("{needle:?} never reached the frame: {:?}", rows[replaced]))
    };
    let (left_body, right_body) = (locate("old = 1;"), locate("new = 2;"));
    let (left_sign, right_sign) = (left_body - 4, right_body - 4);
    assert_eq!(
        cell(&buffer, left_sign as u16, replaced as u16).symbol(),
        "-",
        "the left half must be signed as a removal, with its number after it: {:?}",
        rows[replaced]
    );
    assert!(
        cell(&buffer, right_sign as u16, replaced as u16)
            .symbol()
            .starts_with('+'),
        "the right half must be signed as an addition: {:?}",
        rows[replaced]
    );
    // The right half of a split diff is a *column* of the same row, half the body
    // away from the left one, and the card's own sign/number chrome (`- 2 `) is the
    // same four cells inside each half. An off-by-one in `left_width = width / 2` /
    // `width - left_width` shows up here and nowhere in the 80-column tests, which
    // never take this branch.
    let half = BODY_WIDTH / 2;
    assert_eq!(
        right_body,
        left_body + half,
        "the right half's body must begin exactly {half} columns after the left \
         half's — and its sign column the same distance further along: signs at \
         {left_sign}/{right_sign} on {:?}",
        rows[replaced]
    );
    assert_eq!(
        right_sign,
        left_sign + half,
        "…which is the same statement about the signs: {:?}",
        rows[replaced]
    );
    // The numbers, as cells: 2 on both sides — the two files are level at this
    // point, which is the property `state.rs::parse_diff`'s docs describe.
    for (column, side) in [(left_sign + 2, "removed"), (right_sign + 2, "added")] {
        assert!(
            cell(&buffer, column as u16, replaced as u16)
                .symbol()
                .starts_with('2'),
            "the {side} line is its file's second: {:?}",
            rows[replaced]
        );
    }

    // 3. Per-side colour, on the frame, both halves: the success colour on the
    //    right and the danger colour on the left, at the sign *and* at the body.
    //    A side that inherited the other's colour would still read correctly in
    //    text and wrongly in every terminal that has colour at all.
    for (column, what) in [
        (left_sign, "the removed sign"),
        (left_body, "the removed line"),
    ] {
        assert_eq!(
            cell(&buffer, column as u16, replaced as u16).style().fg,
            Some(danger),
            "column {column} ({what}) is not in the danger colour: {:?}",
            rows[replaced]
        );
    }
    for (column, what) in [
        (right_sign, "the added sign"),
        (right_body, "the added line"),
    ] {
        assert_eq!(
            cell(&buffer, column as u16, replaced as u16).style().fg,
            Some(ok),
            "column {column} ({what}) is not in the success colour: {:?}",
            rows[replaced]
        );
    }

    // 4. The blank half, which is what a split diff is *for*: a line with no
    //    counterpart leaves its side empty rather than absent, so the columns stay
    //    aligned and "nothing here" is said rather than implied. The added-only
    //    line is the case: its right half is signed, and its left half is blank, in
    //    the faint role, with no change colour anywhere in its columns.
    let inserted_sign = column_of(&buffer, inserted as u16, FRAME_WIDTH, "+ 3")
        .unwrap_or_else(|| panic!("the added-only line has no sign: {:?}", rows[inserted]));
    assert_eq!(
        inserted_sign, right_sign,
        "the added-only line's sign belongs in the right half's sign column: {:?}",
        rows[inserted]
    );
    assert_eq!(
        inserted_sign, right_sign,
        "the added-only line's sign belongs in the right half's sign column: {:?}",
        rows[inserted]
    );
    // The left half owns `[left_sign, left_sign + half)` — a half starting at its
    // own body's sign column, the same origin the two halves are measured from.
    let reversed: String = row_cells(&buffer, inserted as u16, FRAME_WIDTH)
        .into_iter()
        .filter(|(column, _)| *column >= left_sign && *column < left_sign + half)
        .map(|(_, ch)| ch)
        .collect();
    assert!(
        reversed.trim().is_empty(),
        "the added-only line's left half must be blank — that blank half is what \
         says 'this line has no counterpart': {:?}",
        rows[inserted]
    );
    // Every cell that half owns, read from the buffer: the missing side must be
    // drawn in the faint role and never in either change colour. The sign column is
    // included on purpose — a blank half that kept the removal's red would be the
    // worst of both readings.
    for column in left_sign..left_sign + half {
        let fg = cell(&buffer, column as u16, inserted as u16).style().fg;
        assert_ne!(
            fg,
            Some(ok),
            "column {column} of the blank half borrowed the added line's colour: {:?}",
            rows[inserted]
        );
        assert_ne!(
            fg,
            Some(danger),
            "column {column} of the blank half borrowed a removal colour: {:?}",
            rows[inserted]
        );
        assert!(
            fg == Some(faint) || fg.is_none(),
            "column {column} of the blank half is {fg:?}, not the faint role: {:?}",
            rows[inserted]
        );
    }

    // 5. Context lines are the same text on both sides, so a reader can trust that
    //    the two halves show the same file at the same place — and a context line
    //    takes neither sign, exactly as in the unified view.
    let occurrences: Vec<usize> = row_cells(&buffer, unchanged as u16, FRAME_WIDTH)
        .windows("unchanged = true;".len())
        .filter(|window| {
            window
                .iter()
                .map(|(_, ch)| *ch)
                .eq("unchanged = true;".chars())
        })
        .map(|window| window[0].0)
        .collect();
    assert_eq!(
        occurrences.len(),
        2,
        "a context line is drawn once per side, and a split diff shows both: {:?}",
        rows[unchanged]
    );
    assert_eq!(
        occurrences[0], left_body,
        "the left half shows the context line where the removed line sits: {:?}",
        rows[unchanged]
    );
    assert_eq!(
        occurrences[1],
        occurrences[0] + half,
        "and the right half shows it exactly one half-width later: {:?}",
        rows[unchanged]
    );
    for column in occurrences {
        for sign_column in [column - 4, column - 3] {
            let symbol = cell(&buffer, sign_column as u16, unchanged as u16).symbol();
            assert!(
                symbol.trim().is_empty(),
                "a context line takes neither sign, found {symbol:?} at column \
                 {sign_column}: {:?}",
                rows[unchanged]
            );
        }
    }

    // 6. And every row fits the frame, which is the property this whole file
    //    exists for and the one a padding error would break: the split path pads
    //    each half to its own width (`pad_to`), so `left + right` has to be exactly
    //    the body width rather than approximately it.
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(
            crate::ui::layout::display_width(row),
            FRAME_WIDTH as usize,
            "row {index} is {} cells wide, not the frame's {FRAME_WIDTH}: {row:?}",
            crate::ui::layout::display_width(row)
        );
    }
    // The frame really is in split mode — a 138-column body against a 120-column
    // threshold — and the unified fallback would have put the halves on separate
    // rows, which the `replaced == added` assertion above would already have
    // caught. A *compile-time* assertion rather than a run-time one: both values are
    // constants, so it either holds for every build or for none, and a lowered
    // `SPLIT_MIN_WIDTH` should fail the build rather than one test. Written this way
    // after clippy rejected the run-time form (`clippy::assertions_on_constants`) —
    // which was right, and the compile-time form is the stronger guard too.
    const _: () = assert!(
        BODY_WIDTH >= crate::ui::card::SPLIT_MIN_WIDTH,
        "the split-diff test's frame must be in split mode"
    );
}

/// Every character of a drawn row, in order, with its **cell column**.
///
/// Read from the buffer rather than from the row's text, and that distinction cost
/// a debugging round: ratatui coalesces a run of identical adjacent cells into one
/// cell whose symbol repeats (`Cell::symbol` is documented to hold one or more
/// graphemes, and `layout::row_text` warns about exactly this), so a row holding
/// 106 spaces is *142 characters* and 140 columns. Counting characters, or
/// computing a column from a byte offset into the printed row, therefore reports a
/// column up to 60 cells away from the one the reader sees — which is what made the
/// first version of the split test measure the right half starting 13 columns after
/// the left instead of 69.
///
/// This is the only sound way to ask where something is on a split row, because the
/// two halves are separated by exactly such a padding run.
///
/// Not a general-purpose helper: a wide glyph contributes its leading cell's column
/// once, so callers ask for ASCII needles, which is what every needle here is.
fn row_cells(buffer: &ratatui::buffer::Buffer, row: u16, width: u16) -> Vec<(usize, char)> {
    let mut out = Vec::new();
    let mut column = 0usize;
    for x in 0..width {
        let symbol = buffer[(x, row)].symbol();
        for ch in symbol.chars() {
            out.push((column, ch));
        }
        column += crate::ui::layout::display_width(symbol).max(1);
    }
    out
}

/// The cell column where `needle` starts on a drawn row.
///
/// The needle is located in the concatenated symbols — which is what `contains`
/// does against the printed row — and the answer is the column of the first cell
/// the match starts in, which is the correction the printed row cannot give.
fn column_of(
    buffer: &ratatui::buffer::Buffer,
    row: u16,
    width: u16,
    needle: &str,
) -> Option<usize> {
    let cells = row_cells(buffer, row, width);
    let text: String = cells.iter().map(|(_, ch)| *ch).collect();
    let byte = text.find(needle)?;
    cells
        .get(text[..byte].chars().count())
        .map(|(column, _)| *column)
}

#[test]
fn a_long_diff_is_capped_and_says_which_key_opens_it() {
    // The cap, and the honesty of the affordance: it names the **real** key, and
    // the number it prints is the number of rows the reader is not seeing.
    let mut app = edit_app(&long_diff(20), ThemeColorMode::Ansi16);
    let key = app
        .keybindings
        .key_text("app.tools.expand")
        .expect("app.tools.expand must resolve, or the card names a dead key");
    assert_eq!(key, "ctrl+x", "the default expand chord moved");

    let rows = render_rows(&mut app, 80, 24);
    let affordance = rows
        .iter()
        .find(|r| r.contains("more lines"))
        .unwrap_or_else(|| panic!("the card was not capped: {rows:#?}"));
    assert!(
        affordance.contains(&format!("({key} to open)")),
        "the affordance must name the key that opens the card: {affordance:?}"
    );
    // The count is the rows the card is hiding: 1 hunk + 20 removed + 20 added =
    // 41 body rows, of which `BODY_ROWS` are shown and the affordance line is one
    // more. Pinned to the constant rather than to a literal, so moving the cap
    // moves this assertion with it.
    let body_rows = 41;
    let hidden = body_rows - crate::ui::card::BODY_ROWS;
    assert!(
        affordance.contains(&format!("… {hidden} more lines")),
        "the affordance must print the true number of hidden rows ({hidden}): {affordance:?}"
    );
    // The cap really caps: no `+new line 19` on screen.
    assert!(
        !rows.iter().any(|r| r.contains("new line 19")),
        "the card drew its whole 41-row body: {rows:#?}"
    );
    // And the affordance names a key that does what it says. The frame here is
    // tall enough for the whole opened card: a card that opens *below* the window
    // is indistinguishable from one that did not open, and the mistake this pins
    // is the test's own (asserting on a row the viewport cuts off).
    let (mut app2, agent, _receivers) = test_app();
    app2.ui.apply(&crate::state::agent_event_edit_result(
        &long_diff(20),
        false,
    ));
    let before = render_rows(&mut app2, 100, 60)
        .iter()
        .any(|r| r.contains("new line 19"));
    assert!(!before, "the frame starts capped");
    assert!(!super::keys::handle_key(
        &mut app2,
        &agent,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)
    ));
    let opened = render_rows(&mut app2, 100, 60)
        .iter()
        .any(|r| r.contains("new line 19"));
    assert!(
        opened,
        "`{key}` did not open the card its own affordance advertised"
    );
}

#[test]
fn ctrl_x_opens_the_newest_card_and_leaves_the_others_alone() {
    // Per-card expansion, on a real frame. Under the old global boolean, opening
    // one card opened every card — the audit's "a user cannot open one card
    // without opening every card". The other cards here carry a marker of their
    // own, so "the others were left alone" is checkable rather than assumed.
    //
    // What the key does, stated once and pinned here: it flips the **global**
    // expand-all toggle (the binding's meaning since M13, kept because a user
    // relies on it) *and* records the newest card's own state, so the card the
    // reader is looking at always changes. The per-card entries are what make the
    // feature possible at all — a card the user opens while the global toggle is
    // off stays open, and one they close while it is on stays closed.
    let (mut app, agent, _receivers) = test_app();
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    // Two short cards, each with a body of its own …
    for i in 0..2 {
        app.ui.apply(&crate::state::agent_event_edit_result(
            &format!("@@ -1,1 +1,1 @@\n-old one {i}\n+new one {i}\n"),
            false,
        ));
    }
    // … and one long card, newest, whose body the cap actually hides. The card
    // count is small on purpose: an opened card must stay inside the window, or
    // "it did not open" and "the viewport cut it off" look the same.
    let long = format!(
        "@@ -1,1 +1,1 @@\n-old visible\n+new visible\n{}",
        filler(20)
    );
    app.ui
        .apply(&crate::state::agent_event_edit_result(&long, false));

    let rows = render_rows(&mut app, 80, 44);
    assert!(
        rows.iter().any(|r| r.contains("more lines")),
        "the long card must be capped before the key is pressed: {rows:#?}"
    );
    assert!(
        !rows.iter().any(|r| r.contains("filler 19")),
        "the capped card must not already be showing its hidden rows: {rows:#?}"
    );
    assert_eq!(
        app.card_open.len(),
        0,
        "nothing has been opened by hand yet"
    );

    // One keystroke, on the newest card.
    let before_press = (
        app.prefs.expand_outputs,
        app.selected_card_id(),
        app.card_open.clone(),
    );
    press_expand(&mut app, &agent);
    assert_eq!(
        (app.prefs.expand_outputs, app.card_open.clone()),
        (true, vec![(before_press.1.expect("a card"), true)]),
        "before the press the app was {before_press:?}; after it, expand_outputs \
         and the recorded card must both have moved"
    );
    // 1. The global toggle still exists and still flips — the user who relies on
    //    "expand everything" has not had it taken away.
    assert!(
        app.prefs.expand_outputs,
        "the global expand-all toggle must still be wired to the key"
    );
    // 2. The selected card is open, and the frame proves it: its hidden rows are
    //    on screen.
    let rows = render_rows(&mut app, 80, 44);
    assert!(
        rows.iter().any(|r| r.contains("filler 19")),
        "the card the key acted on did not open (card_open={:?}, total_rows={}): {rows:#?}",
        app.card_open,
        app.render_cache.total_rows()
    );
    assert!(
        !rows.iter().any(|r| r.contains("more lines")),
        "an open card still claims rows are hidden: {rows:#?}"
    );
    // 3. Exactly one card's state was recorded by hand, and it is the newest: the
    //    other cards were not touched through this path at all. (They follow the
    //    global toggle, which is what "expand all" has always meant.)
    assert_eq!(
        app.card_open.len(),
        1,
        "one press must record one card's own state: {:?}",
        app.card_open
    );
    let newest = app.selected_card_id().expect("the newest card");
    assert_eq!(
        app.card_open[0].0, newest,
        "the card recorded must be the one the affordance pointed at"
    );
    // 4. The other cards' own bodies are still theirs alone: the short cards are
    //    on screen with their own rows, and none of them acquired the long card's.
    assert!(
        rows.iter().any(|r| r.contains("new one 1")),
        "the short cards must still be on screen: {rows:#?}"
    );
    assert!(
        !rows
            .iter()
            .any(|r| r.contains("filler") && r.contains("new one")),
        "a card's body leaked into another's: {rows:#?}"
    );

    // 5. The case that makes per-card state *necessary* rather than decorative:
    //    press again. The global toggle goes back off, so the cards nobody has
    //    touched collapse — while the one the user acted on stays open, because
    //    its own decision outranks the global default. One global boolean cannot
    //    express that, and this is the assertion that fails when
    //    `RenderState::is_open` ignores `explicit`: with the global off and the
    //    per-card entry ignored, the long card is capped again and the two short
    //    cards reopen, so the row count lands back where the first press left it
    //    rather than two rows below.
    let all_open_rows = app.render_cache.total_rows();
    press_expand(&mut app, &agent);
    assert!(
        !app.prefs.expand_outputs,
        "the second press must put the global toggle back off"
    );
    let rows = render_rows(&mut app, 80, 44);
    assert!(
        rows.iter().any(|r| r.contains("filler 19")),
        "the hand-opened card must still be open with the global toggle off: {rows:#?}"
    );
    assert!(
        !rows.iter().any(|r| r.contains("more lines")),
        "the hand-opened card must not be capped again: {rows:#?}"
    );
    // The two short cards each hold a three-line diff (`@@`, `-`, `+`) against a
    // twelve-row cap, so **nothing is hidden and capping them gives back nothing**:
    // open and capped render the same rows, and the total cannot move when the
    // global goes off. This assertion used to expect `all_open_rows - 4`, on the
    // belief that those cards were four rows capped and six open; a probe of the
    // real frame showed the total unchanged (32) with `expand_all=false` and one
    // explicit entry, which is this arithmetic and not a rendering fault.
    //
    // So the honest assertion is that nothing moved — and the observable evidence
    // that the global *did* take effect is above: the long card is still open
    // (which only an explicit entry can do once the global is off) while its
    // `more lines` affordance is gone. A card that could not show the difference
    // cannot be used to prove it.
    assert_eq!(
        app.render_cache.total_rows(),
        all_open_rows,
        "the short cards hide nothing under the cap, so nothing may move"
    );
}

/// Extra diff body lines, so a card is long enough to hit the cap.
///
/// Kept beside the fixtures it describes and *used* by them: it was already here
/// under an `#[allow(dead_code)]` that said as much while the one test that wanted
/// it built the same twenty lines inline with a loop of its own. A helper nothing
/// calls is a fixture nobody reads — and the `allow` is what let the two drift
/// apart without a warning.
fn filler(n: usize) -> String {
    (0..n).map(|i| format!("+filler {i}\n")).collect::<String>()
}

/// The frame-level half of the architecture M152 recorded as a contradiction.
///
/// `only_a_finished_card_carries_a_duration` (`state.rs`) already proves `elapsed_ms()` is
/// **A panel row never carries a display-steering character into its text.**
///
/// The `@` popup shows workspace paths, the session browser shows a conversation's **title**,
/// and the model picker's stage 2 shows ids from a **live listing** — text from a third-party
/// endpoint, which is the only place in the picker where the bytes are not ours. All three were
/// drawn as written.
///
/// The distinction that decides the fix is M261's: the permission dialog **refuses** an ask
/// carrying one, because there a reader is making a decision; these panels only *display*, and
/// what they hand back comes from the row's own field rather than the drawn string. So the
/// right response is to remove the character, not to refuse the row.
#[test]
fn a_panel_row_is_stripped_before_it_is_drawn() {
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Ansi16);

    // A live listing id with an override after the first word, so the strip is visible in the
    // row's *text* and not only in the absence of a character.
    app.picker_open = true;
    app.picker_provider = Some("faux".into());
    app.live_cache = vec![(
        "faux".to_string(),
        vec!["claude\u{202E}-3-opus".to_string()],
    )];

    let rows = render_rows(&mut app, 100, 30);
    let joined = rows.join("\n");
    // **The row survives the strip** — asserted first, because a row that vanished would
    // satisfy a "no override" assertion for entirely the wrong reason. That mistake is what
    // the first version of this test made.
    assert!(joined.contains("claude"), "the row must render.\n{joined}");
    assert!(
        !joined.contains('\u{202E}'),
        "an override reached a live model listing.\n{joined}"
    );
}

/// Stage-1 provider rows carry an auth badge and an active mark (P-313).
///
/// The matrix row claims provider rows carry `• not configured`, `✓ configured`,
/// `✓ env: NAME` or a type-mismatch warning, and names two tests that never look
/// at a provider row — so the row was `done` on evidence that does not establish
/// it, and on literals this window never draws. What it actually draws is the
/// same distinction in its own visual language: a key glyph for a configured
/// provider, `·` for one that is not, `●` on the active one
/// (`panels.rs:112-131`). This test pins that half through the production draw
/// path. It does not establish the `env: NAME` display or the type-mismatch
/// warning — neither exists in the tree — and the row says so.
#[test]
fn stage_one_provider_rows_carry_an_auth_badge_and_an_active_mark() {
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    // Stage 1: the picker open on providers, no filter.
    app.picker_open = true;
    app.picker_provider = None;

    // Aim the window at the faux provider: it sorts last, past the panel's
    // visible slice at this size, and it is the deterministic half of this
    // test — it needs no credential by construction
    // (`row_credentials_ready`: an empty env_key), so its badge is on no
    // matter what this machine holds.
    let providers = crate::picker::build_provider_rows(&app.model_id, &app.customs);
    let faux_at = providers
        .iter()
        .position(|p| p.id == "faux")
        .expect("the catalog holds a faux provider");
    app.picker_index = faux_at;

    let rows = render_rows(&mut app, 100, 30);
    let joined = rows.join("\n");

    // The anchor: at least one rendered provider row, or the loop below is vacuous.
    let provider_lines: Vec<&String> = rows
        .iter()
        .filter(|r| r.contains("models") && providers.iter().any(|p| r.contains(&p.id as &str)))
        .collect();
    assert!(
        !provider_lines.is_empty(),
        "no provider row rendered at all.\n{joined}"
    );

    let faux: Vec<&&String> = provider_lines
        .iter()
        .filter(|r| r.contains("faux"))
        .collect();
    assert!(
        !faux.is_empty(),
        "the faux provider row must render.\n{joined}"
    );
    for line in faux {
        assert!(
            line.contains("🔑"),
            "a provider that needs no credential carries the key badge.\n{line}\n{joined}"
        );
        assert!(
            line.contains("●"),
            "the test app runs faux-1, so faux is the active provider.\n{line}\n{joined}"
        );
    }

    // Structure: every rendered provider row says something in the badge
    // column. A row that said neither would read as "unknown", which is the
    // state the badge exists to prevent.
    for line in provider_lines {
        assert!(
            line.contains("🔑") || line.contains("·"),
            "a provider row with no auth badge.\n{line}\n{joined}"
        );
    }
}

/// The auth picker names nothing when no credential exists anywhere (P-313).
///
/// The positive half — "key (env: NAME)" — is pinned at the precedence helper
/// (`env_credential_name_pins_the_display_precedence`); this pins the draw half
/// a frame can see without touching the machine: a custom provider on a
/// variable no other test uses, unset, renders its row with "—" and no
/// "(env:". Setting process env is unavailable (edition 2024 made `set_var`
/// unsafe; the workspace forbids it), so a frame showing the suffix pixels
/// cannot exist here — stated, not assumed.
#[test]
fn the_auth_picker_shows_no_source_for_an_unconfigured_provider() {
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    // A custom provider on a variable no other test uses. If the process ever
    // holds it set, the row below names it — which is the feature working, and
    // the assertion names the variable so the failure reads as that.
    app.customs.push(
        epsilon_llm::custom::CustomProvider::from_config(
            "testcred",
            "http://localhost:9/v1",
            "EPSILON_TEST_CRED_SRC",
        )
        .expect("custom provider builds"),
    );
    app.auth_pick_open = true;
    // Aim the window at the custom row: it sorts past the panel's visible
    // slice, and an assertion on a row that never drew would pass for the
    // wrong reason (the P-313 test aims the same way).
    // NOTE (read before "fixing"): this machine holds a DeepSeek credential,
    // so the deepseek row reads "key" here. That is the room, not the test —
    // the assertions below touch only the test's own row.
    let providers = crate::picker::build_provider_rows(&app.model_id, &app.customs);
    app.auth_pick_index = providers
        .iter()
        .position(|p| p.id == "testcred")
        .expect("the custom provider is listed");

    let rows = render_rows(&mut app, 100, 30);
    let joined = rows.join("\n");
    let own: Vec<&String> = rows.iter().filter(|r| r.contains("testcred")).collect();
    assert!(
        !own.is_empty(),
        "the custom provider row must render.\n{joined}"
    );
    for line in &own {
        assert!(
            line.contains("—"),
            "an unconfigured provider reads unconfigured, in words.\n{line}\n{joined}"
        );
        assert!(
            !line.contains("(env:"),
            "an unset variable must not be named.\n{line}\n{joined}"
        );
        assert!(
            !line.contains("! oauth login unused"),
            "no stored login, no warning: the warning must never fire on an empty store.\n{line}\n{joined}"
        );
    }
}

/// **The frame cannot see a display-steering character at all, and that is why this strip is
/// asserted on text rather than on pixels.**
///
/// The test above was first written as a control — `safe_to_show` stubbed to a no-op — and it
/// **still passed**. That is not a weak control, it is an instrument that cannot see the thing:
/// `row_text` reads ratatui's **cell symbols**, and `U+202E` never becomes one. The buffer
/// does not carry it, so the terminal does not receive it, and no frame assertion can tell a
/// stripped row from an unstripped one.
///
/// Two consequences, both worth having written down:
///
/// * **A pass here is weak evidence**, which is why the assertion is on the *text* of the row
///   and why the helper also has a direct test. M269's lesson in a new place: an instrument
///   that cannot see a defect makes both fixing and *disproving* one impossible.
/// * **`text_safety`'s own premise is weaker than its doc claims.** The module doc says
///   ratatui "passes it through"; on this ratatui version it evidently does not, at least for
///   the buffer. The strip is still right — defence in depth, and a different backend, width
///   or a future ratatui need not behave the same — but the *reason* to keep it is not "the
///   bytes reach the screen", it is "we do not depend on that being true".
///
/// This test therefore **documents the limit** rather than proving the fix.
#[test]
fn a_frame_cannot_see_a_display_steering_character() {
    use ratatui::widgets::Paragraph;

    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    let (buffer, _rows) = render_buffer(&mut app, 60, 8);
    // Draw a hostile string straight through ratatui, with nothing of ours in between.
    let backend = ratatui::backend::TestBackend::new(60, 8);
    let mut terminal = ratatui::Terminal::new(backend).expect("a terminal");
    terminal
        .draw(|f| {
            f.render_widget(Paragraph::new("claude\u{202E}-3-opus"), f.area());
        })
        .expect("a draw");
    let drawn: String = (0..8)
        .map(|y| super::tests_support::row_text(terminal.backend().buffer(), y, 60))
        .collect();
    assert!(
        !drawn.contains('\u{202E}'),
        "if this ever starts failing, ratatui began passing the character through and every \
         frame-level assertion about it becomes meaningful again -- revisit this note.\n{drawn}"
    );
    assert!(
        drawn.contains("claude"),
        "and the surrounding text must still be drawn, or the check is vacuous.\n{drawn}"
    );
    let _ = buffer;
}

/// `None` while a call is pending. That is a fact about state, and it says nothing about what
/// the reader sees. What is unproven is the pair this milestone claims: the **header** carries a
/// live figure for a running call, and the **card row** carries none.
#[test]
fn a_running_call_shows_its_time_in_the_header_not_on_the_card() {
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    // The public event stream, stopped one step short of the result.
    app.ui.apply(&AgentEvent::MessageEnd {
        message: Box::new(Message::Assistant(crate::state::assistant_calling(
            "running the check",
            "tc_live",
            "bash",
            serde_json::json!({"command": "sleep 30"}),
        ))),
    });
    app.ui.apply(&AgentEvent::ToolExecutionStart {
        tool_call_id: "tc_live".into(),
        tool_name: "bash".into(),
        args: serde_json::json!({"command": "sleep 30"}),
    });

    // The header's live figure comes from `App::run_started_at`, which the real event loop
    // stamps when a turn begins — not from anything in `UiState`. Driven here the same way,
    // because a test that reached the header by a different route would be testing the route.
    app.run_started_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(12));

    let rows = render_rows(&mut app, 80, 24);
    let header = rows.first().cloned().unwrap_or_default();

    // The header is the surface that can afford a live figure: repainted every frame, and it
    // names the phase. This is the half of the pair M152 could not check, because the probe
    // could not then drive a live tool at all.
    assert!(
        header.contains("running 12s"),
        "the header is where a running call states its time: {header:?}"
    );

    // And the card row in that same frame says nothing about duration. A number here would
    // be the stale kind: the row is cached, so it would freeze at whatever the frame that
    // built it happened to see, then jump when the card was finally rebuilt.
    let card_row = rows
        .iter()
        .find(|r| r.contains("sleep 30"))
        .unwrap_or_else(|| panic!("the running bash card is on screen: {rows:#?}"));
    for badge in ["0ms", "0s", "1ms", "1s"] {
        assert!(
            !card_row.contains(badge),
            "a running card must not draw a duration it cannot keep live \
             (found {badge:?} in {card_row:?})"
        );
    }
}

/// The palette is part of the render-cache key, and this is the test that says so.
///
/// The failure it exists to prevent is specific and was real: `reconcile` takes the theme as an
/// input to *rendering* but never consulted it when deciding what to *rebuild*, so a cached row
/// kept the styles it was drawn with. Add a runtime palette switch without fixing that and the
/// chrome changes colour instantly while the conversation keeps the old palette underneath it
/// \u{2014} which looks like a terminal rendering bug rather than a stale cache.
///
/// `the_light_palette_changes_the_body_text_the_frame_draws` above already proves a light frame
/// draws different body text from a dark one. That is a statement about the *theme*. This is a
/// statement about the *cache*, and it is a different question.
#[test]
fn switching_the_palette_redraws_the_transcript_and_not_just_the_chrome() {
    let mut app = bench_app(UiState::new());
    pin_theme(&mut app, ThemeColorMode::Ansi16);
    for i in 0..6 {
        app.ui
            .push_notice(format!("palette probe line {i} with enough words to wrap"));
    }

    let (dark_buffer, _dark_rows) = render_buffer(&mut app, 80, 24);
    let dark_fg = fg_of(&dark_buffer, "words to wrap");

    // **No `render_cache.clear()` here, on purpose.** The first version of this test called it,
    // and its negative control then passed 363/363 \u{2014} a test that cannot fail is not
    // evidence. `clear()` is a blunt instrument that rebuilds the whole transcript, which is
    // exactly the O(n) work the cache exists to avoid; the palette belongs in `CacheKey` so the
    // *cache* decides, through the mechanism the design already has. The `/theme` handler also
    // calls `clear()`, for rows already drawn on the in-flight frame \u{2014} a different
    // property, and conflating the two here would have hidden this one.
    app.theme.set_palette(crate::ui::theme::ThemeMode::Light);
    let (light_buffer, light_rows) = render_buffer(&mut app, 80, 24);
    let light_fg = fg_of(&light_buffer, "words to wrap");

    assert_ne!(
        dark_fg, light_fg,
        "the transcript row must be re-rendered in the new palette, not reused from the cache"
    );
    // And the content survived, so this is a restyle and not a truncation.
    assert!(
        light_rows
            .iter()
            .any(|r| r.contains("palette probe line 3")),
        "the row must keep its text after a palette switch"
    );
}

/// A reply cut off by the token limit says so on screen (P-127).
///
/// Without the line, a half answer reads as a whole one — the same honesty
/// hole the M62 error marker closes for failures. The EndTurn message in
/// the same frame is the control: one truncated reply, one whole one, and
/// exactly one notice.
#[test]
fn a_truncated_reply_says_so() {
    let mut app = bench_app(UiState::new());
    for (id, text, stop) in [
        (
            "msg-cut",
            "the answer so far",
            epsilon_contracts::message::StopReason::MaxTokens,
        ),
        (
            "msg-whole",
            "the whole answer",
            epsilon_contracts::message::StopReason::EndTurn,
        ),
    ] {
        app.ui
            .apply(&epsilon_contracts::agent_event::AgentEvent::MessageEnd {
                message: Box::new(Message::Assistant(
                    epsilon_contracts::message::AssistantMessage {
                        id: id.into(),
                        timestamp_ms: 0,
                        provider: "faux".into(),
                        model_id: "faux-1".into(),
                        content: vec![epsilon_contracts::content::ContentBlock::text(text)],
                        stop_reason: stop,
                        error_message: None,
                        usage: Default::default(),
                    },
                )),
            });
    }

    let rows = render_rows(&mut app, 80, 24);
    let hits: Vec<_> = rows
        .iter()
        .filter(|r| r.contains("Response was truncated before completion."))
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "exactly one truncation notice for one truncated reply: {rows:#?}"
    );
}

/// A resumed truncated reply says so too (P-127, history path).
///
/// The live path and the resume path render assistant messages separately;
/// a session reopened onto a cut-off answer must not read whole either.
#[test]
fn a_resumed_truncated_reply_says_so() {
    let mut app = bench_app(UiState::new());
    app.ui.hydrate_from_history(&[Message::Assistant(
        epsilon_contracts::message::AssistantMessage {
            id: "msg-resumed".into(),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![epsilon_contracts::content::ContentBlock::text(
                "the resumed half",
            )],
            stop_reason: epsilon_contracts::message::StopReason::MaxTokens,
            error_message: None,
            usage: Default::default(),
        },
    )]);

    let rows = render_rows(&mut app, 80, 24);
    assert!(
        rows.iter()
            .any(|r| r.contains("Response was truncated before completion.")),
        "a replayed cut-off answer must carry its notice: {rows:#?}"
    );
}

/// A stopped reply says so on screen (C-110, live path).
///
/// Same honesty hole as truncation, other cause: without the line, an answer
/// cut short by the reader reads as a whole one. The EndTurn message is the
/// control: one stopped reply, one whole one, exactly one notice.
#[test]
fn a_stopped_reply_says_so() {
    let mut app = bench_app(UiState::new());
    for (id, text, stop) in [
        (
            "msg-stop",
            "the answer so far",
            epsilon_contracts::message::StopReason::Aborted,
        ),
        (
            "msg-whole",
            "the whole answer",
            epsilon_contracts::message::StopReason::EndTurn,
        ),
    ] {
        app.ui
            .apply(&epsilon_contracts::agent_event::AgentEvent::MessageEnd {
                message: Box::new(Message::Assistant(
                    epsilon_contracts::message::AssistantMessage {
                        id: id.into(),
                        timestamp_ms: 0,
                        provider: "faux".into(),
                        model_id: "faux-1".into(),
                        content: vec![epsilon_contracts::content::ContentBlock::text(text)],
                        stop_reason: stop,
                        error_message: None,
                        usage: Default::default(),
                    },
                )),
            });
    }

    let rows = render_rows(&mut app, 80, 24);
    let hits: Vec<_> = rows
        .iter()
        .filter(|r| r.contains("Response was stopped before completion."))
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "exactly one stopped notice for one stopped reply: {rows:#?}"
    );
}

/// A resumed stopped reply says so too (C-110, history path).
#[test]
fn a_resumed_stopped_reply_says_so() {
    let mut app = bench_app(UiState::new());
    app.ui.hydrate_from_history(&[Message::Assistant(
        epsilon_contracts::message::AssistantMessage {
            id: "msg-resumed-stop".into(),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![epsilon_contracts::content::ContentBlock::text(
                "the stopped half",
            )],
            stop_reason: epsilon_contracts::message::StopReason::Aborted,
            error_message: None,
            usage: Default::default(),
        },
    )]);

    let rows = render_rows(&mut app, 80, 24);
    assert!(
        rows.iter()
            .any(|r| r.contains("Response was stopped before completion.")),
        "a replayed stopped answer must carry its notice: {rows:#?}"
    );
}

/// Thinking blocks hide when toggled, and come back on untoggle (P-040).
///
/// Gone — not dimmed, not collapsed — with the flag set, across the whole
/// transcript including already-committed lines, because the gate is at
/// render rather than at push. Unhiding restores every block: hiding drops
/// no data. Both the committed path (history) and the live tail are covered:
/// a hidden transcript that still streams reasoning is not hidden.
/// (The chord arm itself is untested by design: it saves to the global
/// config, and a test must never write the real one — the same reason no
/// test drives the favorites toggle. Config round-trips are pinned in
/// `config.rs` against tempdirs instead.)
#[test]
fn thinking_blocks_hide_when_toggled() {
    use epsilon_contracts::content::ContentBlock;
    let thinking = || {
        Message::Assistant(epsilon_contracts::message::AssistantMessage {
            id: "msg-think".into(),
            timestamp_ms: 0,
            provider: "faux".into(),
            model_id: "faux-1".into(),
            content: vec![
                ContentBlock::Thinking {
                    text: "pondering quietly".into(),
                    signature: None,
                },
                ContentBlock::text("the answer aloud"),
            ],
            stop_reason: epsilon_contracts::message::StopReason::EndTurn,
            error_message: None,
            usage: Default::default(),
        })
    };

    // Committed lines, through history.
    let mut app = bench_app(UiState::new());
    app.ui.hydrate_from_history(&[thinking()]);
    let shown = render_rows(&mut app, 80, 24);
    assert!(
        shown.iter().any(|r| r.contains("pondering quietly")),
        "thinking is visible by default: {shown:#?}"
    );

    app.ui.hide_thinking = true;
    let hidden = render_rows(&mut app, 80, 24);
    assert!(
        !hidden.iter().any(|r| r.contains("pondering quietly")),
        "hidden thinking must leave no pixels: {hidden:#?}"
    );
    assert!(
        hidden.iter().any(|r| r.contains("the answer aloud")),
        "prose stays while thinking hides: {hidden:#?}"
    );

    app.ui.hide_thinking = false;
    let back = render_rows(&mut app, 80, 24);
    assert!(
        back.iter().any(|r| r.contains("pondering quietly")),
        "unhiding restores every block: {back:#?}"
    );

    // The live tail, mid-stream.
    let mut live = bench_app(UiState::new());
    live.ui
        .apply(&epsilon_contracts::agent_event::AgentEvent::MessageUpdate {
            message: Box::new(thinking()),
        });
    let streaming = render_rows(&mut live, 80, 24);
    assert!(
        streaming.iter().any(|r| r.contains("pondering quietly")),
        "the live tail shows while streaming: {streaming:#?}"
    );
    live.ui.hide_thinking = true;
    let quiet = render_rows(&mut live, 80, 24);
    assert!(
        !quiet.iter().any(|r| r.contains("pondering quietly")),
        "hiding must quiet the live tail too: {quiet:#?}"
    );
}
