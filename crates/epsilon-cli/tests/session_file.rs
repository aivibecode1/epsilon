//! M210: a launch writes a session file of its own (plan 020 step 2a).
//!
//! The user-visible defect this closes: every launch used to append to one fixed `tui.jsonl`, so
//! `--continue` restored the file's active branch — *all* the conversations this workspace had ever
//! had in the TUI, with no boundary between them — while the header still named only the first.
//!
//! These assert the naming call site, not the naming *rule* (`epsilon_session::time`'s own tests
//! cover the rule against pi's two functions). What matters here is the property the defect was:
//! **two launches do not land on one file.**

use epsilon_cli::runtime::{fresh_session_file, now_millis};

#[test]
fn two_launches_do_not_land_on_one_file() {
    // The same millisecond, two different tokens: the counter in `new_id` is what separates them,
    // and a bare timestamp would not have.
    let now = 1_700_000_000_123;
    let a = fresh_session_file(now, &epsilon_contracts::new_id("ses"));
    let b = fresh_session_file(now, &epsilon_contracts::new_id("ses"));
    assert_ne!(a, b, "two sessions, one file is the defect this closes");
}

#[test]
fn the_name_is_a_generated_token_and_not_the_header_id() {
    // Recorded rather than implied: the store still mints the session's own header id, so the id in
    // the name and the id in the header are different strings. The name's component is a uniqueness
    // token; unifying the two is a follow-up in the plan, not a claim this test makes.
    let name = fresh_session_file(1_700_000_000_123, "ses_abc");
    assert!(
        name.starts_with(".epsilon/sessions/"),
        "the file stays in the sessions directory: {name}"
    );
    assert!(name.ends_with(".jsonl"), "{name}");
    assert!(name.contains("ses_abc"), "{name}");
    // `:` and `.` in the timestamp became `-`, per pi's `replace(/[:.]/g, "-")`.
    assert!(name.contains("2023-11-14T22-13-20-123Z"), "{name}");
}

#[test]
fn the_clock_the_name_uses_is_this_century() {
    // A weak assertion on purpose: it catches a clock that returns 0 or a duration since some other
    // epoch, which is the failure a naming bug would actually produce (all launches in 1970).
    let now = now_millis();
    assert!(
        now > 1_700_000_000_000,
        "the name would be dated before 2023: {now}"
    );
}
