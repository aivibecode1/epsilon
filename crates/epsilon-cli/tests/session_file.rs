//! M210: a launch writes a session file of its own (plan 020 step 2a); M217 put it in a
//! directory of its own (plan 020 step 2b).
//!
//! The user-visible defect 2a closes: every launch used to append to one fixed `tui.jsonl`, so
//! `--continue` restored the file's active branch — *all* the conversations this workspace had ever
//! had in the TUI, with no boundary between them — while the header still named only the first.
//! What 2b adds is the **directory**: pi writes `sessions/--<cwd>--/`, so "the newest session in
//! this workspace" is answerable across workspaces rather than only inside one, and it is the half
//! that keeps a second workspace's sessions out of this one's list.
//!
//! These assert the naming call site, not the naming *rule* (`epsilon_session::time`'s own tests
//! cover the rule against pi's two functions). What matters here is the property the defect was:
//! **two launches do not land on one file**, and the path they land in is the one readers resolve.

use std::path::Path;

use epsilon_cli::runtime::{fresh_session_file, now_millis};

#[test]
fn two_launches_do_not_land_on_one_file() {
    // The same millisecond, two different tokens: the counter in `new_id` is what separates them,
    // and a bare timestamp would not have.
    let now = 1_700_000_000_123;
    let cwd = Path::new("/w");
    let a = fresh_session_file(now, &epsilon_contracts::new_id("ses"), cwd);
    let b = fresh_session_file(now, &epsilon_contracts::new_id("ses"), cwd);
    assert_ne!(a, b, "two sessions, one file is the defect this closes");
}

#[test]
fn the_name_is_a_generated_token_and_not_the_header_id() {
    // Recorded rather than implied: the store still mints the session's own header id, so the id in
    // the name and the id in the header are different strings. The name's component is a uniqueness
    // token; unifying the two is a follow-up in the plan, not a claim this test makes.
    let name = fresh_session_file(1_700_000_000_123, "ses_abc", Path::new("/w"));
    assert!(
        name.starts_with(".epsilon/sessions/"),
        "the file stays in the sessions directory: {name}"
    );
    assert!(name.ends_with(".jsonl"), "{name}");
    assert!(name.contains("ses_abc"), "{name}");
    // `:` and `.` in the timestamp became `-`, per pi's `replace(/[:.]/g, "-")`.
    assert!(name.contains("2023-11-14T22-13-20-123Z"), "{name}");
}

/// M217: the file lands in **this workspace's** directory, and two workspaces do not share one.
///
/// The directory name is pi's (`sessionDirectoryName`, `repo.ts:42-45`), so it is asserted against
/// the reference's own output rather than against this implementation's composition — and the
/// POSIX case is the interesting one, because the leading separator is stripped *before* the
/// replacements and a naive composition produces `---w--`.
#[test]
fn the_file_lands_in_its_own_workspaces_directory() {
    let here = fresh_session_file(0, "ses_a", Path::new("/w"));
    assert_eq!(
        here, ".epsilon/sessions/--w--/1970-01-01T00-00-00-000Z_ses_a.jsonl",
        "the reference's directory name, then its file name"
    );
    let elsewhere = fresh_session_file(0, "ses_a", Path::new("/other"));
    assert!(
        elsewhere.contains("/--other--/"),
        "another working copy gets another directory: {elsewhere}"
    );
    assert_ne!(
        here, elsewhere,
        "two workspaces, one lane is the defect 2b closes"
    );
}

/// The path a reader resolves and the path a writer produces are the same rule, called twice — the
/// property that makes the union in `epsilon_session` safe to rely on. It is asserted here because
/// this is where both sides meet: the writer (`fresh_session_file`) and the reader
/// (`time::session_file_for`) would otherwise be free to drift.
#[test]
fn the_writer_and_the_reader_agree_on_where_a_file_goes() {
    let root = Path::new(".epsilon").join("sessions");
    let cwd = Path::new("/w");
    let written = fresh_session_file(0, "ses_a", cwd);
    assert_eq!(
        Path::new(&written),
        epsilon_session::time::session_file_for(&root, cwd, 0, "ses_a"),
        "a launch writes where the readers look"
    );
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
