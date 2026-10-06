//! Session export rendering: HTML and JSON writers over a session store.
//!
//! Moved from the terminal frontend (full-port round 14) so both front ends
//! share one implementation: export is a capability of the session, not pixels
//! of either UI. TUI `/export` and `/share` call these; the desktop will too.

use crate::store::SessionStore;

/// Render the store's entries as a standalone HTML page into `dest`, creating
/// parents.
///
/// One `<pre>` block with one JSON line per entry, HTML-escaped — the same
/// bytes the terminal has always written, so an export from either front end
/// reads identically. Returns the entry count for the notice.
pub fn write_html(store: &SessionStore, dest: &std::path::Path) -> Result<usize, String> {
    let entries = store.read_all().map_err(|e| format!("{e}"))?;
    let n = entries.len();
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{e}"))?;
    }
    let mut html = String::new();
    html.push_str(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>epsilon export</title></head><body><pre>\n",
    );
    for e in &entries {
        let line = serde_json::to_string(e).unwrap_or_else(|_| "{}".into());
        let esc = line
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        html.push_str(&esc);
        html.push('\n');
    }
    html.push_str("</pre></body></html>\n");
    std::fs::write(dest, html).map_err(|e| format!("{e}"))?;
    Ok(n)
}

/// Render the store's entries as pretty JSON into `dest`, creating parents.
///
/// Returns the entry count for the notice.
pub fn write_json(store: &SessionStore, dest: &std::path::Path) -> Result<usize, String> {
    let entries = store.read_all().map_err(|e| format!("{e}"))?;
    let n = entries.len();
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{e}"))?;
    }
    let pretty = serde_json::to_string_pretty(&entries).map_err(|e| format!("{e}"))?;
    std::fs::write(dest, pretty).map_err(|e| format!("{e}"))?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use epsilon_contracts::session_entry::EntryKind;

    fn append_user(store: &mut SessionStore, id: &str, text: &str) {
        use epsilon_contracts::content::UserContent;
        use epsilon_contracts::message::{Message, UserMessage};
        store
            .append(EntryKind::Message {
                message: Message::User(UserMessage {
                    id: id.into(),
                    timestamp_ms: 1,
                    content: vec![UserContent::Text {
                        text: text.to_string(),
                    }],
                }),
                api_request_sha256: None,
            })
            .expect("message lands");
    }

    fn open_store(dir: &std::path::Path, name: &str) -> SessionStore {
        SessionStore::open(dir.join(name)).expect("store opens")
    }

    #[test]
    fn html_export_escapes_and_counts() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut store = open_store(dir.path(), "s.jsonl");
        append_user(&mut store, "m1", "hello <world> & friends");
        let dest = dir.path().join("sub").join("out.html");
        assert_eq!(write_html(&store, &dest).expect("write"), 1);
        let html = std::fs::read_to_string(&dest).expect("read back");
        assert!(html.contains("<html"), "standalone page");
        assert!(html.contains("hello"), "message text present");
        assert!(
            html.contains("&lt;world&gt;") && html.contains("&amp;"),
            "markup escaped, not rendered"
        );
        assert!(!html.contains("<world>"), "raw markup must not survive");
    }

    #[test]
    fn json_export_is_an_array_of_entries() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut store = open_store(dir.path(), "s.jsonl");
        append_user(&mut store, "m1", "hello");
        append_user(&mut store, "m2", "again");
        let dest = dir.path().join("out.json");
        assert_eq!(write_json(&store, &dest).expect("write"), 2);
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&dest).expect("read back"))
                .expect("valid json");
        let arr = v.as_array().expect("json export must be an array");
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn empty_session_exports_empty_containers() {
        let dir = tempfile::tempdir().expect("tmp");
        let store = open_store(dir.path(), "e.jsonl");
        let html = dir.path().join("e.html");
        let json = dir.path().join("e.json");
        assert_eq!(write_html(&store, &html).expect("write"), 0);
        assert_eq!(write_json(&store, &json).expect("write"), 0);
        let body = std::fs::read_to_string(&html).expect("read back");
        assert!(body.contains("<pre>"), "page frame present with no entries");
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&json).expect("read back"))
                .expect("valid json");
        assert_eq!(v, serde_json::json!([]), "empty array, not null");
    }
}
