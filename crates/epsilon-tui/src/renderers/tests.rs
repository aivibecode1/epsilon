//! What a card *says*, tested as a user reads it.
//!
//! The older suites in this crate assert state. These assert rendered text, on purpose: the
//! defect these tests exist for was never a wrong value in `Card` — it was a correct value
//! formatted into the wrong place, and only the painted line shows that.

use super::*;
use crate::ui::card;
use crate::ui::theme::{ColorMode, Glyphs, Theme};
use serde_json::json;

fn args(v: serde_json::Value) -> serde_json::Value {
    v
}

/// The default dark palette, matching `app/golden.rs`, so a painted assertion here
/// is rendered under the same theme the real screen uses.
fn theme() -> Theme {
    Theme::new(ColorMode::Truecolor, Glyphs::detect())
}

#[test]
fn a_read_card_names_the_file_and_the_lines_it_returned() {
    // The facts the tool itself reported; a renderer may not invent others.
    let head = result_head(
        "read",
        &args(json!({"path": "src/main.rs", "offset": 10})),
        &json!({"read": {"first_line": 10, "last_line": 50, "total_lines": 300}}),
        "fn main() {\n",
    );
    assert_eq!(head, "src/main.rs:10–50 of 300");
}

#[test]
fn a_whole_file_read_does_not_announce_a_range_it_covered_everything_in() {
    // `1–300 of 300` is noise: the interesting fact is "the whole file", which is what
    // saying nothing extra leaves room for.
    let head = result_head(
        "read",
        &args(json!({"path": "Cargo.toml"})),
        &json!({"read": {"first_line": 1, "last_line": 42, "total_lines": 42}}),
        "[package]\n",
    );
    assert_eq!(head, "Cargo.toml");
}

#[test]
fn a_grep_card_says_what_it_searched_for_and_where() {
    // The old heuristic returned the bare pattern, so a `grep` card could not distinguish
    // a search that hit from one that hit in the wrong place.
    let head = call_head("grep", &args(json!({"pattern": "fn main", "path": "src/"})));
    assert_eq!(head, "/fn main/ in src/");
}

#[test]
fn a_grep_card_that_stopped_at_the_match_limit_says_so() {
    let head = result_head(
        "grep",
        &args(json!({"pattern": "todo!", "path": "src/"})),
        &json!({"truncated": true, "match_limit_reached": 100}),
        "src/a.rs:1:todo!()\n",
    );
    assert!(
        head.contains("truncated at 100 matches"),
        "a capped search that looks complete is the worst kind of lie: {head}"
    );
}

#[test]
fn a_shell_card_reads_like_a_command_line() {
    assert_eq!(
        call_head("shell", &args(json!({"command": "cargo test --workspace"}))),
        "$ cargo test --workspace"
    );
}

#[test]
fn a_multiline_command_collapses_to_its_first_line() {
    // A header is one line. A `bash -c` script's second line is usually a different idea.
    let head = call_head("shell", &args(json!({"command": "set -e\nrm -rf /"})));
    assert_eq!(head, "$ set -e");
}

#[test]
fn a_shell_card_with_incomplete_output_says_the_output_is_incomplete() {
    // M148: the command exited 0 while a descendant held the pipe. The card already says
    // `partial`; this is the same fact in words, where a reader will actually meet it.
    let head = result_head(
        "shell",
        &args(json!({"command": "npm test"})),
        &json!({"output_complete": false}),
        "child-exited\n",
    );
    assert!(head.contains("output incomplete"), "{head}");
}

#[test]
fn a_complete_shell_result_is_not_qualified() {
    let head = result_head(
        "shell",
        &args(json!({"command": "npm test"})),
        &json!({"output_complete": true}),
        "ok\n",
    );
    assert!(!head.contains("incomplete"), "{head}");
}

#[test]
fn an_unknown_tool_degrades_to_a_phrase_rather_than_a_json_dump() {
    // The old fallback was `truncate(&args.to_string(), 80)`, so any tool whose arguments
    // did not use one of eight hard-coded spellings opened with a JSON object.
    let head = call_head("mystery", &args(json!({"whatever": "yes"})));
    assert!(!head.starts_with('{'), "{head}");
}

#[test]
fn a_tool_with_nothing_sayable_falls_back_to_the_first_output_line() {
    let head = result_head(
        "mystery",
        &args(json!({})),
        &json!({}),
        "\n  found nothing\n",
    );
    // The tool name survives, because this string is also the collapsed transcript line:
    // a row of output with nothing saying which tool produced it is not a fallback.
    assert_eq!(head, "mystery → found nothing");
}

/// The painted assertion. Everything above tests a string; this one goes through
/// [`card::title`], which is what the terminal actually shows.
#[test]
fn the_card_a_user_sees_carries_the_file_and_the_range() {
    let card = crate::state::Card {
        name: "read".into(),
        status: crate::state::CardStatus::Ok,
        tool_call_id: "tc_1".into(),
        started_at_ms: 0,
        ended_at_ms: Some(1_200),
        body: crate::state::CardBody::Text("fn main() {}".into()),
        args: json!({"path": "src/main.rs"}),
        head: "src/main.rs:10–50 of 300".into(),
    };
    let line = card::title(&card, &theme());
    assert!(line.contains("src/main.rs:10–50 of 300"), "{line}");
    assert!(line.contains("read"), "{line}");
    // The regression in one assertion: the old headline put the file's own text up here.
    assert!(!line.contains("fn main"), "{line}");
}

/// And the same path with nothing to say, which must still produce a usable card rather
/// than a leading space or a double separator.
#[test]
fn a_card_with_no_headline_still_renders_its_name() {
    let card = crate::state::Card {
        name: "todo".into(),
        status: crate::state::CardStatus::Ok,
        tool_call_id: "tc_2".into(),
        started_at_ms: 0,
        ended_at_ms: Some(10),
        body: crate::state::CardBody::Text(String::new()),
        args: json!({}),
        head: String::new(),
    };
    let line = card::title(&card, &theme());
    assert!(line.contains("todo"), "{line}");
    assert!(!line.contains("· ·"), "{line}");
}

#[test]
fn a_tool_with_no_output_at_all_still_names_itself() {
    let head = result_head("todo", &args(json!({})), &json!({}), "   ");
    assert_eq!(head, "todo");
}

/// The duplication the fallback invites. The renderer's fallback already leads with the
/// tool's name, because it doubles as the collapsed transcript line; the title prints the
/// name too. A card that read `bash bash → boom` was what that mismatch produced.
#[test]
fn the_title_does_not_print_the_tools_name_twice() {
    let card = crate::state::Card {
        name: "bash".into(),
        status: crate::state::CardStatus::Failed,
        tool_call_id: "tc_3".into(),
        started_at_ms: 0,
        ended_at_ms: Some(5),
        body: crate::state::CardBody::Text("boom".into()),
        args: json!({}),
        head: "bash \u{2192} boom".into(),
    };
    let line = card::title(&card, &theme());
    assert!(!line.contains("bash bash"), "{line}");
    assert!(line.contains("boom"), "{line}");
}
