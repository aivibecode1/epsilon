//! Editing the composer line: characters, cursor, kill, yank, undo, redo and history.
//!
//! ## Why this file exists
//!
//! Nineteen methods that answer "what is in the editor, and what did the last key do to
//! it". They are the readline-shaped part of the TUI, and they are a *layer*: every one of
//! them is a primitive that composes the ones above it (`backspace` uses `byte_cursor_at`,
//! `yank` uses `stash_draft`), which is why the order in this file is meaningful and why
//! moving them together keeps the dependency direction readable.
//!
//! The two `*_tracked` wrappers are the seam worth knowing about: the key handler calls
//! those, and they apply the plain primitive and then re-derive the slash-popup state, so
//! no caller can forget to do the second half.
//!
//! ## The rule for this move
//!
//! Sliced by line range, boundaries anchored on named methods. Bodies are byte-identical to
//! `app.rs` at the parent commit apart from the `pub(super)` markers.

use super::*;

impl App {
    /// Insert a character, or — when a jump is armed — MOVE the cursor instead.
    ///
    /// M196: this is the composer's single insert path, so arming a jump here
    /// means every route into "the user typed a printable character" is covered by
    /// one check, rather than each key route remembering to look.
    ///
    /// The mode is consumed either way. Leaving it armed after a printable key
    /// would mean the NEXT letter the user typed moved the cursor instead of
    /// being typed — the most confusing failure this feature could have, and the
    /// one pi avoids by clearing `jumpMode` before dispatching
    /// (`editor.ts:697-700`).
    pub(super) fn insert_char(&mut self, c: char) {
        if let Some(dir) = self.jump_mode.take() {
            match crate::app::jump_to_char(&self.input, self.cursor_chars, c, dir) {
                Some(at) => {
                    self.cursor_chars = at;
                }
                None => {
                    // Not found: say so, and leave the cursor alone. Inserting
                    // the character instead would be a surprising way to learn
                    // there was no match.
                    self.ui.push_notice(format!(
                        "no '{c}' {}",
                        match dir {
                            crate::app::JumpDir::Forward => "after",
                            crate::app::JumpDir::Backward => "before",
                        }
                    ));
                }
            }
            return;
        }
        let byte_at = self.byte_cursor();
        self.input.insert(byte_at, c);
        self.cursor_chars += 1;
    }

    /// Arm a jump-to-character, or disarm it if one is already armed.
    ///
    /// The toggle is what makes the feature usable without a visible mode line:
    /// pressing the chord twice means "never mind" without having to remember
    /// which key cancels.
    pub(super) fn toggle_jump(&mut self, dir: crate::app::JumpDir) {
        self.jump_mode = match self.jump_mode {
            Some(d) if d == dir => None,
            Some(_) => Some(dir),
            None => Some(dir),
        };
        if let Some(d) = self.jump_mode {
            let what = match d {
                crate::app::JumpDir::Forward => "forward",
                crate::app::JumpDir::Backward => "backward",
            };
            self.ui.push_notice(format!(
                "jump {what}: type a character, Esc or the same chord cancels"
            ));
        }
    }

    pub(super) fn backspace(&mut self) {
        // M36: erase a whole grapheme cluster — a bare `char` step would split
        // ZWJ families, flags, and combining marks.
        let start = crate::editor_ops::prev_cluster_start(&self.input, self.cursor_chars);
        if start < self.cursor_chars {
            let byte_start = self.byte_cursor_at(start);
            let byte_end = self.byte_cursor();
            self.input.replace_range(byte_start..byte_end, "");
            self.cursor_chars = start;
        }
    }

    /// Kill from cursor to start (Ctrl+U); pushes onto the kill ring.
    pub(super) fn kill_to_start(&mut self) {
        self.push_undo();
        let killed: String = self.input.chars().take(self.cursor_chars).collect();
        let (s, c) = crate::editor_ops::kill_to_start(&self.input, self.cursor_chars);
        // A fresh kill starts a new entry; consecutive kills extend the last one.
        let contiguous = self.last_kill_was_contiguous;
        self.kill_ring.push(killed, contiguous, false);
        self.last_kill_was_contiguous = true;
        self.input = s;
        self.cursor_chars = c;
        self.end_yank();
    }

    /// Kill the word before the cursor (Ctrl+W); pushes/accumulates on the ring.
    /// Paste markers count as ONE unit.
    pub(super) fn kill_word_back(&mut self) {
        self.push_undo();
        let start = crate::editor_ops::word_left_atomic(&self.input, self.cursor_chars);
        let chars: Vec<char> = self.input.chars().collect();
        let killed: String = chars[start..self.cursor_chars].iter().collect();
        let kept: String = chars[..start]
            .iter()
            .chain(chars[self.cursor_chars..].iter())
            .collect();
        let contiguous = self.last_kill_was_contiguous;
        // Backward kills prepend when accumulating (Emacs rule).
        self.kill_ring.push(killed, contiguous, true);
        self.last_kill_was_contiguous = true;
        self.input = kept;
        self.cursor_chars = start;
        self.end_yank();
    }

    /// Yank (Ctrl+Y): insert the top of the kill ring; repeated presses cycle
    /// through older entries, replacing the previously yanked text (yank-pop).
    pub(super) fn yank(&mut self) {
        if self.kill_ring.is_empty() {
            return;
        }
        if self.yank_in_progress
            && let Some(prev_len) = self.yanked_len.take()
        {
            // Delete the previously yanked span behind the cursor.
            let del_start_char = self.cursor_chars - prev_len;
            let byte_start = self.byte_cursor_at(del_start_char);
            let byte_end = self.byte_cursor_at(self.cursor_chars);
            self.input.replace_range(byte_start..byte_end, "");
            self.cursor_chars = del_start_char;
            self.kill_ring.rotate();
        }
        let text = self.kill_ring.peek().cloned().unwrap_or_default();
        let n = text.chars().count();
        self.insert_str(&text);
        self.yanked_len = Some(n);
        self.yank_in_progress = n > 0;
    }

    /// Any non-yank edit ends a yank sequence.
    pub(super) fn end_yank(&mut self) {
        self.yank_in_progress = false;
        self.yanked_len = None;
    }

    /// Stash the current draft (opencode prompt.stash): called when a dialog opens.
    /// The editor becomes the dialog's filter box.
    pub(super) fn stash_draft(&mut self) {
        if !self.input.is_empty() {
            self.stash = Some((std::mem::take(&mut self.input), self.cursor_chars));
            self.cursor_chars = 0;
        }
    }

    /// Restore the stashed draft (opencode onCleanup/onMount pair).
    pub(super) fn unstash_draft(&mut self) {
        if let Some((text, cursor)) = self.stash.take() {
            self.input = text;
            self.cursor_chars = cursor;
        }
    }

    /// Snapshot the current state BEFORE a mutation (parity sweep: Ctrl+Z).
    pub(super) fn push_undo(&mut self) {
        self.undo.push(crate::undo::EditorSnapshot {
            text: self.input.clone(),
            cursor_chars: self.cursor_chars,
        });
    }

    /// Ctrl+Z: restore the most recent pre-mutation snapshot.
    pub(super) fn undo(&mut self) -> bool {
        let current = crate::undo::EditorSnapshot {
            text: self.input.clone(),
            cursor_chars: self.cursor_chars,
        };
        match self.undo.undo(current) {
            Some(snap) => {
                self.input = snap.text;
                self.cursor_chars = snap.cursor_chars.min(self.input.chars().count());
                self.end_yank();
                self.sync_slash_state();
                true
            }
            None => false,
        }
    }

    /// M17b: Ctrl+. re-applies the most recently undone edit (opencode input_redo).
    pub(super) fn redo(&mut self) -> bool {
        let current = crate::undo::EditorSnapshot {
            text: self.input.clone(),
            cursor_chars: self.cursor_chars,
        };
        match self.undo.redo(current) {
            Some(snap) => {
                self.input = snap.text;
                self.cursor_chars = snap.cursor_chars.min(self.input.chars().count());
                self.end_yank();
                self.sync_slash_state();
                true
            }
            None => false,
        }
    }

    pub(super) fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            self.insert_char(c);
        }
    }

    pub(super) fn byte_cursor_at(&self, char_idx: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_idx)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len())
    }

    pub(super) fn byte_cursor(&self) -> usize {
        self.input
            .char_indices()
            .nth(self.cursor_chars)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len())
    }

    pub(super) fn remember_history(&mut self, text: String) {
        if text.trim().is_empty() || self.history.back().is_some_and(|h| *h == text) {
            return;
        }
        self.history.push_back(text);
        if self.history.len() > HISTORY_MAX {
            self.history.pop_front();
        }
        self.history_idx = None;
    }

    pub(super) fn recall(&mut self, older: bool) {
        if self.history.is_empty() {
            return;
        }
        let len = self.history.len();
        let idx = match self.history_idx {
            None => {
                if !older {
                    return;
                }
                len - 1
            }
            Some(i) => {
                let next = if older { i.saturating_sub(1) } else { i + 1 };
                if next >= len {
                    // Past newest entry: restore empty editor.
                    self.history_idx = None;
                    self.input.clear();
                    self.cursor_chars = 0;
                    return;
                }
                next
            }
        };
        self.history_idx = Some(idx);
        self.input = self.history[idx].clone();
        self.cursor_chars = self.input.chars().count();
    }

    /// Flip tool-output expansion, announcing what changed.
    ///
    /// Extracted so the `app.tools.expand` key and the command palette do the *same*
    /// thing. The palette entry used to carry a `\u{1}expand` pseudo-command that
    /// `parse_command` cannot read (it requires a leading `/`), so choosing it closed
    /// the palette, cleared the composer, and ran nothing — with no notice at all.
    /// A menu entry that silently does nothing is worse than a missing one, because
    /// the user is told the feature exists.
    pub(super) fn toggle_tool_expansion(&mut self) {
        let was_open = self
            .selected_card_id()
            .map(|id| self.card_is_open(id, true));
        self.prefs.expand_outputs = !self.prefs.expand_outputs;
        let label = match (self.selected_card_id(), was_open) {
            (Some(id), Some(was_open)) => {
                // What the card's own entry records, and why it is not simply
                // `!was_open` (which is what this used to be, and what made the
                // second press close the card the first press had opened):
                //
                // The action is an *expand*, and the global toggle is what it flips.
                // So after the press the selected card is open if the global is now
                // on, or if it was already open. Reading it as a per-card toggle
                // instead meant "expand all, then collapse all" also collapsed the
                // one card the user had deliberately opened — the opposite of the
                // exception this whole path exists to provide (see the frame test in
                // `app/golden.rs` that asserts the card is still open with the
                // global off).
                let open = self.prefs.expand_outputs || was_open;
                self.set_card_open(id, open);
                if open {
                    "card expanded"
                } else {
                    "card collapsed"
                }
            }
            _ if self.prefs.expand_outputs => "all cards expanded",
            _ => "all cards collapsed",
        };
        // The footer notice only. A palette *command* adds its own transcript line
        // (see the `Expand` arm in `app/commands.rs`): the key path must leave the
        // frame byte-identical, and `app::golden::ctrl_x_opens_the_newest_card…`
        // asserts exactly that — a notice row here moved the card it is about.
        self.notice(format!("{label} (Ctrl+X toggles)"));
    }

    /// Empty the composer, as pi's `app.clear` ("Clear editor", ctrl+c) does.
    ///
    /// The binding existed in the defaults table and was read by nothing, while
    /// `app.exit` consumed the same chord first — so Ctrl+C quit the process with the
    /// draft still in the buffer, and `/hotkeys` advertised a binding that did nothing.
    /// A keybinding table the UI prints is a promise; this is the promise being kept.
    ///
    /// Cursor and history index move with the text. Leaving the cursor past the end of
    /// an empty string would make the next character land somewhere the user did not
    /// type, which is the "no input" symptom all over again.
    pub(super) fn clear_editor(&mut self) {
        self.input.clear();
        self.cursor_chars = 0;
        self.history_idx = None;
        self.sync_slash_state();
    }

    pub(super) fn submit_text(&mut self, text: String) -> Message {
        self.remember_history(text.clone());
        // M16b: persist to the global history file (best-effort; never blocks).
        if let Err(e) = epsilon_core::config::append_prompt_history(&text) {
            self.notice(format!("history not saved: {e}"));
        }
        self.ui.push_local_input(&text);
        // M74: `@` references attach content through the shared constructor — the same one `-p`,
        // the RPC loop and the server use. The message keeps the user's text verbatim.
        epsilon_core::references::user_message(&self.workspace_root, &text)
    }

    /// Insert text and refresh the slash-popup state (open/close + index reset).
    pub(super) fn insert_char_tracked(&mut self, c: char) {
        self.insert_char(c);
        self.sync_slash_state();
    }

    /// Backspace and refresh the slash-popup state.
    pub(super) fn backspace_tracked(&mut self) {
        self.backspace();
        self.sync_slash_state();
    }
}
