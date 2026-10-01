//! What the agent changed, as it changes it.
//!
//! # Why watch the disk rather than read the tool's arguments
//!
//! `write` carries the new content and `edit` carries the exact old and new strings, so
//! a diff could be computed from the call alone. **It would also be the wrong question**:
//! what is worth reviewing is what is on disk afterwards, not what the agent meant to do.
//!
//! So the watcher snapshots the paths a call *names* and re-reads them when it finishes.
//! That covers `write` and `edit`, which send a `path`.
//!
//! **What it does not cover, stated here rather than discovered later:** a `bash` command
//! that edits files. Its arguments carry a `command`, not a path, and scraping a shell
//! string for path-shaped tokens turns every tool call into a filesystem probe and will
//! happily report a file the user merely *mentioned*. Watching `command` properly needs a
//! shell parser, and **a wrong diff is worse than no diff** — so this is a named gap, not
//! an oversight, and `a_shell_command_names_no_file_so_nothing_is_watched` pins it.
//!
//! # Why the snapshot happens at the call, not after it
//!
//! `ToolExecutionStart` is emitted before the tool runs, so reading the file at that
//! moment is genuinely "before". Reading it after would produce an empty diff for every
//! edit — which is the failure mode of a review pane that silently shows nothing.
//!
//! # The diff is a prefix/suffix diff, and it says so when the middle is a rewrite
//!
//! A full LCS diff is the right tool for reviewing an unfamiliar change. Here it is not,
//! for two reasons: a coding agent's edits are overwhelmingly *localised* (one hunk in a
//! known file), which a prefix/suffix diff renders exactly; and an approximate diff shown
//! without qualification is worse than a coarse one shown honestly.
//!
//! The affix walk always accounts for every difference — its prefix and suffix are
//! maximal, so the two middles *are* the changed region. What varies is whether that
//! region reads as a small edit or a rewrite, and past [`COARSE_MIDDLE_LINES`] the row
//! says *"N lines replaced"* rather than implying a tidy hunk the code did not compute.

use std::collections::BTreeMap;
use std::path::Path;

/// One changed line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLine {
    /// Present before and after, unchanged.
    Context(String),
    /// Only after: added.
    Added(String),
    /// Only before: removed.
    Removed(String),
}

impl DiffLine {
    /// The line's text.
    pub fn text(&self) -> &str {
        match self {
            Self::Context(t) | Self::Added(t) | Self::Removed(t) => t,
        }
    }

    /// The marker a reader sees: `+`, `-`, or a space.
    pub fn marker(&self) -> char {
        match self {
            Self::Context(_) => ' ',
            Self::Added(_) => '+',
            Self::Removed(_) => '-',
        }
    }
}

/// What changed in one file.
/// What changed in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Workspace-relative path.
    pub path: String,
    /// The diff, as far as prefix/suffix resolution can honestly describe it.
    pub lines: Vec<DiffLine>,
    /// `true` when the file did not exist before.
    pub created: bool,
    /// `true` when the middle of the diff is a block replacement rather than resolved
    /// lines, so anything that renders it must label it as one.
    pub coarse: bool,
}

impl Change {
    /// How many lines were added.
    pub fn added(&self) -> usize {
        self.lines
            .iter()
            .filter(|l| matches!(l, DiffLine::Added(_)))
            .count()
    }

    /// How many lines were removed.
    pub fn removed(&self) -> usize {
        self.lines
            .iter()
            .filter(|l| matches!(l, DiffLine::Removed(_)))
            .count()
    }

    /// Whether anything changed at all.
    pub fn is_empty(&self) -> bool {
        self.added() == 0 && self.removed() == 0
    }

    /// A one-line summary for a transcript row.
    ///
    /// Names the file, the two counts, and — when the prefix/suffix diff could not
    /// resolve the middle — says so rather than implying a hunk it did not compute.
    pub fn summary(&self) -> String {
        if self.created {
            return format!("created {} · +{}", self.path, self.added());
        }
        let (a, r) = (self.added(), self.removed());
        if self.coarse {
            return format!(
                "{} · +{} −{} · middle {} lines replaced",
                self.path,
                a,
                r,
                a.max(r)
            );
        }
        format!("{} · +{} −{}", self.path, a, r)
    }

    /// `true` when the middle of this diff is a block replacement rather than resolved
    /// lines. Public because a caller rendering the diff needs to label it too.
    pub fn is_coarse(&self) -> bool {
        self.coarse
    }
}

/// Tracks the files a running tool call might change.
///
/// One instance per window. Cheap: an entry is a path and the file's text at the moment
/// the call started, and only paths a call actually names are ever held.
#[derive(Debug, Default, Clone)]
pub struct ChangeWatcher {
    /// `path -> its content before the call`. `None` means "it did not exist".
    before: BTreeMap<String, Option<String>>,
}

/// How many arguments' worth of text are read from a call before giving up on it.
///
/// A tool's arguments are small by construction, but a `bash` command can be enormous
/// and this is on the path of every tool call, so the read is bounded.
const MAX_ARG_TEXT: usize = 64 * 1024;

impl ChangeWatcher {
    /// A watcher that has seen nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether any call is currently being watched.
    pub fn is_watching(&self) -> bool {
        !self.before.is_empty()
    }

    /// Record the state of every workspace file a call mentions.
    ///
    /// Called when the call *starts*, which is what makes the result a "before".
    pub fn begin(&mut self, args: &serde_json::Value, workspace: &Path) {
        self.before.clear();
        for path in candidate_paths(args, workspace) {
            let text = std::fs::read_to_string(workspace.join(&path)).ok();
            self.before.insert(path, text);
        }
    }

    /// Compare what is on disk now with what was there when the call began.
    ///
    /// Returns one [`Change`] per file that actually moved. A call that changed
    /// nothing returns an empty vector — **and that is reported as nothing rather than
    /// as a row saying "no changes"**, because a review pane full of "no changes" is a
    /// review pane nobody reads.
    pub fn finish(&mut self, workspace: &Path) -> Vec<Change> {
        let before = std::mem::take(&mut self.before);
        let mut changes = Vec::new();
        for (path, was) in before {
            let now = std::fs::read_to_string(workspace.join(&path)).ok();
            if now == was {
                continue;
            }
            // `created` is read before the contents are unwrapped: a file that did not
            // exist has no "before" text, and saying so is what distinguishes
            // "created" from "emptied".
            let created = was.is_none();
            let (lines, coarse) =
                diff_lines(was.as_deref().unwrap_or(""), now.as_deref().unwrap_or(""));
            changes.push(Change {
                path,
                lines,
                created,
                coarse,
            });
        }
        changes
    }
}

/// The workspace-relative paths a tool call mentions.
///
/// Only a `path`-shaped string is considered, and only when it resolves inside the
/// workspace. That is deliberately conservative: the alternative — scraping every string
/// that looks like a path out of a shell command — turns a `grep` into a filesystem walk
/// on every tool call, and a path the user mentioned in prose is not a file the agent
/// is about to change.
fn candidate_paths(args: &serde_json::Value, workspace: &Path) -> Vec<String> {
    let Some(obj) = args.as_object() else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for key in ["path", "file_path", "file"] {
        let Some(value) = obj.get(key).and_then(|v| v.as_str()) else {
            continue;
        };
        if value.len() > MAX_ARG_TEXT {
            continue;
        }
        let Some(relative) = relative_if_inside(workspace, Path::new(value)) else {
            continue;
        };
        if !out.contains(&relative) {
            out.push(relative);
        }
    }
    out
}

/// `path` as a workspace-relative string, or `None` when it escapes the workspace.
///
/// The escape check is the same one the tools apply before they write, so the watcher
/// cannot be talked into reading `/etc/passwd` by a path in a tool call.
fn relative_if_inside(workspace: &Path, path: &Path) -> Option<String> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    };
    let normalized = normalize(&joined);
    let root = normalize(workspace);
    let rest = normalized.strip_prefix(&root).ok()?;
    if rest.as_os_str().is_empty() {
        return None;
    }
    Some(rest.to_string_lossy().replace('\\', "/"))
}

/// Resolve `.` and `..` without touching the filesystem.
///
/// `std::path::Component` does not collapse them, and a path like `src/../Cargo.toml`
/// would otherwise fail the containment check for no reason.
fn normalize(path: &Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The longest common prefix and suffix of two line sequences.
fn split_affixes<T: PartialEq>(old: &[T], new: &[T]) -> (usize, usize) {
    let max_prefix = old.len().min(new.len());
    let mut prefix = 0;
    while prefix < max_prefix && old[prefix] == new[prefix] {
        prefix += 1;
    }
    let max_suffix = old.len().min(new.len()) - prefix;
    let mut suffix = 0;
    while suffix < max_suffix && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix] {
        suffix += 1;
    }
    (prefix, suffix)
}

/// A prefix/suffix diff, and whether it had to fall back to a block replacement.
///
/// Returns `(lines, coarse)`. `coarse` is `true` when the common prefix and suffix do not
/// account for every changed line — the caller must then say so rather than presenting
/// the middle as a resolved hunk.
fn diff_lines(old_text: &str, new_text: &str) -> (Vec<DiffLine>, bool) {
    let old: Vec<&str> = old_text.lines().collect();
    let new: Vec<&str> = new_text.lines().collect();
    let (prefix, suffix) = split_affixes(&old, &new);

    let old_mid = &old[prefix..old.len() - suffix];
    let new_mid = &new[prefix..new.len() - suffix];

    let mut lines: Vec<DiffLine> = Vec::new();
    for line in &old[..prefix] {
        lines.push(DiffLine::Context((*line).to_string()));
    }
    for line in old_mid {
        lines.push(DiffLine::Removed((*line).to_string()));
    }
    for line in new_mid {
        lines.push(DiffLine::Added((*line).to_string()));
    }
    for line in &new[new.len() - suffix..] {
        lines.push(DiffLine::Context((*line).to_string()));
    }

    // The affix walk always accounts for every difference: the prefix and suffix are
    // maximal, so the two middles are exactly the changed region. What varies is
    // whether that region is a **localised edit** or a **rewrite**, and only the second
    // is worth labelling. A one-line replacement has a one-line middle and is exactly
    // what this resolves; calling it "N lines replaced" would be wrong in the other
    // direction.
    let coarse = old_mid.len() + new_mid.len() >= COARSE_MIDDLE_LINES;
    (lines, coarse)
}

/// From this many changed lines in the middle, the row stops implying a small edit.
///
/// Eight is chosen against the thing this pane is for: an agent fixing a bug changes
/// one to five lines, and anything past that is a rewrite the reader should be told
/// about rather than shown as a tidy hunk.
const COARSE_MIDDLE_LINES: usize = 8;

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> tempfile::TempDir {
        tempfile::tempdir().expect("a workspace")
    }

    #[test]
    fn a_call_that_changes_nothing_reports_nothing() {
        let dir = ws();
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        let mut w = ChangeWatcher::new();
        w.begin(&serde_json::json!({"path": "a.txt"}), dir.path());
        assert!(w.finish(dir.path()).is_empty());
    }

    #[test]
    fn a_single_line_edit_is_shown_as_exactly_that() {
        let dir = ws();
        std::fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
        let mut w = ChangeWatcher::new();
        w.begin(&serde_json::json!({"path": "a.txt"}), dir.path());
        std::fs::write(dir.path().join("a.txt"), "one\nTWO\nthree\n").unwrap();
        let changes = w.finish(dir.path());
        assert_eq!(changes.len(), 1);
        let c = &changes[0];
        assert_eq!(c.added(), 1, "{:?}", c.lines);
        assert_eq!(c.removed(), 1, "{:?}", c.lines);
        assert!(!c.is_coarse(), "a one-line edit is exact");
        assert_eq!(c.summary(), "a.txt · +1 −1", "{}", c.summary());
    }

    #[test]
    fn a_pure_insertion_keeps_its_context() {
        let dir = ws();
        std::fs::write(dir.path().join("a.txt"), "one\nthree\n").unwrap();
        let mut w = ChangeWatcher::new();
        w.begin(&serde_json::json!({"path": "a.txt"}), dir.path());
        std::fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
        let c = w.finish(dir.path());
        assert_eq!(c[0].added(), 1);
        assert_eq!(c[0].removed(), 0);
        assert!(!c[0].is_coarse(), "{:?}", c[0].lines);
    }

    /// Two changed regions that the affix walk cannot separate must be **labelled** as a
    /// block replacement, never presented as a hunk the code did not compute.
    #[test]
    fn two_distant_edits_are_reported_as_a_block_and_say_so() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\n";
        let new = "A\nb\nc\nd\ne\nf\ng\nH\n";
        let (lines, coarse) = diff_lines(old, new);
        assert!(coarse, "this is the case the honesty rule exists for");
        assert!(lines.iter().any(|l| matches!(l, DiffLine::Added(_))));
        assert!(lines.iter().any(|l| matches!(l, DiffLine::Removed(_))));
    }

    #[test]
    fn a_created_file_says_it_was_created() {
        let dir = ws();
        let mut w = ChangeWatcher::new();
        w.begin(&serde_json::json!({"path": "new.txt"}), dir.path());
        std::fs::write(dir.path().join("new.txt"), "hello\n").unwrap();
        let c = w.finish(dir.path());
        assert!(c[0].created);
        assert_eq!(c[0].summary(), "created new.txt · +1");
    }

    #[test]
    fn a_deleted_file_is_a_removal_not_a_creation() {
        let dir = ws();
        std::fs::write(dir.path().join("gone.txt"), "bye\n").unwrap();
        let mut w = ChangeWatcher::new();
        w.begin(&serde_json::json!({"path": "gone.txt"}), dir.path());
        std::fs::remove_file(dir.path().join("gone.txt")).unwrap();
        let c = w.finish(dir.path());
        assert!(!c[0].created, "it existed a moment ago");
        assert_eq!(c[0].removed(), 1);
    }

    /// The **file** is watched, not the tool's name: whichever tool made the change, the
    /// pane reports the fact.
    #[test]
    fn a_change_is_reported_for_whichever_tool_actually_made_it() {
        let dir = ws();
        std::fs::write(dir.path().join("x.txt"), "before\n").unwrap();
        let mut w = ChangeWatcher::new();
        w.begin(&serde_json::json!({"path": "x.txt"}), dir.path());
        std::fs::write(dir.path().join("x.txt"), "after\n").unwrap();
        let c = w.finish(dir.path());
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].summary(), "x.txt · +1 −1");
    }

    /// The named gap, pinned: a `bash` call edits nothing the pane will report.
    ///
    /// Its arguments carry a `command`, not a path. Scraping a shell string for
    /// path-shaped tokens would put a filesystem probe on every tool call and would
    /// report files the user merely mentioned — and **a wrong diff is worse than no
    /// diff**, because it is a diff the reader has to check.
    #[test]
    fn a_shell_command_names_no_file_so_nothing_is_watched() {
        let dir = ws();
        std::fs::write(dir.path().join("x.txt"), "before\n").unwrap();
        let mut w = ChangeWatcher::new();
        w.begin(
            &serde_json::json!({ "command": "sed -i s/before/after/ x.txt" }),
            dir.path(),
        );
        assert!(!w.is_watching(), "a command string is not a path");
        std::fs::write(dir.path().join("x.txt"), "after\n").unwrap();
        assert!(
            w.finish(dir.path()).is_empty(),
            "and the change is not reported, which is the gap this test exists to name"
        );
    }

    #[test]
    fn a_path_that_escapes_the_workspace_is_not_watched() {
        let dir = ws();
        let mut w = ChangeWatcher::new();
        w.begin(
            &serde_json::json!({"path": "../../../etc/passwd"}),
            dir.path(),
        );
        assert!(
            !w.is_watching(),
            "the watcher must not read outside the workspace, whatever a tool call says"
        );
        assert!(w.finish(dir.path()).is_empty());
    }

    #[test]
    fn an_absolute_path_inside_the_workspace_is_accepted_and_named_relatively() {
        let dir = ws();
        let absolute = dir.path().join("src").join("a.rs");
        std::fs::create_dir_all(absolute.parent().unwrap()).unwrap();
        std::fs::write(&absolute, "x\n").unwrap();
        let mut w = ChangeWatcher::new();
        w.begin(
            &serde_json::json!({ "path": absolute.to_string_lossy() }),
            dir.path(),
        );
        std::fs::write(&absolute, "y\n").unwrap();
        let c = w.finish(dir.path());
        assert_eq!(c[0].path, "src/a.rs", "{}", c[0].path);
    }

    #[test]
    fn a_call_with_no_path_watches_nothing() {
        let dir = ws();
        let mut w = ChangeWatcher::new();
        w.begin(&serde_json::json!({"command": "ls -la"}), dir.path());
        assert!(!w.is_watching(), "a shell command names no file to watch");
        assert!(w.finish(dir.path()).is_empty());
    }

    #[test]
    fn beginning_a_new_call_forgets_the_previous_one() {
        // Otherwise two calls in a row would diff the second against the first's start,
        // and report a change that belongs to neither.
        let dir = ws();
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        let mut w = ChangeWatcher::new();
        w.begin(&serde_json::json!({"path": "a.txt"}), dir.path());
        w.begin(&serde_json::json!({"path": "a.txt"}), dir.path());
        assert!(w.finish(dir.path()).is_empty(), "the same snapshot twice");
    }
}
