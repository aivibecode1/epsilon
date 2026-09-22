//! Session file store: append, branch navigation, and derived model view.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use epsilon_contracts::agent_event::TurnEndReason;
use epsilon_contracts::content::{ContentBlock, UserContent};
use epsilon_contracts::message::{Message, Usage, UserMessage};
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
    /// A synthetic branch-summary message emitted by an [`EntryKind::BranchSummary`]
    /// entry on the active branch (pi's `branchSummary` role,
    /// `session-manager.ts:401-403`): the context of the path this conversation
    /// came back from.
    BranchSummary {
        /// Summary body text.
        text: String,
        /// Id of the entry the abandoned branch started from (pi's `fromId`;
        /// `root` when the summary is itself a new root).
        from_id: String,
        /// Seq of the branch-summary entry that produced it.
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
    /// by a newer Epsilon. Distinct from [`LineDiagnosis::MalformedEntry`], which is the same
    /// shape with a type this build *does* recognize.
    UnknownEntryType {
        /// The `kind.type` string as written in the file.
        entry_type: String,
    },
    /// A `kind.type` this build **knows**, carrying a body it cannot read.
    ///
    /// The counterpart to `UnknownEntryType`, and the reason both exist: "an entry from a
    /// newer build" and "an entry of a kind I know, written wrong" call for different actions
    /// from whoever reads the message. Preserving the second the way an unknown kind is
    /// preserved would open the session with a silent gap where a turn should be — the
    /// failure M108 was written to end, and the reason the `kind` codec insists on a readable
    /// body for a known tag.
    MalformedEntry {
        /// The `kind.type` string, which this build recognizes.
        entry_type: String,
    },
    /// A second header line, mid-file.
    SecondHeader,
    /// Complete JSON that is none of the shapes this build knows: not an entry, not a header, not
    /// a legacy entry. Refused for the same reason as an unknown entry type, and named separately
    /// because "I do not recognize this line" and "this line is damaged" call for different
    /// actions from whoever reads the message.
    UnknownLineShape,
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
            LineDiagnosis::MalformedEntry { entry_type } => format!(
                "entry type `{entry_type}` is one this build knows, but its body cannot be read \
                 — the entry is damaged rather than foreign. Refusing rather than preserving it, \
                 because an entry nothing can interpret would leave a silent gap in the \
                 transcript"
            ),
            LineDiagnosis::SecondHeader => "a second header line, mid-file".to_string(),
            LineDiagnosis::UnknownLineShape => {
                "valid JSON that is not an entry, a header or a legacy entry — a shape this build \
                 does not know. Refusing rather than dropping the line, because a session written \
                 by a newer Epsilon is not a damaged session"
                    .to_string()
            }
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

/// The active branch tip (pi's `leafId`), with its "no leaf" state made explicit.
///
/// `leaf_id` answering `None` used to be indistinguishable from a failed lookup,
/// so the one navigation pi has that is *not* an id — `resetLeaf`, "put the leaf
/// back before the first entry so the next append starts a new root" — could not
/// be expressed at all. The two states below are the whole vocabulary: the next
/// append chains onto [`Leaf::At`]'s id, or starts a root when there is no leaf.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Leaf {
    /// The active branch ends at this entry (seq kept alongside for O(1) reads).
    At {
        /// Id of the leaf entry.
        id: String,
        /// Its file sequence number.
        seq: u64,
    },
    /// No active leaf: a session with no entries yet, or one whose leaf was reset
    /// ([`SessionStore::reset_leaf`]). Either way the next append is a new root.
    None,
}

impl Leaf {
    /// Id of the leaf entry, when there is one.
    fn id(&self) -> Option<&str> {
        match self {
            Leaf::At { id, .. } => Some(id),
            Leaf::None => None,
        }
    }
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
    /// Active branch tip (pi's `leafId`); [`Leaf::None`] means "the next append is
    /// a root" — an empty session, or a leaf deliberately reset (pi `resetLeaf`).
    leaf: Leaf,
    next_seq: u64,
    /// Header read at open (or synthesized for old headerless files).
    /// `None` only for a brand-new empty file with no entries yet.
    header: Option<SessionHeader>,
    /// Byte offset just past the header line, when the file has one. Set by
    /// [`SessionStore::reindex`]. This is the floor for
    /// [`SessionStore::truncate_to_index`]: a repaired file keeps its header even when no
    /// entry survived, because a header is content and losing it costs the session its
    /// identity.
    header_end: Option<u64>,
    /// True when the indexed entries were migrated forward from an older
    /// version (caller persists via [`SessionStore::persist_migration`]).
    migrated: bool,
    /// The entries as [`SessionStore::reindex`] parsed them, in seq order — the
    /// read-through copy [`SessionStore::read_entry`] serves before touching the file.
    ///
    /// One vector, two jobs, which is why it is populated for EVERY file and not only for
    /// a migrated one (an earlier version of this comment claimed otherwise, and the code
    /// never matched it). For an old file it holds the MIGRATED entries while the index
    /// still points at the old lines, so reads must come from here until
    /// [`SessionStore::persist_migration`] rewrites the file. For a current file it is an
    /// ordinary cache of what the last scan parsed.
    ///
    /// The invariant is positional: `cached_entries[i]` is the entry at `index[i]`, and
    /// the cache is never longer than the index. Appending leaves it one short — the new
    /// entry is read back from the file, which is where it was just written, so a torn
    /// write still shows up as a read error instead of hiding behind a memory copy.
    /// Anything that DROPS an index entry must shorten the cache in step; otherwise the
    /// dropped entry is served again under the next sequence number.
    cached_entries: Vec<SessionEntry>,
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
    /// "Crash-truncated" means a *fragment*: a final line that is not whole JSON, so the write
    /// demonstrably died inside a value. A final line that IS whole JSON but uses a shape this
    /// build does not know is refused instead, with the file left untouched (F08) — see
    /// [`SessionStore::reindex`].
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
            leaf: Leaf::None,
            next_seq: 0,
            header: None,
            header_end: None,
            migrated: false,
            cached_entries: Vec::new(),
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
        // Record where the header ends so `keep_end` still has a floor on a file the
        // store stamped itself. A file opened before this field existed has it filled in
        // by `reindex`; this is the other half of "set whenever `header` is set".
        self.header_end = Some(line.len() as u64);
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
    /// malformed lines keep the M33 semantics: only a bad *last* line can be a
    /// crash-truncated tail; a bad mid-file line is genuine corruption
    /// ([`StoreError::Corrupt`], file untouched).
    ///
    /// F08 narrowed "bad last line" to what the phrase can honestly cover. A last line that is
    /// **not whole JSON** is a fragment — the write died inside a value — so it is dropped as
    /// debris. A last line that **is** whole JSON but not a shape this build knows (an entry type
    /// from a newer Epsilon, a second header, bare JSON of any kind) was written on purpose, so
    /// it is refused with a typed [`LineDiagnosis`] and the file is left alone. Position is not
    /// what makes a line damaged; the old behaviour truncated the same bytes that a mid-file
    /// position would have refused.
    ///
    /// Old files (no header, or header version < [`CURRENT_SESSION_VERSION`])
    /// migrate in memory: entries gain chain `id`/`parent_id` (v1→v2),
    /// compaction `firstKeptEntryIndex` becomes `firstKeptEntryId` (v1→v2),
    /// `hookMessage` roles become `custom` (v2→v3). The store reports
    /// [`SessionStore::migrated`] so the caller can persist.
    ///
    /// Returns `true` when the file was fully clean, `false` when a crash-truncated *fragment* was
    /// found (indexing stopped there; call [`Self::truncate_to_index`] to remove it). A complete
    /// but unrecognized line is an `Err`, not a `false` — the file must not be repaired.
    pub fn reindex(&mut self) -> Result<bool, StoreError> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut raw = String::new();
        (&self.file).read_to_string(&mut raw)?;
        self.index.clear();
        self.leaf = Leaf::None;
        self.next_seq = 0;
        self.header = None;
        self.header_end = None;
        self.migrated = false;
        self.cached_entries.clear();

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

        // Header: the first non-blank line, when it parses as one. Its end offset is
        // remembered separately from the entry index, so a repair can keep it (below).
        let mut first_entry_idx = 0usize;
        #[allow(clippy::collapsible_if)]
        if let Some((off, fragment, _)) = lines.first() {
            if let Some(Ok(SessionLine::Header(h))) = SessionLine::parse(fragment) {
                self.header = Some(h);
                self.header_end = Some(off + fragment.len() as u64);
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
                    //
                    // A *complete* last line is corruption wherever it sits (F08): it was
                    // written on purpose, so removing it destroys something. Only a line
                    // that is still a fragment can be crash debris.
                    if i + 1 == last && !is_whole_json(fragment) {
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
                            if i + 1 == last && !is_whole_json(fragment) {
                                // The write stopped mid-line: no closing brace, no
                                // newline, nothing that parses. That is what a crash
                                // leaves, and dropping it is the repair.
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
                    if i + 1 == last && !is_whole_json(fragment) {
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
            self.leaf = Leaf::At {
                id: entry.id.clone(),
                seq: entry.seq,
            };
            self.next_seq = entry.seq + 1;
        }
        // The parse above produced the entries in their CURRENT shape (migrated, when
        // the file was older). Keep them beside the index: their indexed offsets may
        // still point at the OLD lines, so reads have to come from here until
        // `persist_migration` rewrites the file. See the field's doc for the invariant.
        self.cached_entries = parsed.into_iter().map(|(_, _, e, _)| e).collect();
        Ok(!torn_tail)
    }

    /// Byte offset the file must be cut back to in order to drop everything the index
    /// does not cover — just past the last indexed entry, or, when no entry survived,
    /// just past the header.
    ///
    /// The header is the floor deliberately. It is content, not framing: it carries the
    /// session's id, its working directory and — for a delegated child — the parent link.
    /// Cutting back to zero instead would empty the file while `self.header` still said
    /// otherwise, so the next append would skip the header (it believes one exists) and
    /// the file would come back as a headerless v1 session, re-migrated under fresh ids
    /// with its identity and its parent gone. A torn tail can leave exactly that state:
    /// a new session whose very first entry was half-written.
    fn keep_end(&self) -> u64 {
        self.index
            .last()
            .map(|loc| loc.offset + loc.len as u64)
            .or(self.header_end)
            .unwrap_or(0)
    }

    /// Physically truncate anything after the last well-formed indexed line.
    ///
    /// Reopens the file read+write WITHOUT append: append-only handles on Windows carry
    /// only FILE_APPEND_DATA, which `set_len` rejects — and reindex needs read access.
    pub fn truncate_to_index(&mut self) -> Result<(), StoreError> {
        let good_end = self.keep_end();
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
    /// serves the parsed copy from [`SessionStore`]'s cache when it has one).
    pub fn read_entry(&self, seq: u64) -> Result<Option<SessionEntry>, StoreError> {
        if let Some(cached) = self.cached_entries.get(seq as usize) {
            return Ok(Some(cached.clone()));
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
            parent_id: self.leaf.id().map(str::to_string),
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
        self.leaf = Leaf::At {
            id: entry.id.clone(),
            seq: entry.seq,
        };
        self.next_seq += 1;
        Ok(entry)
    }

    /// Current leaf entry id (the tip of the active branch), or `None` when there
    /// is no leaf — an empty session, or one whose leaf was reset
    /// ([`SessionStore::reset_leaf`]): the next append starts a new root.
    pub fn leaf_id(&self) -> Option<String> {
        self.leaf.id().map(str::to_string)
    }

    /// The entry kinds this store kept **without understanding**, in file order.
    ///
    /// An entry whose `kind.type` this build does not know is preserved rather than refused
    /// ([`epsilon_contracts::session_entry::EntryKind::Unknown`]), so it never reaches the
    /// model view and never appears in a transcript — which makes it exactly the kind of
    /// silent absence a caller has to be able to report. A frontend that opens a session with
    /// a non-empty list here should say so, the same way it reports repairs.
    ///
    /// Only the entries the last scan parsed are inspected, which for a store that has since
    /// appended is a prefix of the file; an entry appended this session is written by this
    /// build and cannot be unknown.
    pub fn preserved_unknown(&self) -> Vec<(u64, String)> {
        self.cached_entries
            .iter()
            .filter_map(|entry| match &entry.kind {
                EntryKind::Unknown { type_name, .. } => Some((entry.seq, type_name.clone())),
                _ => None,
            })
            .collect()
    }

    /// Walk back from the leaf collecting the active branch (oldest first).
    ///
    /// Parses the file once; branch walks are infrequent (model view, navigation), so
    /// simplicity beats a reverse id->seq index until profiling says otherwise.
    ///
    /// Tolerant by design: this walk backs the model view and navigation, and a
    /// transcript whose chain is broken is still worth showing. It already stopped at a
    /// chain that ends on a missing parent; M91 adds the two hand-corruption shapes a
    /// file the store wrote can never contain, both of which used to be fatal — an
    /// infinite walk or an out-of-bounds index:
    ///
    /// - **Cycle** — a file whose entries form a parent cycle (a's parent is `b`, `b`'s
    ///   is `a`; pi's own walk is equally unbounded). The walk keeps a visited set and
    ///   stops the moment the next parent would re-enter an entry already on the chain,
    ///   returning the entries it legitimately reached, each once, in file order — the
    ///   same tolerant contract as a chain that ends early.
    /// - **`seq` mismatch** — the id → position map below is built from the entries'
    ///   STORED `seq` values, so in a hand-edited file whose `seq`s disagree with line
    ///   order a position can point past the end of the parsed vector (panic) or at a
    ///   different entry (a silently wrong branch). Every step now requires the entry at
    ///   that position to be the one the chain named; anything else counts as the same
    ///   broken chain and stops the walk.
    pub fn branch_entries(&self) -> Result<Vec<SessionEntry>, StoreError> {
        let all = self.read_all()?;
        let by_pos: std::collections::HashMap<&str, usize> = all
            .iter()
            .map(|e| (e.id.as_str(), e.seq as usize))
            .collect();

        let mut chain_rev: Vec<SessionEntry> = Vec::new();
        // M91: entries already on the chain — a repeat is a parent cycle, and the walk
        // stops there instead of looping forever.
        let mut on_chain: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let mut cursor = self.leaf_id();
        while let Some(id) = cursor {
            let Some(pos) = by_pos.get(id.as_str()).copied() else {
                break;
            };
            // M91: `pos` is `seq`-derived, not an index into `all` — resolve safely and
            // require the id to match, so a mismatched `seq` is a chain break, not a
            // panic or a wrong entry.
            let Some(entry) = all.get(pos).filter(|e| e.id == id) else {
                break;
            };
            if !on_chain.insert(entry.id.as_str()) {
                break;
            }
            cursor = entry.parent_id.clone();
            chain_rev.push(entry.clone());
        }
        chain_rev.reverse();
        Ok(chain_rev)
    }

    /// Remove the final entry (overflow-retry discard). Index-aware, O(1).
    ///
    /// This is a real product path, not test scaffolding: the agent loop calls it to
    /// discard the message that overflowed the context window before retrying
    /// (`epsilon_core`'s `pop_last_message`). Three things have to move together or the
    /// store starts lying about its own contents — the index, the read-through cache
    /// (see `cached_entries`), and the file.
    pub fn pop_last(&mut self) -> Result<Option<SessionEntry>, StoreError> {
        if self.next_seq == 0 {
            return Ok(None);
        }
        let last_seq = self.next_seq - 1;
        let entry = self.read_entry(last_seq)?;
        self.file.flush()?;
        self.index.pop();
        // The cache is positional, so shortening it to the index both drops the popped
        // entry and repairs a cache an earlier append left one short. Leaving it long was
        // a live defect: `read_entry` consults the cache first, so the next append got the
        // POPPED entry back for its own sequence number, and `branch_entries` — which
        // resolves the leaf id through `read_all` — found no leaf at all and reported an
        // empty transcript.
        self.cached_entries.truncate(self.index.len());
        self.next_seq = last_seq;
        self.leaf = match self.index.last() {
            Some(_) => match self.read_entry(last_seq - 1)? {
                Some(prev) => Leaf::At {
                    id: prev.id,
                    seq: prev.seq,
                },
                None => Leaf::None,
            },
            None => Leaf::None,
        };
        // Truncate the physical file to drop the popped line, keeping the header when
        // that line was the only entry (see `keep_end`).
        let good_end = self.keep_end();
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

    /// Read one entry's identity (`id`, `seq`) by id, or `None` when the file holds
    /// no such entry. The hash index narrows the search; the parsed id confirms it,
    /// so a hash collision can never navigate to the wrong entry.
    fn entry_identity(&self, entry_id: &str) -> Result<Option<(String, u64)>, StoreError> {
        let target_hash = hash_id(entry_id);
        for (s, loc) in self.index.iter().enumerate() {
            if loc.id_hash == target_hash {
                let entry = self.read_entry(s as u64)?.expect("indexed entry exists");
                if entry.id == entry_id {
                    return Ok(Some((entry.id, entry.seq)));
                }
            }
        }
        Ok(None)
    }

    /// Navigate: make `entry` the new leaf (pi's tree navigation). Everything after it stays
    /// in the file (human log intact) but leaves the active branch.
    ///
    /// Navigating to the position *before the first entry* — re-editing the first
    /// user message, pi's `resetLeaf` — is not an id and has no entry to name:
    /// use [`SessionStore::reset_leaf`] for that.
    pub fn navigate_to(&mut self, entry_id: &str) -> Result<(), StoreError> {
        match self.entry_identity(entry_id)? {
            Some((id, seq)) => {
                self.leaf = Leaf::At { id, seq };
                Ok(())
            }
            None => Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no such entry: {entry_id}"),
            ))),
        }
    }

    /// Reset the leaf to before the first entry (pi `resetLeaf`,
    /// `session-manager.ts:1372-1374`): the next append starts a new ROOT
    /// (`parent_id = None`) instead of chaining onto the old tip.
    ///
    /// This is the navigation `/tree` needs when the selected entry is the first
    /// user message: its parent is "the beginning of the session", which is a
    /// position rather than an id. Only the active branch moves — every existing
    /// entry stays in the file untouched (ADR-006) — and reopening the file
    /// restores the leaf from its last line exactly as before (a reset is
    /// in-memory state, like pi's, until the next append lands).
    pub fn reset_leaf(&mut self) {
        self.leaf = Leaf::None;
    }

    /// Branch from `branch_from` and attach a summary of the abandoned path
    /// (pi `branchWithSummary`, `session-manager.ts:1381-1405`).
    ///
    /// pi's two call shapes map one-to-one. `Some(id)` is
    /// `branchWithSummary(id, …)`: the leaf moves to that entry (validated first)
    /// and the summary is appended as its child. `None` is
    /// `branchWithSummary(null, …)`: the summary is a new ROOT — whatever leaf
    /// existed is abandoned and the entry records pi's `fromId = "root"`. A caller
    /// that wants the summary at the current leaf passes it explicitly
    /// (`store.branch_with_summary(store.leaf_id().as_deref(), …)`), which is what
    /// pi's callers do with the leaf id.
    ///
    /// The abandoned siblings stay in the file untouched (append-only, ADR-006).
    /// An unknown target is refused before any state moves: a failed navigation
    /// must not leave the leaf pointing somewhere the caller did not ask for.
    /// Returns the appended entry.
    pub fn branch_with_summary(
        &mut self,
        branch_from: Option<&str>,
        summary: impl Into<String>,
        usage: Option<Usage>,
    ) -> Result<SessionEntry, StoreError> {
        match branch_from {
            Some(target) => {
                let Some((id, seq)) = self.entry_identity(target)? else {
                    return Err(StoreError::Io(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("no such entry: {target}"),
                    )));
                };
                self.leaf = Leaf::At { id, seq };
            }
            // pi `branchWithSummary(null, …)`: attach at the root, abandoning the leaf.
            None => self.leaf = Leaf::None,
        }
        let from_id = self
            .leaf
            .id()
            .map(str::to_string)
            .unwrap_or_else(|| "root".to_string());
        self.append(EntryKind::BranchSummary {
            from_id,
            summary: summary.into(),
            usage,
        })
    }

    /// The path from root to `leaf_id`, oldest first, validated.
    ///
    /// Unlike [`SessionStore::branch_entries`] (which walks the active leaf and
    /// tolerates a chain that ends early), a fork must be exact: an unknown id is
    /// `NotFound`, and an ancestry that does not reach a root is `InvalidData`,
    /// because a partial path written out as a session file would be a silent lie
    /// about what the branch contains.
    ///
    /// M91: a parent CYCLE is refused the same way. The walk used to be unbounded —
    /// with the two entries pointing at each other it never reached a root and never
    /// terminated (pi's walk is equally unbounded); a hand-corrupted file must be
    /// refused by name instead. The refusal is `InvalidData` and names the entry the
    /// chain re-entered, so the report says which cycle to look for.
    fn branch_to(&self, leaf_id: &str) -> std::io::Result<Vec<SessionEntry>> {
        let all = self.read_all().map_err(std::io::Error::other)?;
        let by_id: std::collections::HashMap<&str, &SessionEntry> =
            all.iter().map(|e| (e.id.as_str(), e)).collect();
        let mut chain_rev: Vec<SessionEntry> = Vec::new();
        // M91: entries already on the chain (see the doc above).
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let mut cursor = Some(leaf_id.to_string());
        while let Some(id) = cursor {
            let Some(entry) = by_id.get(id.as_str()).copied() else {
                break;
            };
            if !seen.insert(entry.id.as_str()) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "entry {leaf_id} does not reach a root: its parent chain contains a \
                         cycle through {id}"
                    ),
                ));
            }
            cursor = entry.parent_id.clone();
            chain_rev.push(entry.clone());
        }
        if chain_rev.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no such entry: {leaf_id}"),
            ));
        }
        if let Some(missing) = chain_rev
            .last()
            .and_then(|e| e.parent_id.as_deref())
            .filter(|p| !by_id.contains_key(p))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("entry {leaf_id} does not reach a root: parent {missing} is missing"),
            ));
        }
        chain_rev.reverse();
        Ok(chain_rev)
    }

    /// Fork the session at `leaf_id`: write a NEW session file holding only the
    /// path from root to that entry (pi `createBranchedSession`,
    /// `session-manager.ts:1412-1491`), and return its path.
    ///
    /// The new file gets a fresh `ses_` id and a current-version header carrying
    /// the source's `cwd` and a `parent_session` link to the SOURCE FILE PATH (pi
    /// records the file the fork came from). Entries keep their `id`, `parent_id`,
    /// timestamp and payload; `seq` is renumbered gaplessly in path order, because
    /// `seq` is the file index and a gap would be read as a missing line. The
    /// source is a pure read: it is byte-identical when this returns, and every
    /// entry that is not on the selected path stays there (ADR-006 — forking
    /// copies a path, it never prunes history).
    ///
    /// The destination is written and then opened through the store's own open path
    /// (store.rs:273-275), so the fork is verified readable — and a torn tail would
    /// be repaired — before the path is handed back; an unusable destination is
    /// refused, never truncated. Failure paths clean up after themselves: a failed
    /// write or rename removes its temporary file, and a destination that fails its own
    /// verification pass is removed (its lease released first) — so an `Err` means
    /// nothing this call created is left behind, and anything that existed before the
    /// call is never touched.
    pub fn fork_branch_at(
        &self,
        leaf_id: &str,
        dest_dir: impl AsRef<Path>,
    ) -> std::io::Result<PathBuf> {
        let entries = self.branch_to(leaf_id)?;
        let cwd = self
            .header
            .as_ref()
            .map(|h| h.cwd.clone())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
        let dest_dir = dest_dir.as_ref();
        std::fs::create_dir_all(dest_dir)?;
        let timestamp = iso_now();
        let header = SessionHeader {
            kind: SessionHeaderKind::Session,
            version: Some(CURRENT_SESSION_VERSION),
            id: epsilon_contracts::new_id("ses"),
            timestamp: timestamp.clone(),
            cwd: cwd.clone(),
            parent_session: Some(self.path.to_string_lossy().into_owned()),
        };
        let dest = dest_dir.join(format!(
            "{}_{}.jsonl",
            timestamp.replace([':', '.'], "-"),
            header.id
        ));
        // Best-effort "do not clobber" guard, NOT a guarantee: the name carries a fresh
        // timestamp plus a fresh `ses_` id, so a collision needs the same millisecond
        // AND the same random id — and this check is inherently TOCTOU anyway. The
        // atomic step is the rename below; on Windows that replaces an existing file
        // rather than refusing, so this pre-check is courtesy, not protection.
        if dest.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "refusing to overwrite an existing session file: {}",
                    dest.display()
                ),
            ));
        }

        // Assemble the body in path order, `seq` renumbered gaplessly 0..n-1.
        let expected = entries.len() as u64;
        let mut body = String::new();
        body.push_str(&serde_json::to_string(&header).map_err(std::io::Error::other)?);
        body.push('\n');
        for (seq, mut entry) in entries.into_iter().enumerate() {
            entry.seq = seq as u64;
            body.push_str(&serde_json::to_string(&entry).map_err(std::io::Error::other)?);
            body.push('\n');
        }
        // Write-then-rename (the migration discipline): the fork appears whole.
        let tmp = dest.with_extension("jsonl.forking");
        if let Err(e) = std::fs::write(&tmp, &body) {
            // A failed write can leave a partial temp file; it is this call's own
            // scratch, so it must not survive the error.
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        if let Err(e) = std::fs::rename(&tmp, &dest) {
            // The destination was never created (the rename is the step that would
            // have created it), so only the temp file is ours to remove.
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }

        // Open it through the store's own path — the same reindex + torn-tail
        // repair every session file gets — and require the index to hold exactly
        // the entries written. A destination that fails either check is one this
        // call created moments ago: remove it, so `Err` reads as "nothing was
        // written" instead of leaving an unusable session on disk. Anything that
        // existed before this call is never touched (the pre-check refused it).
        let check = match SessionStore::open_in(&dest, Path::new(&cwd)) {
            Ok(check) => check,
            Err(e) => {
                let _ = std::fs::remove_file(&dest);
                return Err(std::io::Error::other(e));
            }
        };
        let verified = check.len();
        drop(check); // release the lease (it removes its own lock file)
        if verified != expected {
            let _ = std::fs::remove_file(&dest);
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "forked session {} reindexed to {verified} entries, expected {expected}",
                    dest.display()
                ),
            ));
        }
        Ok(dest)
    }

    /// The model-visible view of the active branch: message entries plus the synthetic
    /// summaries (compaction, branch summary) that belong to it; shadowed ranges are
    /// skipped entirely.
    ///
    /// A later compaction *replaces* the projection an earlier one built rather than
    /// appending to it: pi's `session-manager.ts:418-453` selects the latest compaction
    /// on the active path and builds context from it plus its retained entries. Without
    /// the replacement the superseded summary and its tail would ride along in every
    /// provider request while runtime history already held only the newer pair.
    ///
    /// A branch summary is projected where it sits on the path, exactly once, and only
    /// while its entry is on the ACTIVE branch (pi `session-manager.ts:401-403`):
    /// attached at the navigation target, it describes the path this conversation came
    /// back from, and the abandoned sibling it summarized contributes nothing. Like a
    /// message it waits in the buffer, so a later compaction that summarizes it away
    /// drops it — by then its material rides inside that compaction's retained window.
    /// An EMPTY summary text is not projected at all (pi's truthiness guard,
    /// `session-manager.ts:401`); the entry itself still lands in the file.
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
                EntryKind::BranchSummary {
                    from_id, summary, ..
                } => {
                    // pi's projection is truthiness-guarded (`session-manager.ts:401`:
                    // `entry.type === "branch_summary" && entry.summary`): an empty
                    // summary contributes no message at all. Only the empty string is
                    // falsy in that check — a whitespace summary still projects.
                    if !summary.is_empty() {
                        pending.push(ModelViewEntry::BranchSummary {
                            text: summary.clone(),
                            from_id: from_id.clone(),
                            seq: e.seq,
                        });
                    }
                }
                EntryKind::Compaction {
                    summary,
                    retained_tail,
                    ..
                } => {
                    // This compaction supersedes whatever projection exists so far:
                    // everything an earlier one synthesized is summarized inside this
                    // one, so keeping both would rebuild the context it just replaced.
                    out.clear();
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
    /// Convert to provider-visible messages (a summary becomes a user-role context message).
    ///
    /// The two summary kinds carry different tags so a host — or a human reading a
    /// request dump — can tell "the history before this point was compacted" from
    /// "this conversation came back from another branch", the distinction pi keeps
    /// with separate message roles (`messages.ts:100-120`).
    pub fn into_message(self) -> Message {
        match self {
            ModelViewEntry::Message(m) => m,
            ModelViewEntry::Summary { text, .. } => summary_message("compaction_summary", text),
            ModelViewEntry::BranchSummary { text, .. } => summary_message("branch_summary", text),
        }
    }
}

/// One synthetic summary message, wrapped the way the projection has always wrapped
/// a compaction summary (`<compaction_summary>`), with `tag` naming its source.
fn summary_message(tag: &str, text: String) -> Message {
    Message::User(UserMessage {
        id: epsilon_contracts::new_id("msg"),
        timestamp_ms: 0,
        content: vec![epsilon_contracts::content::UserContent::Text {
            text: format!("<{tag}>\n{text}\n</{tag}>"),
        }],
    })
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
        // The list of kinds this build knows decides which of the two sentences the reader
        // gets. Without it, an entry of a kind we understand but could not read was reported
        // as "probably written by a newer Epsilon" — the wrong conclusion, and the one
        // M108's Limits recorded as owed.
        return if epsilon_contracts::session_entry::KNOWN_ENTRY_KINDS.contains(&kind_type) {
            LineDiagnosis::MalformedEntry {
                entry_type: kind_type.to_string(),
            }
        } else {
            LineDiagnosis::UnknownEntryType {
                entry_type: kind_type.to_string(),
            }
        };
    }
    if value.get("session_id").is_some() || value.get("version").is_some() {
        return LineDiagnosis::SecondHeader;
    }
    if is_legacy_entry_shape(&value) {
        return LineDiagnosis::LegacyInCurrentFile;
    }
    // Whole JSON, but none of the shapes above: `"hello"`, `42`, `[]`, `{}`, or an
    // object with nothing this build recognizes in it. Not damage — a shape from
    // somewhere else — and the reader needs to be told which of the two it is.
    LineDiagnosis::UnknownLineShape
}

/// Is this line a complete JSON value, as opposed to a fragment of one?
///
/// The store repairs a crash-truncated final line by dropping it, and that is safe exactly when
/// the line never became a whole thought: a half-written value is debris, and removing it costs
/// the user nothing. A line that *is* whole JSON was written deliberately by something, so
/// deleting it would discard the user's last turn (or invent a transcript that never happened);
/// those are refused instead, with the file left alone.
///
/// Only the *syntax* is judged here. Whether the shape is one this build knows is the caller's
/// question, and [`diagnose_line`] has a typed answer for it.
fn is_whole_json(fragment: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(fragment.trim_end()).is_ok()
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
/// pi's v1 message lines carry the full `AgentMessage` under `message`, and the
/// `message` branch below converts that faithfully (see [`heal_legacy_message`]);
/// anything this build cannot express heals to a `Custom` entry preserving the
/// raw JSON (nothing silently dropped — the human log stays complete, and the
/// model view skips `Custom` by construction).
///
/// The entry timestamp is the line's own `timestamp`, parsed via
/// [`legacy_timestamp_ms`] — pi writes it as an ISO-8601 **string** there, while
/// Epsilon's own v1 files wrote epoch numbers, and both spellings are accepted.
fn heal_legacy_line(leg: &LegacyLine, ids: &[String]) -> SessionEntry {
    let map = leg.value.as_object();
    let kind = map.and_then(|m| m.get("type")).and_then(|t| t.as_str());
    let timestamp_ms = map
        .and_then(|m| m.get("timestamp"))
        .map_or_else(now_ms, legacy_timestamp_ms);
    // pi stamps a message line twice: an ISO string on the line and epoch milliseconds
    // inside the message, milliseconds apart. The millisecond one is the finer spelling of
    // that instant, so it becomes the entry's stamp too (`heal_legacy_message` reads the
    // same field), and the two can never disagree about when the turn happened.
    let timestamp_ms = map
        .and_then(|m| m.get("message"))
        .and_then(|m| m.get("timestamp"))
        .and_then(|t| t.as_u64())
        .unwrap_or(timestamp_ms);
    let id = epsilon_contracts::new_id("ent");
    let entry_kind = match kind {
        Some("message") => {
            // pi stamps the line and the message separately, and the message's own
            // `timestamp` (epoch millis) is the finer of the two, so it wins where pi wrote
            // one — see `heal_legacy_message`, which re-reads it. The entry and its message
            // then carry one instant; a healed transcript whose envelope and message
            // disagreed would be ordered by one value and displayed with another.
            EntryKind::Message {
                message: heal_legacy_message(map.and_then(|m| m.get("message")), timestamp_ms),
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
        // The two entry kinds whose pi shape lines up with an Epsilon variant field for
        // field, so healing them typed is lossless *and* applied: a pi session's model
        // switch and thinking-level change take effect on resume instead of sitting in a
        // `Custom` entry nothing reads. Verified against pi's own fixture lines
        // (`{"type":"model_change","provider":…,"modelId":…}`,
        // `{"type":"thinking_level_change","thinkingLevel":"high"}`); a line missing the
        // field it needs falls through to `Custom` below, which is still lossless.
        Some("model_change") if map.and_then(|m| m.get("modelId")).is_some() => {
            EntryKind::ModelChange {
                provider: map
                    .and_then(|m| m.get("provider"))
                    .and_then(|p| p.as_str())
                    .unwrap_or("")
                    .to_string(),
                model_id: map
                    .and_then(|m| m.get("modelId"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("")
                    .to_string(),
            }
        }
        Some("thinking_level_change")
            if map
                .and_then(|m| m.get("thinkingLevel"))
                .and_then(|l| l.as_str())
                .is_some() =>
        {
            EntryKind::ThinkingLevelChange {
                // Kept as pi's word: `ThinkingLevel` is parsed where the level is applied,
                // so a level this build does not know is visible rather than silently
                // clamped to `off`.
                level: map
                    .and_then(|m| m.get("thinkingLevel"))
                    .and_then(|l| l.as_str())
                    .unwrap_or("")
                    .to_string(),
            }
        }
        // `label` and `session_info` have Epsilon variants too, but no fixture here shows
        // pi's v1 spelling of them and both carry id references that a *positional* v1 line
        // may not be able to resolve; guessing would put a label on the wrong entry, so
        // they stay `Custom` (lossless, merely unapplied) until a real file settles it.
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

/// Heal one pi v1 `message` line's `message` object into an Epsilon [`Message`].
///
/// The entry-level kind is always `message`; the *role* inside chooses the variant, and
/// that is the whole point of this function: an importer that ignores the role turns a
/// transcript of tool calls into a transcript of things the user supposedly said.
///
/// Fidelity rules:
///
/// - **Content is an array of parts.** pi writes `[{"type":"text","text":...},
///   {"type":"toolCall",...}]`; Epsilon's own v1 files wrote a bare string. Both are
///   accepted, because a reader that understands only one of them silently empties the
///   other — which is the defect this replaces.
/// - **Nothing unrecognized is dropped.** A part this build cannot type (pi's `image`,
///   anything newer) is kept verbatim under [`LEGACY_EXTRAS_KEY`] rather than discarded.
///   A *user* message has no slot for that, so one carrying such a part heals to
///   [`Message::Custom`] with the whole object: losing the entry's LLM visibility is a
///   smaller harm than losing its content, and `Custom` is visible to the human log.
/// - **A role with no equivalent** (pi's `bashExecution`) heals to `Custom` for the same
///   reason: forcing it into `assistant` would put words in the model's mouth and into
///   `user` would put them in the user's.
/// - **Provider, model, stop reason and usage are copied when pi wrote them**, and left
///   empty/zero when it did not. Inventing a provider would be a worse lie than naming
///   none; a made-up token count would corrupt cost accounting.
/// - **The message's own `timestamp` wins over `fallback_ts`** (the entry's): pi writes
///   epoch milliseconds there and an ISO string on the line, so the message's is the finer
///   spelling of a nearby instant, and both it and the entry get that one value.
fn heal_legacy_message(raw: Option<&serde_json::Value>, fallback_ts: u64) -> Message {
    // A `message` line with no `message` object says "there is a message here" and then
    // carries nothing; an empty user turn is the only sane reading of it.
    let Some(msg) = raw.and_then(|m| m.as_object()) else {
        return Message::User(UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: fallback_ts,
            content: Vec::new(),
        });
    };
    let ts = msg
        .get("timestamp")
        .and_then(|t| t.as_u64())
        .unwrap_or(fallback_ts);
    let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
    let content = msg.get("content");
    let (blocks, extras) = legacy_content_blocks(content);
    let id = || epsilon_contracts::new_id("msg");

    match role {
        "assistant" => Message::Assistant(epsilon_contracts::message::AssistantMessage {
            id: id(),
            timestamp_ms: ts,
            provider: str_field(msg, "provider").unwrap_or_default(),
            model_id: str_field(msg, "model").unwrap_or_default(),
            content: blocks,
            stop_reason: match msg.get("stopReason").and_then(|s| s.as_str()) {
                Some("toolUse") => epsilon_contracts::message::StopReason::ToolUse,
                Some("length") | Some("maxTokens") => {
                    epsilon_contracts::message::StopReason::MaxTokens
                }
                Some("error") => epsilon_contracts::message::StopReason::Error,
                Some("aborted") => epsilon_contracts::message::StopReason::Aborted,
                // pi's word for "finished on its own", and the honest reading of a
                // message that carries no stop reason at all.
                _ => epsilon_contracts::message::StopReason::EndTurn,
            },
            error_message: str_field(msg, "errorMessage"),
            usage: msg.get("usage").map(legacy_usage).unwrap_or_default(),
        }),
        "toolResult" => Message::ToolResult(epsilon_contracts::message::ToolResultMessage {
            id: id(),
            timestamp_ms: ts,
            tool_call_id: str_field(msg, "toolCallId").unwrap_or_default(),
            tool_name: str_field(msg, "toolName").unwrap_or_default(),
            content: blocks,
            // `details` is this variant's own free-form slot, so a part pi wrote that
            // this build cannot type rides here and stays reachable.
            details: extras
                .map(|parts| serde_json::json!({ LEGACY_EXTRAS_KEY: parts }))
                .unwrap_or(serde_json::Value::Null),
            usage: None,
            is_error: msg
                .get("isError")
                .and_then(|e| e.as_bool())
                .unwrap_or(false),
        }),
        "user" if extras.is_none() => Message::User(UserMessage {
            id: id(),
            timestamp_ms: ts,
            content: blocks.into_iter().map(user_content_from_block).collect(),
        }),
        // `user` with an unmappable part, and every role Epsilon has no variant for.
        // The raw object goes in whole, so `data.role` still names what pi called it.
        _ => Message::Custom(epsilon_contracts::message::CustomMessage {
            id: id(),
            timestamp_ms: ts,
            custom_type: "legacy".into(),
            payload: serde_json::json!({"role": role, "message": msg}),
        }),
    }
}

/// Key under which content parts this build cannot type are preserved inside a healed
/// message.
const LEGACY_EXTRAS_KEY: &str = "legacy_unmapped_parts";

/// Convert one pi content array (or a v1 bare string) into typed blocks.
///
/// The parts both harnesses share become typed blocks; everything else comes back as the
/// second element, raw, for the caller to keep — pi's `image` is the plausible arrival,
/// and this build must not be the reason a transcript loses it.
fn legacy_content_blocks(
    content: Option<&serde_json::Value>,
) -> (Vec<ContentBlock>, Option<serde_json::Value>) {
    let mut parts: Vec<&serde_json::Value> = match content {
        Some(serde_json::Value::Array(parts)) => parts.iter().collect(),
        // Epsilon's own v1 spelling.
        Some(serde_json::Value::String(text)) => {
            return (vec![ContentBlock::Text { text: text.clone() }], None);
        }
        // No content at all is an empty message, not an unrecognized part.
        None | Some(serde_json::Value::Null) => return (Vec::new(), None),
        // A shape that is neither array nor string is preserved, not ignored.
        Some(other) => return (Vec::new(), Some(other.clone())),
    };
    let mut blocks = Vec::new();
    let mut extras = Vec::new();
    for part in parts.drain(..) {
        match legacy_block(part) {
            Some(block) => blocks.push(block),
            None => extras.push(part.clone()),
        }
    }
    let extras = match extras.len() {
        0 => None,
        _ => Some(serde_json::Value::Array(extras)),
    };
    (blocks, extras)
}

/// One content part as a typed block, or `None` when this build cannot express it.
///
/// A `text` part with no string or a `toolCall` with no id is `None` too: a typed block
/// with an empty field would be a quieter kind of loss than a preserved unknown.
fn legacy_block(part: &serde_json::Value) -> Option<ContentBlock> {
    match part.get("type").and_then(|t| t.as_str())? {
        "text" => Some(ContentBlock::Text {
            text: part.get("text")?.as_str()?.into(),
        }),
        // Epsilon models reasoning as first-class content; pi's spelling is a guess we
        // can afford, because a wrong guess degrades to "unmapped" rather than to loss.
        "thinking" => Some(ContentBlock::Thinking {
            text: part.get("thinking")?.as_str()?.into(),
        }),
        "toolCall" | "tool_call" => Some(ContentBlock::ToolCall {
            id: part.get("id")?.as_str()?.into(),
            name: part.get("name")?.as_str()?.into(),
            // No arguments means a call with none; an empty object keeps it executable.
            arguments: part
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        }),
        // Deliberately includes pi's `image`: Epsilon carries an image only as base64
        // (`ImageRef { mime, data }`), and guessing the encoding of a shape no fixture
        // here shows is how a conversion invents data. Preserved instead.
        _ => None,
    }
}

/// Fold one typed block into a user-message block.
///
/// [`UserContent`] has no tool-call variant, so the two block kinds a user message cannot
/// hold become text that names them rather than disappearing; a `user` message carrying
/// one never reaches here (see [`heal_legacy_message`]), so this is belt and braces.
fn user_content_from_block(block: ContentBlock) -> UserContent {
    match block {
        ContentBlock::Text { text } => UserContent::Text { text },
        ContentBlock::Image(image) => UserContent::Image(image),
        ContentBlock::Thinking { text } => UserContent::Text {
            text: format!("<thinking>\n{text}\n</thinking>"),
        },
        ContentBlock::ToolCall {
            id,
            name,
            arguments,
        } => UserContent::Text {
            text: format!("<tool_call id=\"{id}\" name=\"{name}\">{arguments}</tool_call>"),
        },
    }
}

/// One optional string field: `None` when absent or not a string.
fn str_field(map: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    map.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

/// pi's camelCase `usage` object as an Epsilon [`Usage`].
fn legacy_usage(value: &serde_json::Value) -> Usage {
    let n = |key: &str| value.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
    Usage {
        input: n("input"),
        output: n("output"),
        cache_read: n("cacheRead"),
        cache_write: n("cacheWrite"),
        total_tokens: n("totalTokens"),
    }
}

/// A legacy `timestamp` in either spelling pi and Epsilon have used.
///
/// - **ISO-8601 string** — what pi writes (`"2025-12-08T22:41:05.306Z"`), read as UTC with
///   any numeric offset applied rather than ignored.
/// - **Number** — what Epsilon's own v1 files wrote. Read as epoch *milliseconds*, or as
///   epoch *seconds* when it is small enough to be one: pi's own log holds both
///   (`"timestamp":1765233665292` in the header beside ISO strings in the entries). The
///   cutoff, 1e11 ms (1973) vs 1e11 s (year 5138), cannot misread a real session stamp.
///
/// Unparseable input has no honest answer, so the caller's `now_ms()` default stands: a
/// wrong timestamp is not a lost transcript, and zero would sort the entry to 1970.
fn legacy_timestamp_ms(value: &serde_json::Value) -> u64 {
    if let Some(s) = value.as_str() {
        return parse_iso8601_ms(s).unwrap_or_else(now_ms);
    }
    match value.as_u64() {
        Some(n) if n < 100_000_000_000 => n.saturating_mul(1000),
        Some(n) => n,
        None => now_ms(),
    }
}

/// Parse an ISO-8601 timestamp to epoch milliseconds, or `None` if it is not one.
///
/// Hand-rolled because the workspace carries no date crate and this direction is the one
/// nothing else needs (`iso_now` above writes rather than reads; epsilon-core's calendar
/// helper goes the other way). The day count is Howard Hinnant's `days_from_civil`, the
/// inverse of the one there.
///
/// `YYYY-MM-DDTHH:MM:SS[.frac][Z|±HH:MM]` is what pi writes. Anything else — including a
/// bare `"1765233665292"`, which `iso_now` can put in a header — names no date, so `None`
/// lets the caller fall back instead of inventing 1970.
fn parse_iso8601_ms(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    // The date and time fields are fixed-width, so byte slicing is index-safe.
    if b.len() < 19
        || b.get(4) != Some(&b'-')
        || b.get(7) != Some(&b'-')
        || b.get(10) != Some(&b'T')
    {
        return None;
    }
    let n = |from: usize, to: usize| s.get(from..to)?.parse::<i64>().ok();
    let (year, month, day) = (n(0, 4)?, n(5, 7)?, n(8, 10)?);
    let (hour, min, sec) = (n(11, 13)?, n(14, 16)?, n(17, 19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || min > 59 || sec > 60 {
        return None;
    }
    let mut rest = &s[19..];
    let mut millis = 0;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits: String = frac.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            return None;
        }
        rest = &frac[digits.len()..];
        // Truncate past the third digit: `.30` is 300 ms, `.3061` is 306 ms.
        let mut ms = digits;
        ms.truncate(3);
        while ms.len() < 3 {
            ms.push('0');
        }
        millis = ms.parse::<i64>().ok()?;
    }
    let offset_min = match rest {
        "" | "Z" | "z" => 0,
        _ => {
            let (sign, rest) = match rest.as_bytes().first() {
                Some(b'+') => (1, &rest[1..]),
                Some(b'-') => (-1, &rest[1..]),
                _ => return None,
            };
            let (h, m) = rest.split_once(':').unwrap_or((rest, "0"));
            sign * (h.parse::<i64>().ok()? * 60 + m.parse::<i64>().ok()?)
        }
    };
    let days = days_from_civil(year, month, day);
    let secs = days * 86_400 + hour * 3600 + min * 60 + sec - offset_min * 60;
    u64::try_from(secs * 1000 + millis).ok()
}

/// Days since 1970-01-01 for a Gregorian date (Howard Hinnant's `days_from_civil`).
///
/// Exact for every date a session file can hold; 719_468 is the day count from year 0 to
/// the epoch, the constant epsilon-core's calendar helper uses in the other direction.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400; // [0, 399]
    let mp = (m + 9) % 12; // Mar=0 … Feb=11
    let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
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
