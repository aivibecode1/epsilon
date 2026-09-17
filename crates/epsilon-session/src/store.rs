//! Session file store: append, branch navigation, and derived model view.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use epsilon_contracts::agent_event::TurnEndReason;
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_contracts::session_entry::{
    CURRENT_SESSION_VERSION, EntryKind, SessionEntry, SessionHeader, SessionHeaderKind, SessionLine,
};

use crate::lease::{LeaseOutcome, SessionLease};

/// One row of the model-visible view.
#[derive(Debug, Clone)]
pub enum ModelViewEntry {
    /// A real transcript message within the retained window.
    Message(Message),
    /// A synthetic summary message emitted by a compaction entry.
    Summary {
        /// Summary body text.
        text: String,
        /// Seq of the compaction entry that produced it.
        seq: u64,
    },
}

/// Why one line of a session file could not be taken as an entry (M75).
///
/// The distinction this type exists to make: "this file is damaged" and "this file was written by
/// something that knows more than I do" are different problems with different answers, and
/// reporting both as `corrupt line 5` is how a user ends up deleting a session that was fine.
/// pi classifies contradictions into typed reasons for the same reason; DSH refuses an unknown
/// event *type* rather than skipping it. This is that rule at the line level: refuse, and say
/// which of the two it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineDiagnosis {
    /// Not JSON at all: a partial write, or an edit that damaged the file.
    NotJson,
    /// Valid JSON in entry shape whose `kind.type` this build does not know — a session written
    /// by a newer Epsilon. Refused rather than skipped: dropping it silently would lose a
    /// message and change the transcript's meaning without a word.
    UnknownEntryType {
        /// The `kind.type` string as written in the file.
        entry_type: String,
    },
    /// A second header line, mid-file.
    SecondHeader,
    /// A v1-shaped entry in a current-version file, where healing it would change its meaning.
    LegacyInCurrentFile,
}

impl LineDiagnosis {
    /// A sentence naming the cause and, where it matters, what to do about it.
    pub fn explain(&self) -> String {
        match self {
            LineDiagnosis::NotJson => {
                "not valid JSON (a partial write, or an edit that damaged the file)".to_string()
            }
            LineDiagnosis::UnknownEntryType { entry_type } => format!(
                "entry type `{entry_type}` is not known to this build — the session was probably \
                 written by a newer Epsilon. Refusing rather than dropping the line, because \
                 skipping it would change the transcript silently"
            ),
            LineDiagnosis::SecondHeader => "a second header line, mid-file".to_string(),
            LineDiagnosis::LegacyInCurrentFile => {
                "a v1-format entry in a current-version file (healing it in place would change \
                 its meaning)"
                    .to_string()
            }
        }
    }
}

/// Errors surfaced by the store. Failures are values, not panics — a corrupt session must be
/// repairable or at least inspectable, never fatal to the process.
#[derive(Debug)]
pub enum StoreError {
    /// Underlying filesystem error.
    Io(std::io::Error),
    /// A JSONL line could not be used; carries the 1-based line number and why (M75).
    Corrupt {
        /// 1-based physical line number, as an editor would show it.
        line: usize,
        /// What was wrong with it.
        diagnosis: LineDiagnosis,
    },
    /// Another process holds this session's write lease (M61). Carries the
    /// holder's recorded identity (`pid=…`) when the lock file was readable.
    Leased {
        /// Holder line from the lock file, empty when unreadable.
        holder: String,
    },
}

impl StoreError {
    /// A corrupt-line failure with a typed reason.
    pub fn corrupt(line: usize, diagnosis: LineDiagnosis) -> Self {
        StoreError::Corrupt { line, diagnosis }
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Io(e) => write!(f, "io error: {e}"),
            StoreError::Corrupt { line, diagnosis } => {
                write!(
                    f,
                    "session line #{line}: {} (file left intact)",
                    diagnosis.explain()
                )
            }
            StoreError::Leased { holder } => write!(
                f,
                "another epsilon is already writing this session ({}) — one writer per \
                 session file; close it, or remove the .lock file if it is a crash \
                 leftover (a live holder re-stamps it every few seconds)",
                if holder.trim().is_empty() {
                    "holder unknown"
                } else {
                    holder.trim()
                }
            ),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e)
    }
}

/// Where one entry physically lives in the file (offset-index for lazy reads).
#[derive(Debug, Clone, Copy)]
struct EntryLocation {
    offset: u64,
    len: usize,
    id_hash: u64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn hash_id(id: &str) -> u64 {
    // FNV-1a: enough to correlate index rows with parsed entries without keeping strings.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

/// An open session file. Holds an offset index so long sessions stream lazily instead of
/// living fully in RAM (fixing pi's memory-growth weakness while keeping the JSONL format).
///
/// M41: the first line is a [`SessionHeader`] (`version`, session `id`, `cwd`,
/// `parent_session`). New files get one on creation; old headerless files open
/// at version 1 with no header (all reads tolerate the missing line).
pub struct SessionStore {
    path: PathBuf,
    file: File,
    /// seq -> location of the entry's line.
    index: Vec<EntryLocation>,
    /// Parsed leaf entry (id + seq); `None` for an empty session.
    leaf: Option<(String, u64)>,
    next_seq: u64,
    /// Header read at open (or synthesized for old headerless files).
    /// `None` only for a brand-new empty file with no entries yet.
    header: Option<SessionHeader>,
    /// True when the indexed entries were migrated forward from an older
    /// version (caller persists via [`SessionStore::persist_migration`]).
    migrated: bool,
    /// Migrated entries in seq order (populated only when `migrated`).
    /// Reads serve these clones: indexed offsets still point at the OLD lines
    /// until [`SessionStore::persist_migration`] rewrites the file.
    migrated_entries: Vec<SessionEntry>,
    /// Cross-process write lease (M61). The store IS the writer, so the writer's
    /// lock belongs to it: a second store on the same file — in this process or
    /// any other — is refused at construction, and the lock is released exactly
    /// when the store goes away. Readers never open a store (session discovery
    /// parses the file directly), so reads stay unconstrained.
    _lease: SessionLease,
}

impl SessionStore {
    /// Open (creating if absent) a session file.
    ///
    /// A crash-truncated final line is detected and physically removed automatically —
    /// recovery is the expected path after a kill, not an exceptional error.
    ///
    /// M41: header/version handling (pi `session-manager.ts`):
    /// - a missing header line on a non-empty file means version 1;
    /// - v1 entries (no `id`/`parent_id`, compaction `firstKeptEntryIndex`) are
    ///   migrated in memory (chain-linked, index converted) and
    ///   [`SessionStore::persist_migration`] writes the result back;
    /// - `hookMessage`-role messages become `custom` (pi's v2→v3 rename).
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_in(path, &std::env::current_dir().unwrap_or_default())
    }

    /// Open with an explicit working directory for the new-file header
    /// (pi records the session cwd at creation). Pass the real workspace root.
    pub fn open_in(path: impl AsRef<Path>, cwd: &Path) -> Result<Self, StoreError> {
        Self::open_with_parent(path, cwd, None)
    }

    /// [`SessionStore::open_in`] with the parent-session link a NEW file's header
    /// should record (M58: a delegated task's transcript is a session spawned by
    /// another session). An existing file keeps its header verbatim — a child is
    /// never re-parented, and a top-level session never gains a parent.
    ///
    /// The link must be part of the *creation* write: the store's handle is
    /// append-only, and on Windows a write through FILE_APPEND_DATA lands at EOF
    /// no matter where the cursor was seeked (the M4 lesson) — so re-stamping a
    /// header after the fact appends a second one instead of overwriting it.
    pub fn open_child_in(
        path: impl AsRef<Path>,
        cwd: &Path,
        parent_session: &str,
    ) -> Result<Self, StoreError> {
        Self::open_with_parent(path, cwd, Some(parent_session))
    }

    /// Shared constructor: `parent` lands in the header of a brand-new file.
    fn open_with_parent(
        path: impl AsRef<Path>,
        cwd: &Path,
        parent: Option<&str>,
    ) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // M61: one writer per session file. The check is `create_new` on
        // `<file>.lock` (atomic on NTFS and POSIX), and the holder heartbeats it
        // so a live process is never mistaken for a crash leftover.
        let lease = match SessionLease::acquire(&path)? {
            LeaseOutcome::Acquired(lease) => lease,
            LeaseOutcome::Held { holder } => {
                return Err(StoreError::Leased {
                    holder: holder.unwrap_or_default(),
                });
            }
        };
        let preexisting = path.exists();
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&path)?;

        let mut store = Self {
            path,
            file,
            index: Vec::new(),
            leaf: None,
            next_seq: 0,
            header: None,
            migrated: false,
            migrated_entries: Vec::new(),
            _lease: lease,
        };
        if !store.reindex()? {
            store.truncate_to_index()?;
        }
        if store.header.is_none() && !preexisting {
            // Brand-new file: stamp the header (with any parent link) before any
            // entry lands. (A preexisting-but-empty file gets its header lazily
            // on first append — same bytes, no empty-file rewrite race.)
            store.write_header(cwd, parent)?;
        }
        Ok(store)
    }

    /// The session header (`None` only for an empty file with no entries yet).
    pub fn header(&self) -> Option<&SessionHeader> {
        self.header.as_ref()
    }

    /// Effective file format version (1 when the file predates headers).
    pub fn version(&self) -> u32 {
        self.header
            .as_ref()
            .map(SessionHeader::effective_version)
            .unwrap_or(1)
    }

    /// True when [`SessionStore::open`] migrated old entries in memory.
    /// Call [`SessionStore::persist_migration`] to write the result back.
    pub fn migrated(&self) -> bool {
        self.migrated
    }

    /// Rewrite the file with the migrated entries plus a current header
    /// (atomic tmp+rename, same discipline as the auth store).
    pub fn persist_migration(&mut self) -> Result<(), StoreError> {
        let entries = self.read_all()?;
        let header = self.header.clone().unwrap_or_else(|| SessionHeader {
            kind: SessionHeaderKind::Session,
            version: Some(CURRENT_SESSION_VERSION),
            id: epsilon_contracts::new_id("ses"),
            timestamp: iso_now(),
            cwd: String::new(),
            parent_session: None,
        });
        let mut header = header;
        header.version = Some(CURRENT_SESSION_VERSION);
        let tmp = self.path.with_extension("jsonl.migrating");
        {
            let mut out = String::new();
            out.push_str(&serde_json::to_string(&header).map_err(std::io::Error::other)?);
            out.push('\n');
            for e in &entries {
                out.push_str(&serde_json::to_string(e).map_err(std::io::Error::other)?);
                out.push('\n');
            }
            std::fs::write(&tmp, out)?;
        }
        std::fs::rename(&tmp, &self.path)?;
        self.file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&self.path)?;
        self.header = Some(header);
        self.migrated = false;
        self.reindex()?;
        Ok(())
    }

    /// Fork bookkeeping: the header id of the session this store's file
    /// continues (pi's `parentSession`). `None` for a root session.
    pub fn parent_session(&self) -> Option<&str> {
        self.header
            .as_ref()
            .and_then(|h| h.parent_session.as_deref())
    }

    /// Stamp a fresh header line at the front of an empty file.
    fn write_header(&mut self, cwd: &Path, parent_session: Option<&str>) -> Result<(), StoreError> {
        let header = SessionHeader {
            kind: SessionHeaderKind::Session,
            version: Some(CURRENT_SESSION_VERSION),
            id: epsilon_contracts::new_id("ses"),
            timestamp: iso_now(),
            cwd: cwd.to_string_lossy().into_owned(),
            parent_session: parent_session.map(str::to_string),
        };
        let mut line = serde_json::to_string(&header).map_err(std::io::Error::other)?;
        line.push('\n');
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(line.as_bytes())?;
        self.file.flush()?;
        self.file.seek(SeekFrom::End(0))?;
        self.header = Some(header);
        Ok(())
    }

    /// Path of the underlying file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Number of entries in the file (all branches).
    pub fn len(&self) -> u64 {
        self.next_seq
    }

    /// True when no entries exist yet.
    pub fn is_empty(&self) -> bool {
        self.next_seq == 0
    }

    /// Rebuild the offset index by scanning the file once.
    ///
    /// M41: lines parse as [`SessionLine`]. A leading header line is consumed
    /// into [`SessionStore::header`] (NOT indexed — seq 0 is the first entry,
    /// so old files and new files number entries identically). Blank and
    /// malformed lines keep the M33 semantics: only a bad *last* line counts
    /// as a crash-truncated tail; a bad mid-file line is genuine corruption
    /// ([`StoreError::Corrupt`], file untouched).
    ///
    /// Old files (no header, or header version < [`CURRENT_SESSION_VERSION`])
    /// migrate in memory: entries gain chain `id`/`parent_id` (v1→v2),
    /// compaction `firstKeptEntryIndex` becomes `firstKeptEntryId` (v1→v2),
    /// `hookMessage` roles become `custom` (v2→v3). The store reports
    /// [`SessionStore::migrated`] so the caller can persist.
    ///
    /// Returns `true` when the file was fully clean, `false` when a crash-truncated tail
    /// line was found (indexing stopped there; call [`Self::truncate_to_index`] to remove it).
    pub fn reindex(&mut self) -> Result<bool, StoreError> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut raw = String::new();
        (&self.file).read_to_string(&mut raw)?;
        self.index.clear();
        self.leaf = None;
        self.next_seq = 0;
        self.header = None;
        self.migrated = false;
        self.migrated_entries.clear();

        // Collect the non-blank lines with file offsets first, so a bad last line
        // (crash mid-write) can be told apart from a bad mid-file line (corruption).
        // Line numbers are physical 1-based file lines, blanks counted — what an
        // editor shows when the user goes to inspect the reported line.
        let mut lines: Vec<(u64, &str, usize)> = Vec::new();
        let mut offset = 0u64;
        let mut line_no = 0usize;
        for fragment in raw.split_inclusive('\n') {
            line_no += 1;
            let len = fragment.len();
            if !fragment.trim().is_empty() {
                lines.push((offset, fragment, line_no));
            }
            offset += len as u64;
        }

        // Header: the first non-blank line, when it parses as one.
        let mut first_entry_idx = 0usize;
        #[allow(clippy::collapsible_if)]
        if let Some((_, fragment, _)) = lines.first() {
            if let Some(Ok(SessionLine::Header(h))) = SessionLine::parse(fragment) {
                self.header = Some(h);
                first_entry_idx = 1;
            }
        }

        // Parse entries (header excluded); track raw values for migration.
        let mut parsed: Vec<(u64, usize, SessionEntry, usize)> = Vec::new();
        let mut legacy: Vec<LegacyLine> = Vec::new();
        let mut first_bad: Option<(usize, LineDiagnosis)> = None;
        let mut torn_tail = false;
        let mut good_end = if first_entry_idx == 1 {
            lines
                .first()
                .map(|(off, frag, _)| off + frag.len() as u64)
                .unwrap_or(0)
        } else {
            0
        };
        let last = lines.len();
        for (i, (off, fragment, no)) in lines.iter().enumerate().skip(first_entry_idx) {
            match SessionLine::parse(fragment) {
                Some(Ok(SessionLine::Entry(entry))) => {
                    good_end = off + fragment.len() as u64;
                    parsed.push((*off, fragment.len(), entry, *no));
                }
                Some(Ok(SessionLine::Header(_))) => {
                    // A second header line mid-file is not a header — but it is
                    // also not a transcript entry. Treat it as corruption so the
                    // file gets inspected rather than silently skipped.
                    if i + 1 == last {
                        torn_tail = true;
                    } else if first_bad.is_none() {
                        first_bad = Some((*no, LineDiagnosis::SecondHeader));
                    }
                }
                Some(Err(_)) => {
                    // Typed parse failed: legacy v1 shape (healed by migration
                    // when the file predates current) or genuine garbage.
                    // Legacy lines are well-formed JSON, so a torn tail is
                    // still detectable: only the LAST line may be legacy.
                    match parse_entry_lenient(fragment) {
                        Some(Ok(entry)) => {
                            good_end = off + fragment.len() as u64;
                            parsed.push((*off, fragment.len(), entry, *no));
                        }
                        Some(Err(mut leg)) => {
                            leg.offset = *off;
                            leg.len = fragment.len();
                            leg.line_no = *no;
                            good_end = off + fragment.len() as u64;
                            legacy.push(leg);
                        }
                        None => {
                            if i + 1 == last {
                                torn_tail = true;
                            } else if first_bad.is_none() {
                                // The line is unusable — say *why*, so an unknown entry type is
                                // not reported as damage to a file that is perfectly intact.
                                first_bad = Some((*no, diagnose_line(fragment)));
                            }
                        }
                    }
                }
                None => {
                    if i + 1 == last {
                        // Crash mid-write: the final line is a partial write.
                        torn_tail = true;
                    } else if first_bad.is_none() {
                        first_bad = Some((*no, diagnose_line(fragment)));
                    }
                }
            }
        }
        self.file.seek(SeekFrom::Start(good_end))?;
        if let Some((n, diagnosis)) = first_bad {
            return Err(StoreError::corrupt(n, diagnosis));
        }

        // Migration (pi `migrateToCurrentVersion`, in memory): only when the
        // file predates the current version. New files parse clean — ids,
        // chain links, and roles already in final form.
        let version = self.version();
        if version < CURRENT_SESSION_VERSION && (!parsed.is_empty() || !legacy.is_empty()) {
            migrate_entries(&mut parsed, &legacy, &mut self.header);
            self.migrated = true;
            // Reassign seqs gaplessly: migration preserves order, never drops.
            for (seq, (_, _, entry, _)) in parsed.iter_mut().enumerate() {
                entry.seq = seq as u64;
            }
        } else if !legacy.is_empty() {
            // A current-version file must not contain legacy shapes: without
            // migration they would silently change meaning. Report the first.
            return Err(StoreError::corrupt(
                legacy[0].line_no,
                LineDiagnosis::LegacyInCurrentFile,
            ));
        }

        for (off, len, entry, _) in &parsed {
            self.index.push(EntryLocation {
                offset: *off,
                len: *len,
                id_hash: hash_id(&entry.id),
            });
            self.leaf = Some((entry.id.clone(), entry.seq));
            self.next_seq = entry.seq + 1;
        }
        // Migrated entries live in memory only until persisted; their indexed
        // offsets still point at the OLD lines, so reads must serve the
        // migrated copies. Stash them alongside the index.
        self.migrated_entries = parsed.into_iter().map(|(_, _, e, _)| e).collect();
        Ok(!torn_tail)
    }

    /// Physically truncate anything after the last well-formed indexed line.
    ///
    /// Reopens the file read+write WITHOUT append: append-only handles on Windows carry
    /// only FILE_APPEND_DATA, which `set_len` rejects — and reindex needs read access.
    pub fn truncate_to_index(&mut self) -> Result<(), StoreError> {
        let good_end = self
            .index
            .last()
            .map(|loc| loc.offset + loc.len as u64)
            .unwrap_or(0);
        self.file.flush()?;
        let f = OpenOptions::new().read(true).write(true).open(&self.path)?;
        f.set_len(good_end)?;
        f.sync_all()?;
        // Adopt the freshly-opened handle as our reader/writer going forward.
        self.file = f;
        self.file.seek(SeekFrom::End(0))?;
        Ok(())
    }

    /// Read one entry by sequence number (lazy: seeks and parses just that line;
    /// serves the migrated in-memory copy when the file hasn't been rewritten yet).
    pub fn read_entry(&self, seq: u64) -> Result<Option<SessionEntry>, StoreError> {
        if let Some(migrated) = self.migrated_entries.get(seq as usize) {
            return Ok(Some(migrated.clone()));
        }
        let Some(loc) = self.index.get(seq as usize) else {
            return Ok(None);
        };
        let mut buf = vec![0u8; loc.len];
        let mut f = &self.file;
        f.seek(SeekFrom::Start(loc.offset))?;
        f.read_exact(&mut buf)?;
        let line = String::from_utf8_lossy(&buf);
        Ok(Some(serde_json::from_str(line.trim_end()).map_err(
            |_| {
                // Reading one entry back: diagnose it the same way a scan would, so the
                // message names the cause rather than only the line.
                StoreError::corrupt(seq as usize + 1, diagnose_line(&line))
            },
        )?))
    }

    /// Append a new entry on the current lane; assigns seq/parent/timestamp/id and flushes.
    ///
    /// M41: the first append to a headerless preexisting file stamps the header
    /// first (lazy creation — `open` only stamps brand-new files, so a resumed
    /// empty file and a fresh file converge on the same bytes).
    pub fn append(&mut self, kind: EntryKind) -> Result<SessionEntry, StoreError> {
        if self.header.is_none() {
            let cwd = std::env::current_dir().unwrap_or_default();
            self.write_header(&cwd, None)?;
            // A migrated file must be persisted before new appends land, or the
            // new line would dangle past rewritten offsets.
            if self.migrated {
                self.persist_migration()?;
            }
        }
        let entry = SessionEntry {
            id: epsilon_contracts::new_id("ent"),
            seq: self.next_seq,
            parent_id: self.leaf.as_ref().map(|(id, _)| id.clone()),
            timestamp_ms: now_ms(),
            kind,
        };
        let mut line = serde_json::to_string(&entry).map_err(std::io::Error::other)?;
        line.push('\n');
        let offset = self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(line.as_bytes())?;
        self.file.flush()?;

        self.index.push(EntryLocation {
            offset,
            len: line.len(),
            id_hash: hash_id(&entry.id),
        });
        self.leaf = Some((entry.id.clone(), entry.seq));
        self.next_seq += 1;
        Ok(entry)
    }

    /// Current leaf entry id (the tip of the active branch).
    pub fn leaf_id(&self) -> Option<String> {
        self.leaf.as_ref().map(|(id, _)| id.clone())
    }

    /// Walk back from the leaf collecting the active branch (oldest first).
    ///
    /// Parses the file once; branch walks are infrequent (model view, navigation), so
    /// simplicity beats a reverse id->seq index until profiling says otherwise.
    pub fn branch_entries(&self) -> Result<Vec<SessionEntry>, StoreError> {
        let all = self.read_all()?;
        let by_pos: std::collections::HashMap<&str, usize> = all
            .iter()
            .map(|e| (e.id.as_str(), e.seq as usize))
            .collect();

        let mut chain_rev: Vec<SessionEntry> = Vec::new();
        let mut cursor = self.leaf_id();
        while let Some(id) = cursor {
            let Some(pos) = by_pos.get(id.as_str()).copied() else {
                break;
            };
            let entry = &all[pos];
            cursor = entry.parent_id.clone();
            chain_rev.push(entry.clone());
        }
        chain_rev.reverse();
        Ok(chain_rev)
    }

    /// Remove the final entry (overflow-retry discard). Index-aware, O(1).
    pub fn pop_last(&mut self) -> Result<Option<SessionEntry>, StoreError> {
        if self.next_seq == 0 {
            return Ok(None);
        }
        let last_seq = self.next_seq - 1;
        let entry = self.read_entry(last_seq)?;
        self.index.pop();
        self.next_seq = last_seq;
        self.leaf = match self.index.last() {
            Some(_) => {
                let prev = self.read_entry(last_seq - 1)?;
                prev.map(|e| (e.id.clone(), e.seq))
            }
            None => None,
        };
        // Truncate the physical file to drop the popped line.
        let good_end = self
            .index
            .last()
            .map(|l| l.offset + l.len as u64)
            .unwrap_or(0);
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)?;
        f.set_len(good_end)?;
        f.seek(SeekFrom::End(0))?;
        self.file = f;
        Ok(entry)
    }
    /// Read every parsed entry (oldest first). Used by branch walks and validation tooling.
    pub fn read_all(&self) -> Result<Vec<SessionEntry>, StoreError> {
        let mut out = Vec::with_capacity(self.index.len());
        for s in 0..self.index.len() as u64 {
            out.push(self.read_entry(s)?.expect("index and file agree"));
        }
        Ok(out)
    }

    /// Navigate: make `entry` the new leaf (pi's tree navigation). Everything after it stays
    /// in the file (human log intact) but leaves the active branch.
    pub fn navigate_to(&mut self, entry_id: &str) -> Result<(), StoreError> {
        // find the entry anywhere
        let target_hash = hash_id(entry_id);
        for s in 0..self.index.len() {
            if self.index[s].id_hash == target_hash {
                let entry = self.read_entry(s as u64)?.expect("indexed entry exists");
                self.leaf = Some((entry.id.clone(), entry.seq));
                return Ok(());
            }
        }
        Err(StoreError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("no such entry: {entry_id}"),
        )))
    }

    /// The model-visible view of the active branch: message entries plus synthetic summary
    /// messages emitted by compaction entries; shadowed ranges are skipped entirely.
    pub fn model_view(&self) -> Result<Vec<ModelViewEntry>, StoreError> {
        let branch = self.branch_entries()?;
        let mut out = Vec::new();
        // Messages buffer until we know whether a later compaction shadows them
        // (compaction entries shadow everything *before* them since the prior boundary).
        let mut pending: Vec<ModelViewEntry> = Vec::new();

        for e in &branch {
            match &e.kind {
                EntryKind::Message { message, .. } => {
                    if message.is_llm_visible() {
                        pending.push(ModelViewEntry::Message(message.clone()));
                    }
                }
                EntryKind::Compaction {
                    summary,
                    retained_tail,
                    ..
                } => {
                    out.push(ModelViewEntry::Summary {
                        text: summary.clone(),
                        seq: e.seq,
                    });
                    for m in retained_tail {
                        out.push(ModelViewEntry::Message(m.clone()));
                    }
                    pending.clear();
                }
                _ => {}
            }
        }
        out.extend(pending);
        Ok(out)
    }

    /// Append a typed turn-end marker (Epsilon addition; sticky reason lives in the log).
    pub fn record_turn_end(&mut self, reason: TurnEndReason) -> Result<(), StoreError> {
        self.append(EntryKind::TurnEnd { reason })?;
        Ok(())
    }
}

impl ModelViewEntry {
    /// Convert to provider-visible messages (summary becomes a user-role context message).
    pub fn into_message(self) -> Message {
        match self {
            ModelViewEntry::Message(m) => m,
            ModelViewEntry::Summary { text, .. } => Message::User(UserMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: 0,
                content: vec![epsilon_contracts::content::UserContent::Text {
                    text: format!("<compaction_summary>\n{text}\n</compaction_summary>"),
                }],
            }),
        }
    }
}

/// ISO-8601 UTC timestamp for new headers (pi's `new Date().toISOString()`).
fn iso_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // Millis → calendar date without a date crate: format as epoch-millis with
    // an ISO envelope pi tolerates on read (it never parses the timestamp).
    format!("{ms}")
}

/// One legacy (v1) entry line that failed typed parse, kept raw for migration.
///
/// v1 entries lack `id`/`parent_id` entirely and compaction entries carry
/// `firstKeptEntryIndex` instead of `firstKeptEntryId` — shapes the typed
/// `SessionEntry` can no longer express, so they ride here as JSON until
/// [`migrate_entries`] heals them.
#[derive(Debug, Clone)]
struct LegacyLine {
    offset: u64,
    len: usize,
    line_no: usize,
    value: serde_json::Value,
}

/// Why this line is unusable, in the terms a reader needs (M75).
///
/// Runs only after every parse path has already failed, so it can afford to look at the shape
/// rather than be fast.
fn diagnose_line(fragment: &str) -> LineDiagnosis {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(fragment.trim_end()) else {
        return LineDiagnosis::NotJson;
    };
    // An entry's kind is `{"type": "…"}`; anything else is either a header or a legacy shape.
    if let Some(kind_type) = value
        .get("kind")
        .and_then(|k| k.get("type"))
        .and_then(|t| t.as_str())
    {
        return LineDiagnosis::UnknownEntryType {
            entry_type: kind_type.to_string(),
        };
    }
    if value.get("session_id").is_some() || value.get("version").is_some() {
        return LineDiagnosis::SecondHeader;
    }
    if is_legacy_entry_shape(&value) {
        return LineDiagnosis::LegacyInCurrentFile;
    }
    LineDiagnosis::NotJson
}

/// Lenient parse: typed entries first, then legacy v1 shapes (no `id`/`seq`),
/// then genuine garbage (`None`).
fn parse_entry_lenient(fragment: &str) -> Option<Result<SessionEntry, LegacyLine>> {
    let trimmed = fragment.trim_end();
    if let Ok(entry) = serde_json::from_str::<SessionEntry>(trimmed) {
        return Some(Ok(entry));
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(value) if is_legacy_entry_shape(&value) => Some(Err(LegacyLine {
            offset: 0,
            len: 0,
            line_no: 0,
            value,
        })),
        _ => None,
    }
}

/// A JSON object shaped like a v1 transcript entry: `type` + `timestamp` with
/// no `id`/`seq` (pi's v1 lines predate the tree structure).
fn is_legacy_entry_shape(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            map.contains_key("type")
                && map.contains_key("timestamp")
                && !map.contains_key("id")
                && !map.contains_key("seq")
        }
        _ => false,
    }
}

/// In-memory migration of parsed v1/v2 entries to the current version
/// (pi `migrateV1ToV2` + `migrateV2ToV3`; the header version is bumped by the
/// caller after this runs).
///
/// Typed entries gain chain `id`/`parent_id` when missing; legacy lines heal
/// into typed entries (compaction `firstKeptEntryIndex` → `firstKeptEntryId`,
/// `hookMessage` roles → `custom`); the header version is bumped by the caller.
fn migrate_entries(
    parsed: &mut Vec<(u64, usize, SessionEntry, usize)>,
    legacy: &[LegacyLine],
    header: &mut Option<SessionHeader>,
) {
    // Heal order: legacy lines slot into file order by (offset, line_no) —
    // but legacy offsets are filled by the caller before this runs.
    let mut healed: Vec<(u64, usize, SessionEntry, usize)> = Vec::new();
    for leg in legacy {
        let entry = heal_legacy_line(leg, &parsed_healed_ids(parsed, &healed));
        healed.push((leg.offset, leg.len, entry, leg.line_no));
    }
    parsed.extend(healed);
    // Chain-link + rename pass over everything (pi `migrateV1ToV2` walks the
    // whole file, so a v2 file with one legacy line still heals its neighbors).
    parsed.sort_by_key(|(off, _, _, _)| *off);
    let mut prev_id: Option<String> = None;
    for (_, _, entry, _) in parsed.iter_mut() {
        if entry.id.is_empty() {
            entry.id = epsilon_contracts::new_id("ent");
        }
        if entry.parent_id.is_none() {
            entry.parent_id = prev_id.clone();
        }
        prev_id = Some(entry.id.clone());
        if let EntryKind::Message { message, .. } = &mut entry.kind {
            rename_hook_message_role(message);
        }
    }
    if let Some(h) = header.as_mut() {
        h.version = Some(CURRENT_SESSION_VERSION);
    }
}

/// Ids assigned so far (typed + already-healed), for legacy compaction targets.
fn parsed_healed_ids(
    parsed: &[(u64, usize, SessionEntry, usize)],
    healed: &[(u64, usize, SessionEntry, usize)],
) -> Vec<String> {
    parsed
        .iter()
        .chain(healed.iter())
        .map(|(_, _, e, _)| e.id.clone())
        .collect()
}

/// Heal one legacy v1 line into a typed entry.
///
/// pi's v1 message lines carry the full `AgentMessage` under `message`; the
/// shapes this build round-trips are user text (`message.content` string) and
/// compaction summaries. Anything else heals to a `Custom` entry preserving the
/// raw JSON (nothing silently dropped — the human log stays complete, and the
/// model view skips `Custom` by construction).
fn heal_legacy_line(leg: &LegacyLine, ids: &[String]) -> SessionEntry {
    let map = leg.value.as_object();
    let kind = map.and_then(|m| m.get("type")).and_then(|t| t.as_str());
    let timestamp_ms = map
        .and_then(|m| m.get("timestamp"))
        .and_then(|t| t.as_u64())
        .unwrap_or_else(now_ms);
    let id = epsilon_contracts::new_id("ent");
    let entry_kind = match kind {
        Some("message") => {
            let text = map
                .and_then(|m| m.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();
            EntryKind::Message {
                message: Message::User(UserMessage {
                    id: epsilon_contracts::new_id("msg"),
                    timestamp_ms,
                    content: vec![epsilon_contracts::content::UserContent::Text { text }],
                }),
                api_request_sha256: None,
            }
        }
        Some("compaction") => {
            // pi v1→v2: `firstKeptEntryIndex` (positional) becomes
            // `firstKeptEntryId`. Epsilon's shadowing is positional (seq), so
            // the healed compaction keeps its summary/tokens and the resolved
            // target rides in `details` for future tooling.
            let target_id = map
                .and_then(|m| m.get("firstKeptEntryIndex"))
                .and_then(|i| i.as_u64())
                .and_then(|i| ids.get(i as usize).cloned());
            EntryKind::Compaction {
                summary: map
                    .and_then(|m| m.get("summary"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                tokens_before: map
                    .and_then(|m| m.get("tokensBefore"))
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0),
                retained_tail: Vec::new(),
                details: target_id.map(|id| serde_json::json!({"firstKeptEntryId": id})),
                usage: None,
            }
        }
        _ => EntryKind::Custom {
            custom_type: "legacy".into(),
            data: Some(leg.value.clone()),
        },
    };
    SessionEntry {
        id,
        seq: 0,          // reassigned gaplessly by the caller after migration
        parent_id: None, // chain-linked by the caller after migration
        timestamp_ms,
        kind: entry_kind,
    }
}

/// pi's v2→v3 rename: `hookMessage`-role messages become `custom`.
/// Epsilon models custom messages as `Message::Custom`, so the rename moves
/// the entry across variants (preserving id/timestamp/payload).
fn rename_hook_message_role(message: &mut Message) {
    let hook = match message {
        Message::Assistant(a) if a.provider == "hookMessage" => Some(a),
        _ => None,
    };
    if let Some(a) = hook {
        let replacement = Message::Custom(epsilon_contracts::message::CustomMessage {
            id: std::mem::replace(&mut a.id, epsilon_contracts::new_id("msg")),
            timestamp_ms: a.timestamp_ms,
            custom_type: "hookMessage".into(),
            payload: serde_json::json!({"text": a.text()}),
        });
        *message = replacement;
    }
}
