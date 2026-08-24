//! Editor undo (M16 parity sweep) — pi's `undo-stack.ts` semantics on the
//! `(buffer, cursor)` pair: snapshot BEFORE each mutation, Ctrl+Z pops.

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

/// Bounded LIFO of [`EditorSnapshot`]s; duplicate-suppressing.
#[derive(Debug, Default)]
pub struct UndoStack {
    stack: Vec<EditorSnapshot>,
}

impl UndoStack {
    /// Empty stack.
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a snapshot; skipped when identical to the current top (keeps
    /// per-keystroke calls cheap without flooding the stack).
    pub fn push(&mut self, snap: EditorSnapshot) {
        if self.stack.last() == Some(&snap) {
            return;
        }
        self.stack.push(snap);
        if self.stack.len() > UNDO_MAX {
            let overflow = self.stack.len() - UNDO_MAX;
            self.stack.drain(..overflow);
        }
    }

    /// Pop the most recent snapshot, if any.
    pub fn pop(&mut self) -> Option<EditorSnapshot> {
        self.stack.pop()
    }

    /// Number of stored snapshots.
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

    #[test]
    fn push_pop_round_trips_and_dedupes() {
        let mut u = UndoStack::new();
        assert!(u.is_empty());

        u.push(EditorSnapshot {
            text: "a".into(),
            cursor_chars: 1,
        });
        u.push(EditorSnapshot {
            text: "ab".into(),
            cursor_chars: 2,
        });
        assert_eq!(u.len(), 2);

        // Identical top is suppressed.
        u.push(EditorSnapshot {
            text: "ab".into(),
            cursor_chars: 2,
        });
        assert_eq!(u.len(), 2);

        let top = u.pop().unwrap();
        assert_eq!((top.text.as_str(), top.cursor_chars), ("ab", 2));
        let prev = u.pop().unwrap();
        assert_eq!((prev.text.as_str(), prev.cursor_chars), ("a", 1));
        assert!(u.is_empty());
    }

    #[test]
    fn depth_capped_dropping_oldest() {
        let mut u = UndoStack::new();
        for i in 0..UNDO_MAX + 25 {
            u.push(EditorSnapshot {
                text: format!("s{i}"),
                cursor_chars: 0,
            });
        }
        assert_eq!(u.len(), UNDO_MAX);
        // Newest survives, oldest gone.
        let top = u.pop().unwrap();
        assert_eq!(top.text, format!("s{}", UNDO_MAX + 24));
    }
}
