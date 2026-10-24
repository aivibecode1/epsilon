//! Remembered per-path project trust.
//!
//! The implementation lives in `epsilon-runtime`, which owns the launch-time
//! trust decision as well. Keeping a second copy here was a real defect: the
//! two writers disagreed about corrupt files (one replaced, one refused) and
//! about whether keys were sorted, so the same `trust.json` could be published
//! two ways depending on which frontend answered first. `epsilon-tui` already
//! depends on `epsilon-runtime` for composition, so this module is a re-export,
//! not a second store.

pub use epsilon_runtime::{ProjectTrustDecision, ProjectTrustEntry, ProjectTrustStore};

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn store_in(dir: &Path) -> ProjectTrustStore {
        ProjectTrustStore::open_in(dir.join("trust.json"))
    }

    #[test]
    fn empty_store_asks_set_get_clear_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        let ws = tmp.path().join("ws");
        assert_eq!(s.get(&ws), None);
        s.set(&ws, Some(true)).unwrap();
        assert_eq!(s.get(&ws), Some(true));
        s.set(&ws, Some(false)).unwrap();
        assert_eq!(s.get(&ws), Some(false));
        s.clear(&ws).unwrap();
        assert_eq!(s.get(&ws), None);
    }

    #[test]
    fn nearest_ancestor_wins_and_child_overrides() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        let parent = tmp.path().join("repo");
        let child = parent.join("sub");
        s.set(&parent, Some(true)).unwrap();
        assert_eq!(s.get(&child), Some(true));
        let entry = s.get_entry(&child).unwrap();
        assert!(entry.path.ends_with("repo"));
        s.set(&child, Some(false)).unwrap();
        assert_eq!(s.get(&child), Some(false));
        // Clearing the child falls back to the parent.
        s.clear(&child).unwrap();
        assert_eq!(s.get(&child), Some(true));
    }

    #[test]
    fn set_many_is_atomic_and_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        let a = tmp.path().join("b-proj");
        let b = tmp.path().join("a-proj");
        s.set_many(&[(a.clone(), Some(true)), (b.clone(), Some(false))])
            .unwrap();
        assert_eq!(s.get(&a), Some(true));
        assert_eq!(s.get(&b), Some(false));
        let raw = std::fs::read_to_string(tmp.path().join("trust.json")).unwrap();
        let apos = raw.find("b-proj").unwrap();
        let bpos = raw.find("a-proj").unwrap();
        assert!(bpos < apos, "keys sorted on write: {raw}");
    }

    #[test]
    fn corrupt_store_reads_as_no_memory() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        let corrupt = "{bad json";
        std::fs::write(tmp.path().join("trust.json"), corrupt).unwrap();
        assert_eq!(s.get(&tmp.path().join("ws")), None);
        let refused = s
            .set(&tmp.path().join("ws"), Some(true))
            .expect_err("a corrupt store is not silently replaced");
        assert!(refused.contains("unreadable"), "{refused}");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("trust.json")).unwrap(),
            corrupt,
            "a refused write keeps every remembered denial"
        );
        std::fs::write(tmp.path().join("trust.json"), "{}").unwrap();
        s.set(&tmp.path().join("ws"), Some(true)).unwrap();
        assert_eq!(s.get(&tmp.path().join("ws")), Some(true));
    }
}
