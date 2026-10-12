//! What a card *says*, tested as a user reads it.
//!
//! The older suites in this crate assert state. These assert rendered text, on purpose: the
//! defect these tests exist for was never a wrong value in `Card` — it was a correct value
//! formatted into the wrong place, and only the painted line shows that.

use super::*;
use crate::ui::card;
use crate::ui::theme::UNICODE;
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
        UNICODE.ellipsis,
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
        UNICODE.ellipsis,
    );
    assert_eq!(head, "Cargo.toml");
}

#[test]
fn a_skill_call_names_the_skill_it_loads() {
    // P-056: a skill invocation renders as a card like every tool call, and
    // the collapsed head must name WHICH skill — the generic fallback reads
    // no `name` key, so without this arm the card carries no identity at all.
    assert_eq!(
        call_head(
            "skill",
            &args(json!({"name": "ratatui-tui"})),
            UNICODE.ellipsis
        ),
        "ratatui-tui"
    );
    assert_eq!(
        call_head("skill", &args(json!({"name": ""})), UNICODE.ellipsis),
        ""
    );
}

#[test]
fn a_read_call_with_a_window_names_its_range() {
    // Pi's tree prefix (`tree-selector.ts:946-960`): the range comes from the CALL's
    // `offset`/`limit` — `start = offset ?? 1`, `end = start + limit - 1` — which the
    // snip sees, exactly as pi does. The earlier note claiming this was `result_head`
    // territory was wrong: pi never reads the result for this line.
    assert_eq!(
        call_head(
            "read",
            &args(json!({"path": "src/main.rs", "offset": 20, "limit": 10})),
            UNICODE.ellipsis
        ),
        "src/main.rs:20-29"
    );
    // Offset alone names where the window starts; limit alone starts at 1.
    assert_eq!(
        call_head(
            "read",
            &args(json!({"path": "src/main.rs", "offset": 20})),
            UNICODE.ellipsis
        ),
        "src/main.rs:20"
    );
    assert_eq!(
        call_head(
            "read",
            &args(json!({"path": "src/main.rs", "limit": 10})),
            UNICODE.ellipsis
        ),
        "src/main.rs:1-10"
    );
    // No window, no range: the head degrades to the bare path rather than a guess.
    // This is the removal control — the range must come from the args, not be invented.
    assert_eq!(
        call_head(
            "read",
            &args(json!({"path": "src/main.rs"})),
            UNICODE.ellipsis
        ),
        "src/main.rs"
    );
}

#[test]
fn a_settled_read_without_reported_lines_keeps_the_requested_window() {
    // `result_head` states actuals when it has them (bare path plus `:first–last`)
    // and the request when it does not — never both (`path:10:10–50 of 300`
    // states the start twice). A result with no `details.read` is a result whose
    // lines are unknown, and the requested window is the only range in evidence.
    let head = result_head(
        "read",
        &args(json!({"path": "src/main.rs", "offset": 20, "limit": 10})),
        &json!({}),
        "line twenty\n",
        UNICODE.ellipsis,
    );
    assert_eq!(head, "src/main.rs:20-29");
}

#[test]
fn a_grep_card_says_what_it_searched_for_and_where() {
    // The old heuristic returned the bare pattern, so a `grep` card could not distinguish
    // a search that hit from one that hit in the wrong place.
    let head = call_head(
        "grep",
        &args(json!({"pattern": "fn main", "path": "src/"})),
        UNICODE.ellipsis,
    );
    assert_eq!(head, "/fn main/ in src/");
}

#[test]
fn a_grep_card_that_stopped_at_the_match_limit_says_so() {
    let head = result_head(
        "grep",
        &args(json!({"pattern": "todo!", "path": "src/"})),
        &json!({"truncated": true, "match_limit_reached": 100}),
        "src/a.rs:1:todo!()\n",
        UNICODE.ellipsis,
    );
    assert!(
        head.contains("truncated at 100 matches"),
        "a capped search that looks complete is the worst kind of lie: {head}"
    );
}

#[test]
fn a_shell_card_reads_like_a_command_line() {
    assert_eq!(
        call_head(
            "shell",
            &args(json!({"command": "cargo test --workspace"})),
            UNICODE.ellipsis
        ),
        "$ cargo test --workspace"
    );
}

#[test]
fn a_multiline_command_collapses_to_its_first_line() {
    // A header is one line. A `bash -c` script's second line is usually a different idea.
    let head = call_head(
        "shell",
        &args(json!({"command": "set -e\nrm -rf /"})),
        UNICODE.ellipsis,
    );
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
        UNICODE.ellipsis,
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
        UNICODE.ellipsis,
    );
    assert!(!head.contains("incomplete"), "{head}");
}

#[test]
fn an_unknown_tool_degrades_to_a_phrase_rather_than_a_json_dump() {
    // The old fallback was `truncate(&args.to_string(), 80)`, so any tool whose arguments
    // did not use one of eight hard-coded spellings opened with a JSON object.
    let head = call_head(
        "mystery",
        &args(json!({"whatever": "yes"})),
        UNICODE.ellipsis,
    );
    assert!(!head.starts_with('{'), "{head}");
}

#[test]
fn a_tool_with_nothing_sayable_falls_back_to_the_first_output_line() {
    let head = result_head(
        "mystery",
        &args(json!({})),
        &json!({}),
        "\n  found nothing\n",
        UNICODE.ellipsis,
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
    let head = result_head(
        "todo",
        &args(json!({})),
        &json!({}),
        "   ",
        UNICODE.ellipsis,
    );
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

/// A truncated shell result names its spill file (P-159).
///
/// The path arrives in `details.full_output_path` (set only when truncation
/// actually spilled); the headline carries it so the rest is retrievable.
/// No key, no name — small outputs must not grow one.
#[test]
fn truncated_shell_result_names_its_spill_file() {
    let spilled = super::result_head(
        "bash",
        &json!({ "command": "make big" }),
        &json!({ "full_output_path": "/tmp/spill/bash-1.txt" }),
        "head…",
        "…",
    );
    assert!(
        spilled.contains("/tmp/spill/bash-1.txt"),
        "must name the file: {spilled}"
    );
    let small = super::result_head(
        "bash",
        &json!({ "command": "echo hi" }),
        &json!({}),
        "hi",
        "…",
    );
    assert!(
        !small.contains("full output"),
        "must not invent one: {small}"
    );
}

/// A failed shell card's title carries the exit number (P-158).
///
/// Pi prints `(exit 101)` in its card's status parts
/// (`bash-execution.ts:192`); Epsilon's status parts are the `· {word}` the title
/// already draws, so the number goes in the headline — the place whose documented
/// job is *what a settled card knows that the call did not*. Before this, the
/// number reached the screen only as the last line of a body the twelve-row cap
/// hides, so the commonest failure in a coding agent cost a scroll.
#[test]
fn a_failed_shell_card_names_its_exit_code() {
    let failed = super::result_head(
        "bash",
        &json!({ "command": "cargo test" }),
        &json!({ "exit_code": 101 }),
        "error[E0425]: boom",
        "…",
    );
    assert!(
        failed.contains("exit 101"),
        "the headline must carry the code: {failed}"
    );
    assert!(
        failed.starts_with("$ cargo test"),
        "and must still read as the command line: {failed}"
    );
}

/// The same headline grows nothing for a command that succeeded.
///
/// `exit 0` beside `ok` is noise, and an absent code — a process killed by a
/// signal, which the body already names `[terminated by signal]` — must not
/// become `exit ` with nothing after it. This is the control the row above is
/// read against: without it, printing the code unconditionally would still pass.
#[test]
fn a_successful_shell_card_grows_no_exit_code() {
    let zero = super::result_head(
        "bash",
        &json!({ "command": "echo hi" }),
        &json!({ "exit_code": 0 }),
        "hi",
        "…",
    );
    assert!(!zero.contains("exit"), "zero must stay silent: {zero}");
    let absent = super::result_head(
        "bash",
        &json!({ "command": "sleep 999" }),
        &json!({ "output_complete": false }),
        "stopped",
        "…",
    );
    assert!(
        !absent.contains("exit"),
        "no code must not print an empty one: {absent}"
    );
    // The incomplete-output marker this headline already carried is unaffected.
    assert!(absent.contains("output incomplete"), "{absent}");
}
