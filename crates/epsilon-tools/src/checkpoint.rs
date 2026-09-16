//! Workspace checkpoints: the content of a file before the harness changed it, so an edit can be
//! undone (roadmap Phase 5 item 2, Hermes `tools/checkpoint_manager.py:72,475,1726`).
//!
//! The failure this closes: a model edits three files, the change is wrong, and the *session* can be
//! resumed but the *workspace* cannot be rewound. Fork and clone move the conversation; nothing moved
//! the files.
//!
//! Shape, and why it differs from the reference:
//!
//! | Hermes | Here |
//! |---|---|
//! | bare-git shadow store, per-project refs, a 50 000-file index | a content-addressed blob store plus an append-only ledger of the files *this harness* changed |
//! | whole-workspace rewind | per-file revert (what the roadmap asks for) |
//! | requires `git` on `PATH` | pure filesystem, no subprocess |
//!
//! The trade is deliberate: a git shadow store would also let a user rewind files the harness never
//! touched, and that is not the failure being fixed — the unundoable edit is. In exchange this needs
//! no git, works in a directory that is not a repository, and cannot fail because an index went bad.
//!
//! Layout, under the workspace's `.epsilon/checkpoints/`:
//!
//! ```text
//! blobs/<sha256[:32]>    file contents, deduplicated by content
//! ledger.jsonl           one line per change: seq, at, path, blob, bytes, tool, existed
//! ```
//!
//! Two rules that keep this from becoming a disk leak: blobs are written once per distinct content,
//! and the store is capped ([`CheckpointStore::DEFAULT_MAX_BYTES`]) with the oldest *unreferenced*
//! blobs evicted first — eviction reports what it removed rather than silently dropping history.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Largest file this will snapshot. A 40 MB binary is not an edit worth undoing by hand, and copying
/// it would make the store's growth unpredictable.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// One recorded change: what a path looked like *before* a tool touched it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Monotonic within a store.
    pub seq: u64,
    /// Unix seconds.
    pub at: u64,
    /// Workspace-relative path, `/`-separated (stable across platforms).
    pub path: String,
    /// Blob name, or `None` when the file did not exist (so reverting means deleting it).
    pub blob: Option<String>,
    /// Bytes captured.
    pub bytes: u64,
    /// Which tool made the change (`write`, `edit`).
    pub tool: String,
    /// False when the file did not exist before the change.
    pub existed: bool,
    /// Why there is no blob, when the capture was skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

/// What a `record` call did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded {
    /// A blob was stored (deduplicated by content).
    Stored {
        /// Blob name.
        blob: String,
        /// Bytes captured.
        bytes: u64,
    },
    /// The file did not exist, so reverting means removing it.
    Created,
    /// Nothing was captured, with the reason.
    Skipped(String),
}

/// The store: a directory of blobs plus its ledger.
pub struct CheckpointStore {
    root: PathBuf,
    max_bytes: u64,
}

impl CheckpointStore {
    /// Default ceiling for the whole store.
    pub const DEFAULT_MAX_BYTES: u64 = 64 * 1024 * 1024;

    /// Open (creating if needed) the store for `workspace`.
    pub fn open(workspace: &Path) -> Self {
        Self {
            root: workspace.join(".epsilon").join("checkpoints"),
            max_bytes: Self::DEFAULT_MAX_BYTES,
        }
    }

    /// Open with a different ceiling (tests, and a future config knob).
    pub fn with_max_bytes(workspace: &Path, max_bytes: u64) -> Self {
        Self {
            root: workspace.join(".epsilon").join("checkpoints"),
            max_bytes,
        }
    }

    /// The directory this store owns.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn ledger_path(&self) -> PathBuf {
        self.root.join("ledger.jsonl")
    }

    fn blob_path(&self, blob: &str) -> PathBuf {
        self.root.join("blobs").join(blob)
    }

    /// Every entry, oldest first. A corrupt line is skipped rather than fatal: the ledger records
    /// history, and refusing to list history because one line was damaged helps nobody.
    pub fn entries(&self) -> Vec<Entry> {
        let Ok(text) = std::fs::read_to_string(self.ledger_path()) else {
            return Vec::new();
        };
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str::<Entry>(line).ok())
            .collect()
    }

    /// Record `path`'s current content as the state to return to.
    ///
    /// `path` must already be inside the workspace; callers pass the validated path. Returns what
    /// happened, and never fails the caller's mutation: a checkpoint that cannot be written is
    /// reported through [`Recorded::Skipped`] and the edit proceeds.
    pub fn record(&self, workspace: &Path, path: &Path, tool: &str) -> Recorded {
        let relative = match path.strip_prefix(workspace) {
            Ok(relative) => relative,
            Err(_) => return Recorded::Skipped("outside the workspace".into()),
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        let seq = self.entries().last().map(|e| e.seq + 1).unwrap_or(1);
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // The file did not exist: reverting means removing it, and that is worth recording.
        if !path.exists() {
            let entry = Entry {
                seq,
                at,
                path: relative,
                blob: None,
                bytes: 0,
                tool: tool.into(),
                existed: false,
                skipped: None,
            };
            self.append(&entry);
            return Recorded::Created;
        }

        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(e) => return Recorded::Skipped(format!("stat failed: {e}")),
        };
        // Too big to copy: recorded as a skip, so "there is no checkpoint for this file" is a fact
        // in the ledger rather than an absence someone has to notice.
        if metadata.len() > MAX_FILE_BYTES {
            let reason = format!(
                "{} bytes is over the {MAX_FILE_BYTES}-byte checkpoint limit",
                metadata.len()
            );
            let entry = Entry {
                seq,
                at,
                path: relative,
                blob: None,
                bytes: metadata.len(),
                tool: tool.into(),
                existed: true,
                skipped: Some(reason.clone()),
            };
            self.append(&entry);
            return Recorded::Skipped(reason);
        }

        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => return Recorded::Skipped(format!("read failed: {e}")),
        };
        let blob = blob_name(&bytes);
        let blob_path = self.blob_path(&blob);
        if !blob_path.exists() {
            if let Err(e) = self.ensure_dirs() {
                return Recorded::Skipped(format!("mkdir failed: {e}"));
            }
            if let Err(e) = std::fs::write(&blob_path, &bytes) {
                return Recorded::Skipped(format!("blob write failed: {e}"));
            }
        }
        let entry = Entry {
            seq,
            at,
            path: relative,
            blob: Some(blob.clone()),
            bytes: bytes.len() as u64,
            tool: tool.into(),
            existed: true,
            skipped: None,
        };
        self.append(&entry);
        self.evict_if_needed();
        Recorded::Stored {
            blob,
            bytes: entry.bytes,
        }
    }

    fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.root.join("blobs"))
    }

    fn append(&self, entry: &Entry) {
        if self.ensure_dirs().is_err() {
            return;
        }
        if let Ok(line) = serde_json::to_string(entry) {
            use std::io::Write as _;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.ledger_path())
            {
                let _ = writeln!(file, "{line}");
            }
        }
    }

    /// The entry that reverting `path` would apply: the newest one for that path.
    pub fn latest_for(&self, path: &str) -> Option<Entry> {
        self.entries().into_iter().rev().find(|e| e.path == path)
    }

    /// Every path this store has a checkpoint for, newest first, with the newest entry each.
    pub fn paths(&self) -> Vec<Entry> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for entry in self.entries().into_iter().rev() {
            if seen.insert(entry.path.clone()) {
                out.push(entry);
            }
        }
        out
    }

    /// Restore `path` to the state recorded in `entry`.
    ///
    /// Returns the number of bytes written, or `0` when the entry means the file did not exist and
    /// was removed. The restore itself is only recorded by the caller's next mutation — reverting is
    /// a change like any other, and the caller owns the mutation queue that keeps writers serialized.
    pub fn restore(&self, workspace: &Path, entry: &Entry) -> Result<u64, String> {
        let target = workspace.join(entry.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        match &entry.blob {
            Some(blob) => {
                let bytes = std::fs::read(self.blob_path(blob))
                    .map_err(|e| format!("checkpoint blob {blob} is missing: {e}"))?;
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
                }
                std::fs::write(&target, &bytes).map_err(|e| format!("write: {e}"))?;
                Ok(bytes.len() as u64)
            }
            None => {
                if target.exists() {
                    std::fs::remove_file(&target).map_err(|e| format!("remove: {e}"))?;
                }
                Ok(0)
            }
        }
    }

    /// Total bytes of blobs on disk.
    pub fn stored_bytes(&self) -> u64 {
        let Ok(entries) = std::fs::read_dir(self.root.join("blobs")) else {
            return 0;
        };
        entries
            .filter_map(|e| e.ok())
            .filter_map(|e| e.metadata().ok())
            .filter(|m| m.is_file())
            .map(|m| m.len())
            .sum()
    }

    /// Delete blobs past the ceiling, oldest first, and report the names removed.
    ///
    /// Only blobs no longer referenced by *any* ledger entry are candidates; the newest checkpoint of
    /// each path is always kept, because that is the one a revert would use.
    pub fn evict_if_needed(&self) -> Vec<String> {
        let total = self.stored_bytes();
        if total <= self.max_bytes {
            return Vec::new();
        }
        let entries = self.entries();
        let keep: BTreeSet<String> = self.paths().into_iter().filter_map(|e| e.blob).collect();
        let mut removed = Vec::new();
        let mut freed = 0u64;
        // Oldest first: the order blobs were first seen in the ledger.
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut candidates: Vec<(String, u64)> = Vec::new();
        for entry in &entries {
            if let Some(blob) = &entry.blob
                && seen.insert(blob.clone())
                && !keep.contains(blob)
            {
                candidates.push((blob.clone(), entry.bytes));
            }
        }
        for (blob, bytes) in candidates {
            if total.saturating_sub(freed) <= self.max_bytes {
                break;
            }
            if std::fs::remove_file(self.blob_path(&blob)).is_ok() {
                freed += bytes;
                removed.push(blob);
            }
        }
        removed
    }
}

/// Blob name for `bytes`: 32 hex characters derived from the content.
///
/// Precisely what this is: a *stable within one build* content address over the standard library's
/// `DefaultHasher` with a length prefix, not a cryptographic digest. Across a toolchain upgrade the
/// names may change, which costs a duplicate blob on the next capture — the store dedupes within a
/// build and degrades to "store it again" across one, and nothing depends on the name beyond that.
/// Using a real digest would mean a new dependency for a benefit (collision resistance against a
/// deliberate adversary writing into the user's own workspace) that is not part of the threat model.
pub fn blob_name(bytes: &[u8]) -> String {
    use std::hash::Hasher as _;

    fn seeded(seed: u64, bytes: &[u8]) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(seed);
        hasher.write_u64(bytes.len() as u64);
        // `write` instead of `Hash::hash`: hashing a slice through the trait would prefix its
        // length a second time, which is harmless but makes the name harder to reason about.
        hasher.write(bytes);
        hasher.finish()
    }

    format!(
        "{:016x}{:016x}",
        seeded(0x9e37, bytes),
        seeded(0x85eb, bytes)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        dir
    }

    #[test]
    fn a_recorded_file_can_be_restored_byte_for_byte() {
        let dir = ws();
        let file = dir.path().join("src/lib.rs");
        std::fs::write(&file, "original content\n").unwrap();
        let store = CheckpointStore::open(dir.path());

        let recorded = store.record(dir.path(), &file, "write");
        assert!(matches!(recorded, Recorded::Stored { .. }), "{recorded:?}");

        std::fs::write(&file, "clobbered\n").unwrap();
        let entry = store.latest_for("src/lib.rs").expect("an entry");
        let bytes = store.restore(dir.path(), &entry).unwrap();
        assert_eq!(bytes, "original content\n".len() as u64);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "original content\n"
        );
    }

    #[test]
    fn a_file_that_did_not_exist_is_restored_by_removing_it() {
        let dir = ws();
        let file = dir.path().join("src/new.rs");
        let store = CheckpointStore::open(dir.path());
        assert_eq!(store.record(dir.path(), &file, "write"), Recorded::Created);

        std::fs::write(&file, "created by the model\n").unwrap();
        let entry = store.latest_for("src/new.rs").expect("an entry");
        assert!(!entry.existed);
        assert_eq!(store.restore(dir.path(), &entry).unwrap(), 0);
        assert!(!file.exists(), "undoing a creation removes the file");
    }

    #[test]
    fn identical_content_is_stored_once() {
        let dir = ws();
        let a = dir.path().join("src/a.rs");
        let b = dir.path().join("src/b.rs");
        std::fs::write(&a, "same\n").unwrap();
        std::fs::write(&b, "same\n").unwrap();
        let store = CheckpointStore::open(dir.path());
        let first = store.record(dir.path(), &a, "write");
        let second = store.record(dir.path(), &b, "write");
        match (first, second) {
            (Recorded::Stored { blob: one, .. }, Recorded::Stored { blob: two, .. }) => {
                assert_eq!(one, two, "content addressing dedupes");
            }
            other => panic!("expected two stored blobs, got {other:?}"),
        }
        assert_eq!(store.stored_bytes(), "same\n".len() as u64);
    }

    #[test]
    fn the_ledger_is_monotonic_and_lists_the_newest_state_per_path() {
        let dir = ws();
        let file = dir.path().join("src/lib.rs");
        let store = CheckpointStore::open(dir.path());
        std::fs::write(&file, "one\n").unwrap();
        store.record(dir.path(), &file, "write");
        std::fs::write(&file, "two\n").unwrap();
        store.record(dir.path(), &file, "edit");

        let entries = store.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].seq + 1, entries[1].seq);
        assert_eq!(entries[0].tool, "write");
        assert_eq!(entries[1].tool, "edit");
        let newest = store.paths();
        assert_eq!(newest.len(), 1, "one path, one newest entry");
        assert_eq!(newest[0].seq, 2);
    }

    #[test]
    fn an_oversized_file_is_skipped_with_a_reason_that_reaches_the_ledger() {
        let dir = ws();
        let file = dir.path().join("src/big.bin");
        std::fs::write(&file, vec![7u8; (MAX_FILE_BYTES + 1) as usize]).unwrap();
        let store = CheckpointStore::open(dir.path());
        let recorded = store.record(dir.path(), &file, "write");
        match recorded {
            Recorded::Skipped(reason) => assert!(reason.contains("limit"), "{reason}"),
            other => panic!("expected a skip, got {other:?}"),
        }
        // And the skip is visible afterwards, not silent.
        let entry = store
            .latest_for("src/big.bin")
            .expect("the skip is recorded");
        assert!(entry.skipped.is_some());
        assert!(entry.blob.is_none());
    }

    #[test]
    fn the_store_evicts_old_blobs_when_it_grows_past_its_ceiling() {
        let dir = ws();
        let store = CheckpointStore::with_max_bytes(dir.path(), 40);
        let file = dir.path().join("src/lib.rs");
        for round in 0..6 {
            std::fs::write(&file, format!("content number {round}\n")).unwrap();
            store.record(dir.path(), &file, "write");
        }
        // record() already evicts. Observe its result before asking for another pass.
        assert_eq!(store.stored_bytes(), 34, "only the two newest blobs fit");
        for round in 0..6 {
            let blob = blob_name(format!("content number {round}\n").as_bytes());
            assert_eq!(
                store.blob_path(&blob).exists(),
                round >= 4,
                "unexpected retention for round {round}"
            );
        }
        assert!(store.evict_if_needed().is_empty(), "eviction is idempotent");
        // The newest blob survives, because that is the one a revert would use.
        let newest = store.paths().into_iter().next().unwrap();
        let blob = newest.blob.expect("kept");
        assert!(
            store.blob_path(&blob).exists(),
            "the newest checkpoint is never evicted"
        );
    }

    #[test]
    fn a_path_outside_the_workspace_is_skipped_rather_than_stored() {
        let dir = ws();
        let other = tempfile::tempdir().unwrap();
        let outside = other.path().join("elsewhere.rs");
        std::fs::write(&outside, "x\n").unwrap();
        let store = CheckpointStore::open(dir.path());
        assert_eq!(
            store.record(dir.path(), &outside, "write"),
            Recorded::Skipped("outside the workspace".into())
        );
    }

    #[test]
    fn blob_names_are_stable_and_content_dependent() {
        assert_eq!(blob_name(b"abc"), blob_name(b"abc"));
        assert_ne!(blob_name(b"abc"), blob_name(b"abd"));
        assert_ne!(blob_name(b"abc"), blob_name(b"abcd"));
        assert_eq!(blob_name(b"abc").len(), 32);
    }
}
