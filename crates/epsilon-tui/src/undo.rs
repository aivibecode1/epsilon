//! Editor undo/redo — pi's `undo-stack.ts` semantics (M16) plus an explicit redo
//! rail (M17b, opencode `input_redo`): snapshots taken BEFORE each mutation;
//! Ctrl+Z pops undo, Ctrl+. re-applies from the redo rail.

/// Maximum undo depth (oldest dropped).
const UNDO_MAX: usize = 200;

/// One point-in-time editor state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorSnapshot {
    /// Buffer content at snapshot time.
    pub text: String,
    /// Cursor as a char index into `text`.
    pub cursor_chars: usize,
}

/// Bounded LIFO of [`EditorSnapshot`]s with duplicate-suppression and a redo rail.
#[derive(Debug, Default)]
pub struct UndoStack {
    stack: Vec<EditorSnapshot>,
    redo: Vec<EditorSnapshot>,
}

impl UndoStack {
    /// Empty stack.
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a snapshot; skipped when identical to the current top (keeps
    /// per-keystroke calls cheap without flooding the stack). Any NEW push
    /// invalidates the redo rail (standard editor semantics: branch diverges).
    pub fn push(&mut self, snap: EditorSnapshot) {
        if self.stack.last() == Some(&snap) {
            return;
        }
        self.redo.clear();
        self.stack.push(snap);
        if self.stack.len() > UNDO_MAX {
            let overflow = self.stack.len() - UNDO_MAX;
            self.stack.drain(..overflow);
        }
    }

    /// Undo: pop the most recent pre-mutation snapshot, recording `current`
    /// (the live state being LEFT) on the redo rail so Ctrl+. can restore it.
    pub fn undo(&mut self, current: EditorSnapshot) -> Option<EditorSnapshot> {
        let snap = self.stack.pop()?;
        self.redo.push(current);
        Some(snap)
    }

    /// Redo: re-apply the most recently undone state; `current` (the live state
    /// being left) returns to the undo path so Ctrl+Z can toggle back.
    pub fn redo(&mut self, current: EditorSnapshot) -> Option<EditorSnapshot> {
        let snap = self.redo.pop()?;
        self.stack.push(current);
        Some(snap)
    }

    /// Number of stored undo snapshots.
    pub fn len(&self) -> usize {
        self.stack.len()
    }

    /// True when nothing to undo.
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(text: &str, cursor: usize) -> EditorSnapshot {
        EditorSnapshot {
            text: text.into(),
            cursor_chars: cursor,
        }
    }

    #[test]
    fn push_undo_round_trips_and_dedupes() {
        let mut u = UndoStack::new();
        assert!(u.is_empty());

        u.push(snap("a", 1));
        u.push(snap("ab", 2));
        assert_eq!(u.len(), 2);

        // Identical top is suppressed.
        u.push(snap("ab", 2));
        assert_eq!(u.len(), 2);

        let restored = u.undo(snap("abc", 3)).unwrap();
        assert_eq!((restored.text.as_str(), restored.cursor_chars), ("ab", 2));
        let prev = u.undo(snap("ab", 2)).unwrap();
        assert_eq!((prev.text.as_str(), prev.cursor_chars), ("a", 1));
        assert!(u.is_empty());
    }

    #[test]
    fn depth_capped_dropping_oldest() {
        let mut u = UndoStack::new();
        for i in 0..UNDO_MAX + 25 {
            u.push(snap(&format!("s{i}"), 0));
        }
        assert_eq!(u.len(), UNDO_MAX);
        // Newest survives, oldest gone.
        let top = u.undo(snap("live", 0)).unwrap();
        assert_eq!(top.text, format!("s{}", UNDO_MAX + 24));
    }

    /// M17b: undo records the live state on the redo rail; redo swaps it back;
    /// a fresh push after an undo clears the rail (branch divergence).
    #[test]
    fn redo_rail_semantics() {
        let mut u = UndoStack::new();
        u.push(snap("a", 1));
        u.push(snap("ab", 2));

        // Live state "abc" → undo restores "ab".
        let restored = u.undo(snap("abc", 3)).expect("undo");
        assert_eq!(restored.text, "ab");

        // Redo from "ab" returns "abc".
        let redone = u.redo(snap("ab", 2)).expect("redo");
        assert_eq!(redone.text, "abc");
        assert!(u.redo(snap("abc", 3)).is_none(), "rail drained");

        // Undo again then diverge with a fresh push: rail clears.
        let _ = u.undo(snap("abc", 3));
        u.push(snap("xyz", 3));
        assert!(
            u.redo(snap("xyz", 3)).is_none(),
            "divergent push clears the rail"
        );
    }
}
