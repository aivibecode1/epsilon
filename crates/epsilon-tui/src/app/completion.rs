//! Slash-command and `@file` completion: what the popups offer, and what happens when the
//! user takes one.
//!
//! ## Why this file exists
//!
//! Five methods, and the invariant that makes them one: **the popup state is derived from
//! the editor buffer, never remembered**. `sync_slash_state` and `sync_at_state` recompute
//! open/index from `self.input`; they do not patch it. That is why every editing method can
//! call them freely, and why a stale popup cannot survive a keypress that did not go
//! through this file.
//!
//! ## The rule for this move
//!
//! Sliced by line range, boundaries anchored on named methods. Bodies are byte-identical to
//! `app.rs` at the parent commit apart from the `pub(super)` markers.

use super::*;

impl App {
    /// Re-derive slash popup open/index from the current editor buffer
    /// (pi: autocomplete state follows the text, never lingers).
    pub(super) fn sync_slash_state(&mut self) {
        let candidates = crate::picker::slash_items(&self.input);
        if candidates.is_empty() {
            if self.slash_open {
                self.slash_open = false;
            }
            self.slash_index = 0;
        } else {
            if !self.slash_open {
                self.slash_open = true;
            }
            if self.slash_index >= candidates.len() {
                self.slash_index = 0;
            }
        }
        self.sync_at_state();
    }

    /// Re-derive the @-file popup from the cursor-adjacent @-word (W1, pi editor.ts).
    pub(super) fn sync_at_state(&mut self) {
        if let Some((_, frag)) = crate::editor_ops::at_word_before(&self.input, self.cursor_chars) {
            if self.at_fragment != frag || self.at_rows.is_none() {
                self.at_fragment = frag.clone();
                let rows = self.at_file_rows(&frag);
                self.at_rows = Some(rows);
                self.at_index = 0;
            } else if let Some(rows) = &self.at_rows
                && self.at_index >= rows.len().max(1)
            {
                self.at_index = 0;
            }
            let has_rows = self
                .at_rows
                .as_ref()
                .map(|r| !r.is_empty())
                .unwrap_or(false);
            self.at_open = has_rows;
            if !has_rows {
                self.at_index = 0;
            }
        } else {
            self.at_open = false;
            self.at_index = 0;
            self.at_rows = None;
            self.at_fragment.clear();
        }
    }

    /// File candidates for `fragment` (case-insensitive substring, cap 200).
    pub(super) fn at_file_rows(&mut self, fragment: &str) -> Vec<String> {
        // The `@` list has to agree with the tools about which files exist: offering a path
        // the model's own read tool would refuse is a promise the harness cannot keep.
        // `epsilon_core::ignore` is that single answer (M99), and `epsilon-tui` already
        // depends on `epsilon-core`, so sharing it costs no new crate edge.
        //
        // What is still this module's own is the *traversal*, deliberately: it is budgeted
        // for typing latency (a few hundred rows, stop early) rather than for a tool call
        // that must see everything. One ignore answer, two walks with different budgets —
        // if a third consumer appears, the walk belongs in core beside `IgnoreSet`.
        //
        // **The file list is cached, not just the ignore rules** (M194).
        //
        // This function runs once per keystroke — the caller re-derives whenever the
        // `@`-fragment changes, so every character typed after an `@` lands here. The
        // rules were cached but the *walk* was not, so each of those keystrokes
        // re-traversed the workspace to produce a list that cannot have changed between
        // two characters of the same word.
        //
        // Measured on this machine by mirroring `at_files::walk_inner`: **4.2 ms** in this
        // repository (847 files), **9.2 ms** against a large home tree — 28% of a 33 ms
        // frame at 30 fps. `AT_ROW_BUDGET` bounds it, so it was never a freeze, and this
        // is not offered as the cause of one. It is a cost paid on every keystroke for an
        // answer that is identical to the previous keystroke's, which is waste whatever
        // its size.
        //
        // The cache expires on the same 5-second TTL as the rules, so the list is at most
        // as stale as the rules that shaped it — a file created while the popup is open
        // appears on the next `@`, not instantly, which is the same bargain the rules
        // already make and the one the branch cache pays.
        const AT_IGNORE_TTL: std::time::Duration = std::time::Duration::from_secs(5);
        let fresh = matches!(&self.at_ignore, Some((_, at, _)) if at.elapsed() <= AT_IGNORE_TTL);
        if !fresh {
            let ignore = epsilon_core::ignore::IgnoreSet::load(
                &self.workspace_root,
                crate::at_files::AT_IGNORE_FILES,
            );
            let mut budget = crate::at_files::AT_ROW_BUDGET;
            let files = crate::at_files::walk(&self.workspace_root, &ignore, &mut budget);
            self.at_ignore = Some((ignore, std::time::Instant::now(), files));
        }
        // Cloned because the fuzzy filter below takes ownership for sorting. A 200-row
        // `Vec<String>` is a few kilobytes, which is the point: the walk behind it costs
        // milliseconds, and this costs microseconds.
        let files = self
            .at_ignore
            .as_ref()
            .expect("the cache was just filled")
            .2
            .clone();
        let needle = fragment.trim();
        if needle.is_empty() {
            let mut out = files;
            out.sort();
            out.truncate(crate::at_files::AT_ROW_BUDGET);
            return out;
        }
        // pi `fuzzyFilter` on full paths — same semantics as model picker.
        let out: Vec<String> = crate::fuzzy::fuzzy_filter(&files, needle, |p| p.clone());
        let mut out = out;
        out.truncate(200);
        out
    }

    /// Insert the highlighted @-file path (pi: Tab inserts path, Enter inserts path+space).
    pub(super) fn apply_at_selection(&mut self, add_space: bool) {
        let Some(rows) = self.at_rows.clone() else {
            return;
        };
        let Some(path) = rows.get(self.at_index).cloned() else {
            return;
        };
        let suffix = if add_space { format!("{path} ") } else { path };
        let (s, c) = crate::editor_ops::at_insert(&self.input, self.cursor_chars, &suffix);
        self.push_undo();
        self.input = s;
        self.cursor_chars = c;
        self.at_open = false;
        self.at_index = 0;
        self.at_rows = None;
        self.at_fragment.clear();
        self.end_yank();
        self.sync_slash_state();
    }

    /// Apply the currently highlighted slash candidate (pi `applyCompletion`):
    /// replace the whole editor with the item's value, cursor at end.
    pub(super) fn apply_slash_selection(&mut self) {
        let candidates = crate::picker::slash_items(&self.input);
        if let Some(item) = candidates.get(self.slash_index) {
            self.input = item.value.to_string();
            self.cursor_chars = self.input.chars().count();
            self.end_yank();
        }
        // After applying, "/model " keeps the popup closed (argument typing next);
        // bare commands close it too — pi closes the menu on selection.
        self.slash_open = false;
        self.slash_index = 0;
    }
}
