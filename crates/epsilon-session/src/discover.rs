//! Session discovery (M57): what a sessions directory holds, answered cheaply.
//!
//! Two frontends need this and neither should own it: the CLI's `--continue`
//! asks "which is newest for this cwd" (pi `findMostRecentSession`), and the
//! TUI's `/sessions` browser asks "what is in here" as a list. Both used to be
//! impossible to share — discovery lived in the CLI, and the TUI could only
//! resume a path a human already knew.
//!
//! Everything here is BOUNDED: a summary reads the header line, the first
//! [`SCAN_LINES`] lines, and the last [`TAIL_BYTES`] bytes of a file. Listing a
//! directory of long sessions therefore costs about the same as listing an
//! empty one — a browser that stalls on a 40 MB transcript is not a browser.
//!
//! Foreign `.jsonl` files share these directories (pi's own note): a file whose
//! first non-blank line is not a header is skipped, never guessed at.

use std::path::{Path, PathBuf};

use epsilon_contracts::session_entry::{EntryKind, SessionHeader, SessionLine};

/// Lines scanned from the head for the first user message.
pub const SCAN_LINES: usize = 64;

/// Bytes read from the tail for the display name (which is written whenever
/// `/name` runs, i.e. anywhere in the file).
pub const TAIL_BYTES: usize = 8 * 1024;

/// A session-file summary for listers and pickers (M57).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    /// File on disk.
    pub path: PathBuf,
    /// Stable session id from the header (`ses_…`).
    pub id: String,
    /// Working directory the session started in (empty on v1 files).
    pub cwd: String,
    /// Last-modified time in epoch millis (`0` when the filesystem says
    /// nothing — listings stay sortable regardless).
    pub modified_ms: u64,
    /// File size in bytes.
    pub bytes: u64,
    /// Display name set by `/name` (`EntryKind::Custom { "session_name" }`).
    pub name: Option<String>,
    /// First user message text, clipped to one line — the usual "what was this
    /// session about" handle when no name was set.
    pub first_message: Option<String>,
    /// Forked-from session id when the header carries one.
    pub parent_session: Option<String>,
    /// Another process is writing this session right now (M61): its lease file
    /// exists and is fresh. The picker marks these `· in use` so a refused
    /// resume is explained before the user tries it.
    pub in_use: bool,
}

impl SessionSummary {
    /// What a picker should show: the display name, else the session's first
    /// user message, else the file's stem (never an empty line).
    pub fn title(&self) -> String {
        if let Some(n) = self.name.as_deref().filter(|n| !n.trim().is_empty()) {
            return n.trim().to_string();
        }
        if let Some(m) = self
            .first_message
            .as_deref()
            .filter(|m| !m.trim().is_empty())
        {
            return m.trim().to_string();
        }
        self.path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "(unnamed session)".to_string())
    }
}

/// Best-effort first-line header read (pi
/// `readSessionHeaderForDiscovery`): `None` for missing/unreadable/foreign
/// files — discovery never fails a run. Blank leading lines are tolerated, the
/// same leniency the store's own scan applies.
pub fn read_session_header(path: &Path) -> Option<SessionHeader> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    for _ in 0..16 {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        if line.trim().is_empty() {
            continue;
        }
        return serde_json::from_str::<SessionLine>(&line)
            .ok()
            .and_then(|l| match l {
                SessionLine::Header(h) => Some(h),
                SessionLine::Entry(_) => None,
            });
    }
    None
}

/// Milliseconds since the Unix epoch for a file's mtime (`0` when unknown).
fn modified_ms(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// First user message text from the head of a session file, one line, clipped.
fn first_user_message(path: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    for line in reader.lines().take(SCAN_LINES) {
        let Ok(line) = line else { break };
        let Some(Ok(parsed)) = SessionLine::parse(&line) else {
            continue;
        };
        if let SessionLine::Entry(entry) = parsed
            && let EntryKind::Message { message, .. } = &entry.kind
            && let Some(text) = first_line_of(message)
        {
            return Some(text);
        }
    }
    None
}

/// The last `session_name` custom entry, read from the tail (the entry can be
/// written anywhere in the file; the newest one wins).
fn display_name(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_BYTES as u64);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = String::new();
    file.read_to_string(&mut buf).ok()?;
    let mut found = None;
    for line in buf.lines() {
        let Some(Ok(parsed)) = SessionLine::parse(line) else {
            // A tail read can start mid-line; a parse failure there is expected.
            continue;
        };
        if let SessionLine::Entry(entry) = parsed
            && let EntryKind::Custom {
                custom_type,
                data: Some(data),
            } = &entry.kind
            && custom_type == "session_name"
            && let Some(name) = data.get("name").and_then(|v| v.as_str())
        {
            found = Some(name.to_string());
        }
    }
    found
}

/// One-line, clipped rendering of a message's text (first line only).
fn first_line_of(message: &epsilon_contracts::message::Message) -> Option<String> {
    use epsilon_contracts::content::UserContent;
    let text = match message {
        epsilon_contracts::message::Message::User(u) => u.content.iter().find_map(|c| match c {
            UserContent::Text { text } => Some(text.clone()),
            _ => None,
        })?,
        epsilon_contracts::message::Message::Assistant(a) => {
            a.content.iter().find_map(|c| match c {
                epsilon_contracts::content::ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })?
        }
        _ => return None,
    };
    let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
    if line.is_empty() {
        return None;
    }
    Some(line.chars().take(160).collect())
}

/// Summarize one file; `None` when it is not a session (foreign `jsonl`).
pub fn summarize(path: &Path) -> Option<SessionSummary> {
    let header = read_session_header(path)?;
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Some(SessionSummary {
        path: path.to_path_buf(),
        id: header.id,
        cwd: header.cwd,
        modified_ms: modified_ms(path),
        bytes,
        name: display_name(path),
        first_message: first_user_message(path),
        parent_session: header.parent_session,
        in_use: is_leased(path),
    })
}

/// Whether a live writer holds this session (M61). A fresh `<session>.lock` is
/// the signal; an old one belongs to a crashed process and is already stealable
/// by the next writer, so it is not reported as in use.
fn is_leased(path: &Path) -> bool {
    let lock = path.with_extension("lock");
    let Ok(meta) = std::fs::metadata(&lock) else {
        return false;
    };
    let age = meta
        .modified()
        .ok()
        .and_then(|m| std::time::SystemTime::now().duration_since(m).ok())
        .unwrap_or(std::time::Duration::ZERO);
    age < crate::lease::STALE_AFTER
}

/// Every session in `dir`, newest first. Files that are not sessions are
/// skipped silently (the directory is shared with other tooling).
pub fn list_sessions(dir: &Path) -> Vec<SessionSummary> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<SessionSummary> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| summarize(&e.path()))
        .collect();
    out.sort_by(|a, b| {
        b.modified_ms
            .cmp(&a.modified_ms)
            .then_with(|| b.path.cmp(&a.path))
    });
    out
}

/// Newest session under `dir` (for `--continue`), optionally restricted to
/// sessions started from `cwd`.
///
/// pi `findMostRecentSession` filters by session-cwd and skips files whose
/// first line is not a valid header. The cwd filter only applies when the
/// session carries one — headerless v1 files have no cwd to compare, so they
/// stay eligible rather than vanishing from `--continue`.
pub fn latest_session(dir: &Path, cwd: Option<&Path>) -> Option<PathBuf> {
    let want = cwd.map(|c| c.to_string_lossy().into_owned());
    let mut best: Option<SessionSummary> = None;
    for summary in list_sessions(dir) {
        let matches = match &want {
            None => true,
            Some(want) => summary.cwd.is_empty() || summary.cwd == *want,
        };
        if matches {
            // `list_sessions` is newest-first; the first match is the answer.
            best = Some(summary);
            break;
        }
    }
    best.map(|s| s.path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use epsilon_contracts::message::{Message, UserMessage};
    use epsilon_contracts::session_entry::{EntryKind, SessionEntry};
    use std::io::Write as _;

    fn header_line(id: &str, cwd: &str) -> String {
        format!(
            "{}\n",
            serde_json::json!({
                "type": "session",
                "version": 3,
                "id": id,
                "timestamp": "2026-09-14T10:00:00Z",
                "cwd": cwd,
            })
        )
    }

    fn user_line(text: &str) -> String {
        let entry = SessionEntry {
            id: "e1".to_string(),
            seq: 1,
            parent_id: None,
            timestamp_ms: 0,
            kind: EntryKind::Message {
                message: Message::User(UserMessage {
                    id: "m1".to_string(),
                    timestamp_ms: 0,
                    content: vec![epsilon_contracts::content::UserContent::Text {
                        text: text.to_string(),
                    }],
                }),
                api_request_sha256: None,
            },
        };
        format!("{}\n", serde_json::to_string(&entry).unwrap())
    }

    fn name_line(name: &str) -> String {
        let entry = SessionEntry {
            id: "e2".to_string(),
            seq: 2,
            parent_id: None,
            timestamp_ms: 0,
            kind: EntryKind::Custom {
                custom_type: "session_name".to_string(),
                data: Some(serde_json::json!({"name": name})),
            },
        };
        format!("{}\n", serde_json::to_string(&entry).unwrap())
    }

    fn write_session(dir: &Path, file: &str, id: &str, cwd: &str, body: &[String]) {
        let mut f = std::fs::File::create(dir.join(file)).unwrap();
        f.write_all(header_line(id, cwd).as_bytes()).unwrap();
        for line in body {
            f.write_all(line.as_bytes()).unwrap();
        }
        f.flush().unwrap();
    }

    #[test]
    fn listing_is_newest_first_and_skips_foreign_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write_session(
            dir,
            "old.jsonl",
            "ses_old",
            "/w",
            &[user_line("first task")],
        );
        std::thread::sleep(std::time::Duration::from_millis(30));
        write_session(
            dir,
            "new.jsonl",
            "ses_new",
            "/w",
            &[user_line("second task"), name_line("nightly sweep")],
        );
        // Not sessions: no header, wrong extension.
        std::fs::write(dir.join("foreign.jsonl"), "{\"nope\":1}\n").unwrap();
        std::fs::write(dir.join("notes.txt"), "hello\n").unwrap();

        let rows = list_sessions(dir);
        assert_eq!(rows.len(), 2, "foreign files are skipped: {rows:?}");
        assert_eq!(rows[0].id, "ses_new");
        assert_eq!(rows[1].id, "ses_old");
        assert_eq!(rows[0].title(), "nightly sweep", "name wins over text");
        assert_eq!(rows[1].title(), "first task", "else the first user line");
        assert_eq!(rows[0].first_message.as_deref(), Some("second task"));
    }

    #[test]
    fn title_falls_back_to_the_file_stem() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write_session(dir, "tui.jsonl", "ses_x", "/w", &[]);
        let rows = list_sessions(dir);
        assert_eq!(rows[0].title(), "tui");
        assert_eq!(rows[0].first_message, None);
    }

    #[test]
    fn latest_session_is_newest_and_honours_the_cwd_filter() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write_session(dir, "other.jsonl", "ses_a", "/elsewhere", &[]);
        std::thread::sleep(std::time::Duration::from_millis(30));
        write_session(dir, "mine.jsonl", "ses_b", "/w", &[]);

        assert_eq!(latest_session(dir, None), Some(dir.join("mine.jsonl")));
        assert_eq!(
            latest_session(dir, Some(Path::new("/w"))),
            Some(dir.join("mine.jsonl"))
        );
        // A cwd nothing matches has no answer — the caller then starts fresh.
        assert_eq!(latest_session(dir, Some(Path::new("/nowhere"))), None);
    }

    #[test]
    fn missing_directory_lists_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nope");
        assert!(list_sessions(&missing).is_empty());
        assert_eq!(latest_session(&missing, None), None);
    }

    #[test]
    fn summaries_read_bounded_heads_and_tails() {
        // A long session body must not change what the summary reports (the
        // scan is bounded) — and a name written late must still be found.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut body: Vec<String> = (0..SCAN_LINES * 3)
            .map(|i| user_line(&format!("filler {i}")))
            .collect();
        body.push(name_line("late name"));
        write_session(dir, "long.jsonl", "ses_long", "/w", &body);
        let rows = list_sessions(dir);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].first_message.as_deref(), Some("filler 0"));
        assert_eq!(rows[0].title(), "late name");
        assert!(rows[0].bytes > 0);
    }
}
