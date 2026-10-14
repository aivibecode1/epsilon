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
//!
//! ## Who reverts
//!
//! [`CheckpointStore::restore`] had no production caller for its whole life: the write half was
//! wired into `files.rs` and the read half existed only in this file's tests, so the store
//! accumulated a durable revert record nobody could reach. The rollback half is now the terminal's
//! `/restore` (`crates/epsilon-tui/src/app/commands.rs`), which resolves the path through
//! [`CheckpointStore::resolve`] and refuses anything whose bytes on disk are not the bytes that
//! were captured. Adding another caller means going through `resolve`, not `latest_for` — the
//! spelling of a ledger key is this file's business, not a front end's.

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
    /// Hash of the bytes as they were when this entry was captured.
    ///
    /// Used to answer one question at restore time: *is the file still the one we
    /// snapshot?* Restoring over a file somebody has since edited destroys their
    /// work, and a snapshot that cannot tell whether that happened is not a
    /// preimage — it is a loaded gun pointed at the user's last hour.
    ///
    /// **This is the PRE-image.** Once a mutation has happened, the file no longer
    /// hashes to it, so this field alone answers "has anything touched this since?"
    /// with a constant yes. [`Self::post_image`] carries what the harness itself left
    /// behind, and it is that one the guard asks about.
    ///
    /// `None` for entries written before this field existed, and for entries
    /// whose capture was skipped. A restore refuses on `None` rather than
    /// assuming, because the assumption it would have to make is the dangerous
    /// one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    /// Hash of the bytes the harness **left on disk** after this entry's mutation.
    ///
    /// # Why this field exists
    ///
    /// `record` runs *before* the write, because a checkpoint that is taken afterwards
    /// captures the damage. So `digest` is the pre-image by construction, and once the write
    /// lands the file no longer hashes to it — which meant the guard in ``Self::restore``
    /// could never pass for any mutation that actually changed a file. Reverting an **edit**
    /// refused one hundred percent of the time, and the reason nobody noticed is recorded in
    /// M104's own commit: *"No UI reaches this yet, which is why it is a fix rather than an
    /// incident."* Wiring `/restore` up is what made it observable.
    ///
    /// The answerable question is not "is the file still what it was before I touched it?" —
    /// it is **"is the file still exactly what this harness left here?"** A post-image answers
    /// that: unchanged means the user has not edited since, so restoring destroys nothing;
    /// changed means they have, so it refuses and leaves their work in place.
    ///
    /// Written by ``Self::amend_with_post_image`` as an **append**, never by rewriting the
    /// entry, because the ledger is append-only and a mutation of it would be the one thing
    /// that could lose a revert record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_image: Option<String>,
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
    /// A post-image amendment was appended for an already-recorded entry.
    ///
    /// Distinct from [`Self::Stored`] because nothing new was captured: the blob already
    /// existed, and the only thing this records is what the harness left on disk afterwards.
    Amended,
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
                // No content to have a digest of: reverting means removing the
                // file, and there is nothing to compare against.
                digest: None,
                post_image: None,
            };
            if let Err(e) = self.append(&entry) {
                return Recorded::Skipped(e);
            }
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
                // Nothing captured, so nothing to compare a restore against.
                digest: None,
                post_image: None,
            };
            // The skip is the outcome here, so an append failure is the same
            // answer with a worse reason.
            if let Err(e) = self.append(&entry) {
                return Recorded::Skipped(format!(
                    "{reason}; and the ledger could not record it: {e}"
                ));
            }
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
            // The blob name IS the content hash, so the preimage is already
            // identified by what was stored; carrying it on the entry is what
            // lets `restore` ask whether the file has moved since.
            digest: Some(blob.clone()),
            // Filled in by `amend_with_post_image` once the write has landed. Until
            // then there is nothing to record, and the restore guard falls back to
            // refusing rather than to assuming.
            post_image: None,
        };
        if let Err(e) = self.append(&entry) {
            // The bytes are on disk but the ledger does not point at them, so the
            // capture cannot be found again. Reporting it as stored would be a
            // durability claim the store cannot honour — and the caller's edit
            // still proceeds either way; only the undo is lost, and it says so.
            return Recorded::Skipped(format!(
                "captured {blob} but the ledger could not be written: {e}"
            ));
        }
        self.evict_if_needed();
        Recorded::Stored {
            blob,
            bytes: entry.bytes,
        }
    }

    fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.root.join("blobs"))
    }

    /// Append one entry to the ledger, or say why it could not be appended.
    ///
    /// This used to return `()`, swallowing four distinct failures — directory
    /// creation, serialization, open, write — while its caller reported the
    /// capture as stored. A caller cannot distinguish "the checkpoint is durable"
    /// from "the checkpoint was never recorded" if both paths return the same
    /// thing, and the second is the one a user discovers when they try to undo.
    fn append(&self, entry: &Entry) -> Result<(), String> {
        self.ensure_dirs()
            .map_err(|e| format!("checkpoint directory: {e}"))?;
        let line =
            serde_json::to_string(entry).map_err(|e| format!("entry not serializable: {e}"))?;
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.ledger_path())
            .map_err(|e| format!("ledger open: {e}"))?;
        // `writeln` then `flush`: a short write that is never flushed is exactly
        // the failure this function exists to stop being silent.
        writeln!(file, "{line}")
            .and_then(|()| file.flush())
            .map_err(|e| format!("ledger write: {e}"))
    }

    /// The entry that reverting `path` would apply: the newest one for that path.
    pub fn latest_for(&self, path: &str) -> Option<Entry> {
        self.entries().into_iter().rev().find(|e| e.path == path)
    }

    /// The entry `argument` names: the newest checkpoint recorded for that path.
    ///
    /// `argument` is what a reader typed after `/restore`, so it may be
    /// workspace-relative (`src/lib.rs`), absolute inside the workspace, or spelled
    /// with either separator. Resolving it HERE rather than in each front end is the
    /// point: [`Self::record`] writes exactly one spelling into the ledger —
    /// workspace-relative and `/`-separated (`Entry::path`) — and a caller that
    /// guesses a second one looks up a key that was never written and reports "no
    /// checkpoint" for a file that has one. That answer is a lie, and a reader who
    /// believes it stops trusting the ones that are true.
    ///
    /// The escape check is not decoration. [`Self::restore`] rejoins
    /// `Entry::path` onto the workspace, so a `..` that survived normalisation here
    /// would let a typed argument name a file outside the tree this store owns — and
    /// `lexically_normalize` is the same function the write and edit tools already
    /// resolve their paths through, so there is one spelling of "inside" rather than
    /// a second answer to the same question.
    ///
    /// The refusal is a sentence and not `None`: every caller has to print
    /// something, and "outside the workspace" and "nothing recorded for this file"
    /// are different facts with different advice. ``Self::no_checkpoint_for`` names
    /// what *is* recoverable, because a reader who typed one wrong path usually has
    /// the right one in mind.
    pub fn resolve(&self, workspace: &Path, argument: &str) -> Result<Entry, String> {
        let raw = argument.trim();
        if raw.is_empty() {
            return Err("no file named — say which one to put back, e.g. src/lib.rs".to_string());
        }
        let (absolute, inside) = crate::util::lexically_normalize(workspace, raw);
        let outside = || {
            format!(
                "'{raw}' is outside the workspace — a checkpoint exists only for a file this \
                 harness changed here"
            )
        };
        if !inside {
            return Err(outside());
        }
        // The ledger is keyed by the workspace-relative form, so recover it by
        // removing the workspace rather than by re-deriving it: two derivations of
        // one key is how the two stop agreeing.
        let Ok(relative) = absolute.strip_prefix(workspace) else {
            return Err(outside());
        };
        self.latest_for(&relative.to_string_lossy().replace('\\', "/"))
            .ok_or_else(|| self.no_checkpoint_for(raw))
    }

    /// Why `raw` has nothing to restore, naming the files that do.
    fn no_checkpoint_for(&self, raw: &str) -> String {
        let available = self.paths();
        if available.is_empty() {
            return format!(
                "no checkpoint for '{raw}' — nothing in this workspace has one; one is \
                 recorded before every write and edit, so only files the harness changed have \
                 a revert"
            );
        }
        // A transcript is the wrong place for a hundred rows, and an answer truncated
        // to nothing is the same as no answer. The count says how many there are, so
        // a truncated list is visibly a sample.
        const SHOWN: usize = 5;
        let shown: Vec<&str> = available
            .iter()
            .take(SHOWN)
            .map(|e| e.path.as_str())
            .collect();
        let more = available.len().saturating_sub(shown.len());
        format!(
            "no checkpoint for '{raw}' — {} file(s) have one: {}{}",
            available.len(),
            shown.join(", "),
            if more == 0 {
                String::new()
            } else {
                format!(" · and {more} more")
            }
        )
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
    /// Returns the number of bytes written, or `0` when the entry means the file
    /// did not exist and was removed. The restore itself is only recorded by the
    /// caller's next mutation — reverting is a change like any other, and the
    /// caller owns the mutation queue that keeps writers serialized.
    ///
    /// # No blob is two different situations, and only one of them means "delete"
    ///
    /// `blob: None` is written for a file that did not exist, so reverting means
    /// removing it. It is *also* written for a file whose capture was **skipped**
    /// — too large, unreadable, or outside the workspace. Those entries carry
    /// `existed: true` and a `skipped` reason.
    ///
    /// Treating the second case like the first turns "we could not save this
    /// file" into "delete this file". That is the worst outcome a revert path can
    /// produce: the user asks to undo a large file's edit, and loses the file.
    /// So a missing blob on a file that existed is an error the caller must see,
    /// never a silent removal.
    ///
    /// # The deletion path is guarded too
    ///
    /// An `existed: false` entry also has `blob: None`, and its revert **removes** the file —
    /// the most destructive thing this store does. It runs the same ``check_unchanged`` the
    /// write path runs, against the post-image that `amend_with_post_image` writes for
    /// creations (a creation has no pre-image, but it does have a post-image). Without it, a
    /// user who edited an agent-created file and then asked to undo the creation lost their
    /// edit — after a preview that said only `DELETE`, which is true of the outcome and silent
    /// about whose work it was.
    pub fn restore(&self, workspace: &Path, entry: &Entry) -> Result<u64, String> {
        let target = workspace.join(entry.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let Some(blob) = &entry.blob else {
            if entry.existed {
                let why = entry
                    .skipped
                    .as_deref()
                    .unwrap_or("the reason was not recorded");
                return Err(format!(
                    "cannot restore {}: nothing was captured for it ({why}). \
                     The file is untouched rather than removed — revert it by hand.",
                    entry.path
                ));
            }
            if target.exists() {
                // Deleting is the most destructive thing this store does, so it is guarded by
                // the same question the write path asks: is the file still exactly what this
                // harness left here? A creation has no pre-image, but it does have a post-image
                // once the write has landed, and that is the one that can be compared.
                check_unchanged(&target, entry)?;
                std::fs::remove_file(&target).map_err(|e| format!("remove: {e}"))?;
            }
            return Ok(0);
        };
        let bytes = std::fs::read(self.blob_path(blob))
            .map_err(|e| format!("checkpoint blob {blob} is missing: {e}"))?;
        // The file must still be the one this entry snapshot. Without this check a
        // restore silently destroys whatever happened between the capture and the
        // revert — the user's own later edits as readily as another agent's.
        check_unchanged(&target, entry)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        }
        std::fs::write(&target, &bytes).map_err(|e| format!("write: {e}"))?;
        Ok(bytes.len() as u64)
    }

    /// Record what this harness **left on disk** for `path`, once the mutation has landed.
    ///
    /// # Why an append and not an edit
    ///
    /// The ledger is append-only on purpose: it is the only durable record of what could be
    /// reverted, so a write that could lose or reorder one is the single worst thing this store
    /// could do. The amendment is therefore a **new entry** that repeats the fields a restore
    /// needs — the same `blob`, the same `bytes`, the same `existed` — and differs only in
    /// carrying [`Entry::post_image`].
    ///
    /// # Why the newest entry for a path is then the amendment
    ///
    /// [`Self::latest_for`] reads newest-first, and [`Self::resolve`] resolves through it, so the
    /// amendment is what a `/restore` finds — which is what we want: the same pre-image bytes,
    /// paired with a guard that can actually be satisfied.
    ///
    /// # When it does not apply
    ///
    /// Nothing to amend (the mutation never recorded a checkpoint, or the newest entry is for a
    /// different path) and a file that cannot be read are both **`Ok(())`**. This runs after a
    /// write the caller has already performed, so it must never become the reason a write is
    /// reported as failed; the cost of not recording is that a later restore refuses, which is
    /// the safe direction.
    pub fn amend_with_post_image(&self, workspace: &Path, path: &Path, tool: &str) -> Recorded {
        let relative = match path.strip_prefix(workspace) {
            Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
            Err(_) => return Recorded::Skipped("outside the workspace".into()),
        };
        let Some(newest) = self.latest_for(&relative) else {
            return Recorded::Skipped("nothing to amend".into());
        };
        // `blob: None` on the newest entry means the capture was skipped — unless the file did
        // not exist, which is also `blob: None` and is **not** a skipped capture. A creation has
        // no pre-image to repeat because there is no pre-image, but the bytes this harness is
        // leaving behind are exactly what the guard needs in order to tell "the user edited
        // this since" from "nothing has touched it". Skipping creations here is what left the
        // deletion path in `restore` with nothing to check against.
        if newest.blob.is_none() && newest.existed {
            return Recorded::Skipped("nothing was captured to amend".into());
        }
        let Ok(bytes) = std::fs::read(path) else {
            return Recorded::Skipped("could not read the file back".into());
        };
        match self.append(&Entry {
            seq: newest.seq + 1,
            at: newest.at,
            path: relative,
            blob: newest.blob.clone(),
            bytes: newest.bytes,
            tool: tool.into(),
            existed: newest.existed,
            skipped: None,
            digest: newest.digest.clone(),
            post_image: Some(blob_name(&bytes)),
        }) {
            Ok(()) => Recorded::Amended,
            Err(e) => Recorded::Skipped(e),
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

/// Refuse a restore that would clobber a file which has moved since capture.
///
/// The comparison is content, not metadata. Length and mtime are cheaper, and a
/// length check is genuinely tempting because an edit often changes it — but two
/// writes inside one filesystem clock tick with the same length are
/// indistinguishable by metadata, and that is not a hypothetical: it is exactly
/// the flake that `read_log`'s own guard test had until it started writing
/// different lengths. Content cannot be fooled that way.
///
/// The content hash is already computed for every capture (`blob_name` is
/// content-addressed), so this costs one read of a file the caller is about to
/// overwrite.
fn check_unchanged(target: &Path, entry: &Entry) -> Result<(), String> {
    if !target.exists() {
        // The snapshot existed and the file is gone: restoring recreates it, which
        // is the one case where a missing file is not a conflict.
        return Ok(());
    }
    let current = std::fs::read(target).map_err(|e| format!("read before restore: {e}"))?;
    let now = blob_name(&current);
    // **Four cases, and each one says something true.** A single "changed or not" test could not:
    // "the user edited it" and "this entry never recorded what we left behind" are different
    // facts with different advice, and merging them is how a refusal becomes a mystery.
    match (
        entry.post_image.as_deref(),
        entry.digest.as_deref(),
        entry.blob.as_deref(),
    ) {
        // The answerable question, and the one that makes `/restore` usable at all.
        (Some(post), _, _) if post == now => Ok(()),
        (Some(_), _, _) => Err(format!(
            "cannot restore {}: the file changed after this harness wrote it \
         ({} bytes now). Reverting would discard that work — read the file first, \
         and delete it if you really want the older content back.",
            entry.path,
            current.len(),
        )),
        // The pre-image still on disk: nothing has touched it since the capture.
        (None, Some(digest), _) if digest == now => Ok(()),
        // A capture with no post-image — written before this field existed, or by a build
        // that never amended. Comparing against the pre-image is the older, coarser rule,
        // and it refuses anything the harness itself changed. That is the safe direction
        // and it is why `post_image` exists; saying so is more useful than implying the
        // user edited anything.
        (None, Some(_), _) => Err(format!(
            "cannot restore {}: this checkpoint does not record what the harness left \
         behind, so a later edit cannot be told from your own. The file is untouched — \
         read it first, and delete it if you really want the older content back.",
            entry.path,
        )),
        // An entry from before digests existed. Its content is in the blob, so the old
        // code would happily write it; "we cannot tell whether this is still the file" is
        // a different answer from "yes", and only one of them is safe.
        (None, None, Some(_)) => Err(format!(
            "cannot restore {}: this checkpoint predates content verification, so \
         overwriting the file cannot be shown to be safe. Remove the file first to \
         force it.",
            entry.path,
        )),
        // A creation carries neither a digest nor a blob — there was no pre-image to hash.
        // Written before `post_image` was amended onto creations, that leaves nothing at
        // all to compare against, and "we cannot tell whether this is still the file" is
        // the only safe answer. The advice differs from the arms above because reverting a
        // creation is a **deletion**: telling a reader to "delete it" when the file is the
        // thing in question would be nonsense.
        (None, None, None) => Err(format!(
            "cannot restore {}: this checkpoint records no content to check the file \
         against, so deleting it could throw away work done since. The file is untouched \
         — read it first, and delete it by hand if you really want it gone.",
            entry.path,
        )),
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

        // The file is untouched since the capture, which is the only state a
        // restore will act on.
        let entry = store.latest_for("src/lib.rs").expect("an entry");
        let bytes = store.restore(dir.path(), &entry).unwrap();
        assert_eq!(bytes, "original content\n".len() as u64);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "original content\n"
        );
    }

    #[test]
    fn a_restore_refuses_to_clobber_a_file_that_changed_since_capture() {
        // The failure this prevents: the user edits a file after the agent's
        // snapshot, asks for an undo, and loses their own work. The snapshot
        // cannot know whether the change was theirs, another agent's or a
        // checkout — so it refuses and says what it found.
        let dir = ws();
        let file = dir.path().join("src/lib.rs");
        std::fs::write(&file, "as the agent saw it\n").unwrap();
        let store = CheckpointStore::open(dir.path());
        let _ = store.record(dir.path(), &file, "write");
        // Then the write lands, and the store learns what the harness left behind. This is
        // the sequence every `write`/`edit` now performs; without it there is nothing to
        // tell "the user edited afterwards" from "the harness's own change", and this
        // test would be exercising a shape the product no longer produces.
        std::fs::write(&file, "as the agent left it\n").unwrap();
        assert_eq!(
            store.amend_with_post_image(dir.path(), &file, "write"),
            Recorded::Amended
        );
        let entry = store.latest_for("src/lib.rs").expect("an entry");

        std::fs::write(&file, "the user's later work\n").unwrap();
        let err = store
            .restore(dir.path(), &entry)
            .expect_err("a changed file must not be overwritten");
        assert!(err.contains("changed after this harness wrote it"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "the user's later work\n",
            "the refusal must leave the file exactly as it was"
        );
    }

    #[test]
    fn a_capture_with_no_post_image_refuses_rather_than_assuming() {
        // The coarser rule, kept honest. A capture written by a build that never amended
        // carries only a pre-image, and the pre-image cannot answer "did the user edit
        // this?" — it can only say "the file differs from before the harness touched it",
        // which is true of every edit the harness itself made. So it refuses, and it says
        // *that* is why, rather than implying the user did something.
        let dir = ws();
        let file = dir.path().join("src/lib.rs");
        std::fs::write(&file, "as the agent saw it\n").unwrap();
        let store = CheckpointStore::open(dir.path());
        let _ = store.record(dir.path(), &file, "write");
        let entry = store.latest_for("src/lib.rs").expect("an entry");
        assert!(
            entry.post_image.is_none(),
            "record alone records no post-image"
        );

        std::fs::write(&file, "whatever is here now\n").unwrap();
        let err = store
            .restore(dir.path(), &entry)
            .expect_err("a capture with no post-image cannot be shown to be safe");
        assert!(
            err.contains("does not record what the harness left behind"),
            "{err}"
        );
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "whatever is here now\n",
            "the refusal leaves the file alone"
        );
    }

    #[test]
    fn a_restore_recreates_a_file_that_was_deleted_after_capture() {
        // The other side of the same rule: a missing file is not a conflict, it
        // is the case the snapshot exists to undo.
        let dir = ws();
        let file = dir.path().join("src/lib.rs");
        std::fs::write(&file, "original\n").unwrap();
        let store = CheckpointStore::open(dir.path());
        let _ = store.record(dir.path(), &file, "write");
        let entry = store.latest_for("src/lib.rs").expect("an entry");

        std::fs::remove_file(&file).unwrap();
        assert_eq!(
            store.restore(dir.path(), &entry).expect("recreates"),
            "original\n".len() as u64
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "original\n");
    }

    #[test]
    fn a_restore_refuses_an_entry_it_cannot_verify() {
        // An entry written before digests existed. Its content is in the blob, so
        // the old code would happily write it; the point of the refusal is that
        // "we cannot tell whether this is still the file" is a different answer
        // from "yes", and only one of them is safe.
        let dir = ws();
        let file = dir.path().join("src/lib.rs");
        std::fs::write(&file, "content\n").unwrap();
        let store = CheckpointStore::open(dir.path());
        let _ = store.record(dir.path(), &file, "write");
        let mut entry = store.latest_for("src/lib.rs").expect("an entry");
        entry.digest = None;

        let err = store
            .restore(dir.path(), &entry)
            .expect_err("an unverifiable entry must be refused");
        assert!(err.contains("predates content verification"), "{err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "content\n");
    }

    #[test]
    fn a_file_that_did_not_exist_is_restored_by_removing_it() {
        let dir = ws();
        let file = dir.path().join("src/new.rs");
        let store = CheckpointStore::open(dir.path());
        assert_eq!(store.record(dir.path(), &file, "write"), Recorded::Created);

        std::fs::write(&file, "created by the model\n").unwrap();
        // The amendment every real `write` performs after its bytes land
        // (`files.rs`). A creation carries no pre-image, but it does get a post-image, and
        // that is the only thing the deletion guard can compare against.
        assert_eq!(
            store.amend_with_post_image(dir.path(), &file, "write"),
            Recorded::Amended
        );
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
    fn reverting_a_skipped_capture_refuses_instead_of_deleting_the_file() {
        // The failure this pins is the worst one a revert path can produce. An
        // oversized file is recorded with `blob: None` and `existed: true`; an
        // entry for a file that never existed also has `blob: None`. Treating
        // them alike means "we could not save this" becomes "delete this", so the
        // user asks to undo one edit and loses the whole file.
        let dir = ws();
        let file = dir.path().join("src/big.bin");
        let original = vec![7u8; (MAX_FILE_BYTES + 1) as usize];
        std::fs::write(&file, &original).unwrap();
        let store = CheckpointStore::open(dir.path());
        let _ = store.record(dir.path(), &file, "write");
        let entry = store.latest_for("src/big.bin").expect("skip recorded");

        let err = store
            .restore(dir.path(), &entry)
            .expect_err("a skipped capture cannot be restored");
        assert!(
            err.contains("nothing was captured"),
            "the refusal must say why: {err}"
        );
        assert!(
            file.exists(),
            "the file must survive a revert that could not restore it"
        );
        assert_eq!(
            std::fs::read(&file).expect("still readable"),
            original,
            "and its contents must be untouched"
        );
    }

    #[test]
    fn reverting_a_file_that_never_existed_still_removes_it() {
        // The other half: `existed: false` really does mean "delete on revert",
        // and the refusal above must not have swallowed that case.
        let dir = ws();
        let file = dir.path().join("src/fresh.rs");
        let store = CheckpointStore::open(dir.path());
        assert_eq!(store.record(dir.path(), &file, "write"), Recorded::Created);
        std::fs::write(&file, "created by the agent").unwrap();
        // As every real write does, record what was left behind — the guard's subject.
        let _ = store.amend_with_post_image(dir.path(), &file, "write");
        let entry = store.latest_for("src/fresh.rs").expect("entry recorded");

        assert_eq!(store.restore(dir.path(), &entry).expect("restores"), 0);
        assert!(
            !file.exists(),
            "a file the agent created is removed by reverting its creation"
        );
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
