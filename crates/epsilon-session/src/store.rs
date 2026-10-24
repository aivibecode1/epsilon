//! Session file store: append, branch navigation, and derived model view.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use epsilon_contracts::agent_event::TurnEndReason;
use epsilon_contracts::content::{ContentBlock, UserContent};
use epsilon_contracts::goal::{GoalChange, GoalSnapshot};
use epsilon_contracts::message::{Message, Usage, UserMessage};
use epsilon_contracts::session_entry::{
    CURRENT_SESSION_VERSION, EntryKind, SessionEntry, SessionHeader, SessionHeaderKind,
    SessionLine, SessionTreeNode,
};

use crate::lease::{LeaseOutcome, LeaseOwnership, SessionLease};

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

/// The model and thinking level a resumed session should come back on.
///
/// Port of pi's `getSessionContextSettings`
/// (`references/pi-0.85.1/packages/coding-agent/src/core/session-manager.ts:362-374`).
/// The type exists because `model_change` / `thinking_level_change` entries were durable
/// and typed but had **no reader**: they round-tripped perfectly and were then dropped,
/// so a session came back on whatever model the launch flag named. A defect that only
/// appears on resume, after a switch that worked, reads as intended behaviour.
///
/// The `Option` on `model` is pi's `null` and means something specific: the path never
/// ran a model. It is not "no preference" and must never be collapsed into a default,
/// or a caller cannot tell an unrun session from one that wants a different model.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionContextSettings {
    /// Last model seen on the selected path, from a `model_change` **or** an assistant
    /// message. `None` when the path has neither.
    pub model: Option<(String, String)>,
    /// Thinking level for the resumed session.
    ///
    /// `None` only before [`SessionStore::context_settings`] has run; the walk always
    /// yields a value, defaulting to pi's `"off"`, so a caller can distinguish "off"
    /// from "never set" without a second field.
    pub thinking_level: Option<String>,
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
    ///
    /// `detail` is the decoder's own sentence. The first version of this variant carried only
    /// the tag, which cost real time: the message read "its body cannot be read" and stopped
    /// there, while the decoder could have said ``missing field `cache_reported` `` — the
    /// field, the shape, and the fix in four words. A refusal that names a line but not a
    /// cause is a refusal the reader has to reproduce by hand.
    MalformedEntry {
        /// The `kind.type` string, which this build recognizes.
        entry_type: String,
        /// The decoder's own message naming the field or the type that did not match.
        detail: Option<String>,
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
            LineDiagnosis::MalformedEntry { entry_type, detail } => {
                // Two things produce this, and the reader has to be able to tell them
                // apart, because the actions differ: an **older writer** omitted a field
                // this build later added (upgrade the reader, or the writer), while a
                // **damaged** entry has a field of the wrong type or no body at all
                // (restore from a backup). The decoder's own sentence distinguishes them
                // — `missing field \`x\`` against `invalid type: …` — so it is quoted
                // rather than summarized.
                let cause = match detail {
                    Some(d) => format!(": {d}"),
                    None => String::new(),
                };
                format!(
                    "entry type `{entry_type}` is one this build knows, but its body cannot \
                     be read{cause} — either an older Epsilon wrote it without a field this \
                     build expects, or the entry is damaged. Refusing rather than preserving \
                     it, because an entry nothing can interpret would leave a silent gap in \
                     the transcript"
                )
            }
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
    /// A named entry does not exist in this file.
    ///
    /// Not a `Corrupt`: nothing here is malformed, and the line-number field
    /// would have to be a lie. pi throws `Entry ${targetId} not found`
    /// (`session-manager.ts:1247`) for exactly this case, so it is a real
    /// refusal in the reference too — M196 added it rather than overloading an
    /// unrelated variant.
    NoSuchEntry {
        /// The id that was asked for.
        id: String,
    },
    /// Another process holds this session's write lease (M61). Carries the
    /// holder's recorded identity (`pid=…`) when the lock file was readable.
    Leased {
        /// Holder line from the lock file, empty when unreadable.
        holder: String,
        /// How long acquisition waited before reporting the refusal (M136).
        waited: std::time::Duration,
    },
    /// **This store's own lease was taken by another process (M312).**
    ///
    /// Deliberately not [`StoreError::Leased`]. That one is `open` refusing to *acquire* a lease
    /// somebody else holds; this one is `append` refusing to *write*, on a store that opened
    /// legitimately and whose lock was taken afterwards. Continuing would interleave two writers
    /// into one file, which corrupts more than the message the refusal costs.
    ///
    /// **The refusal leaves the file untouched**, which is what `AGENTS.md` §6 requires of a
    /// refusal path, and it is reported rather than swallowed — every `append` caller surfaces it,
    /// which is the prerequisite M310 identified and M311 landed.
    LeaseLost {
        /// Holder line from the lock file.
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
            StoreError::NoSuchEntry { id } => {
                // pi's own wording (`session-manager.ts:1247`), so a message a
                // user sees here is the message pi would have shown them.
                write!(f, "entry {id} not found")
            }
            StoreError::Leased { holder, waited } => {
                let who = if holder.trim().is_empty() {
                    "holder unknown".to_string()
                } else {
                    holder.trim().to_string()
                };
                // Say how long we tried, and say plainly that the pid is a hint
                // rather than a verified fact. A user who killed a TUI and reopened
                // it deserves to know we already waited for the crash window — that
                // is the difference between a refusal they can act on and one that
                // looks like the tool is broken.
                let waited_note = if waited.is_zero() {
                    String::new()
                } else {
                    format!(" (waited {}s for it to be released)", waited.as_secs())
                };
                write!(
                    f,
                    "another epsilon is already writing this session ({who}){waited_note} — \
                     one writer per session file. If you just closed the other window, it is \
                     probably still shutting down; wait a moment and try again. Otherwise \
                     close it, or delete the .lock file beside the session: a live holder \
                     re-stamps it every few seconds, so a lock that stays put after that is \
                     a leftover, not a running instance.",
                )
            }
            StoreError::LeaseLost { holder } => {
                // **The message must say the file is intact and that this is not the
                // reader's fault.** The write was refused, so the file still holds everything up
                // to the last successful append; a reader who is told only "refused" will assume
                // the opposite, and the honest thing to do next is to open the session again —
                // which re-runs the lease acquisition and tells them who holds it.
                //
                // Two sentences for two facts. An empty holder is not a holder we failed to read:
                // it is `LeaseOwnership::Missing` — the lock file is gone — and **the wording must
                // stay true for every way that can happen.** Another epsilon took this session and
                // has since closed is one; a directory deleted under a running session (a
                // `TempDir` dropped by a test, a user clearing a sessions folder) is the other,
                // and the battery found that one. So the sentence offers both and names neither as
                // certain: a reader who deletes a lock to unstick a session must not be told a
                // rival exists.
                let who = match holder.trim() {
                    "" => "the .lock file beside it is gone — another epsilon may have taken this \
                           session and since closed it, or it was deleted"
                        .to_string(),
                    held => format!("another epsilon holds it ({held})"),
                };
                write!(
                    f,
                    "this session's write lease is no longer ours — {who}. The write was refused \
                     and the file is unchanged. Close any other window using this session, then \
                     reopen it to continue.",
                )
            }
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
    /// No active leaf: a session with no entries yet, so the next append is a new root.
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

pub(crate) fn now_ms() -> u64 {
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

/// One compaction's replayable digest: what a resumed transcript needs to redraw the
/// row the live path drew (P-054/P-183, replay half). Read from this branch's entries,
/// oldest first, so digest `i` is the digest for the `i`th `[context compacted]` marker
/// in the hydrated transcript — the same order both were written in. A whole-file read
/// would misattach across branches: billing is whole-file in `usage_accounts` because
/// money is owed wherever it was spent, but a digest is positional, so it follows the
/// branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionDigest {
    /// Summary text replacing the shadowed history.
    pub summary: String,
    /// Estimated tokens before compaction.
    pub tokens_before: u64,
    /// Usage of the summarization call(s), when recorded.
    pub usage: Option<Usage>,
    /// Catalog model id of the summarizer, when known.
    pub model_id: Option<String>,
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
    ///
    /// **Read, not merely held, since M312.** It was `_lease` — held for its `Drop`, never
    /// consulted — which is why a holder whose lock was taken went on appending. Every write
    /// now asks [`SessionLease::ownership`] first.
    lease: SessionLease,
    /// Armed fault plan (F07): the next matching write fails with this message instead of
    /// touching the file, index or cache. Empty — the state production code is always in —
    /// leaves the write path exactly as it was.
    ///
    /// This is the store-level fault-injection seam M86's `Limits` said the append-failure
    /// branches needed and did not have. It exists because a *durable write failure* is not
    /// reachable by any honest test on this machine: the handle is already open, so a
    /// read-only file, a revoked directory permission and a non-writable attribute all still
    /// accept writes through it. Injecting at the write path is the only way to test what the
    /// caller does when the disk says no — see [`SessionStore::set_write_fault_for_tests`].
    ///
    /// A list, not one slot: a scenario can need two different writes to fail in one run
    /// (a prompt append and the overflow retry's pop, say), and arming the second must not
    /// silently disarm the first.
    faults: Vec<WriteFault>,
    /// [`SessionStore::append`] calls made on this store, refused ones included
    /// ([`SessionStore::appends`]).
    ///
    /// Exists because a fault is armed as a *count* of matching writes, so aiming one at "the
    /// write that happens second" is a claim about how many appends the caller makes first —
    /// and a claim a test could only assert from inside the store. Without this, a write
    /// inserted ahead of the one under test moves the fault onto the wrong append and the
    /// scenario proves nothing while still passing or hanging (the F07 salvage test cost the
    /// battery a 420-second round for exactly that reason). One `usize` in the struct every
    /// store carries; the append path pays an increment.
    appends: usize,
}

/// A pending injected write failure ([`SessionStore::set_write_fault_for_tests`]).
#[derive(Debug, Clone)]
struct WriteFault {
    /// Which write the injection applies to.
    op: FaultOp,
    /// The operation's own error text.
    message: String,
    /// Which matching write starts failing, counting from `1`. The first `occurrence - 1` are
    /// let through untouched.
    ///
    /// This is what makes "the *second* append fails" expressible, and the F07 salvage test is
    /// why it has to be: the prompt append happens before every other write of a run, so a plan
    /// that starts at the first append always kills the run at the prompt (F07 makes a failed
    /// prompt append fatal) and never reaches the write under test. A count of failures cannot
    /// say that; a position can.
    occurrence: usize,
    /// How many more matching operations fail. `usize::MAX` sticks for the store's life.
    remaining: usize,
    /// Matching operations already seen, so [`WriteFault::occurrence`] can be honoured.
    seen: usize,
}

/// Which write [`SessionStore::set_write_fault_for_tests`] injects a failure into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultOp {
    /// [`SessionStore::append`] — the write every durable-commit path goes through.
    Append,
    /// [`SessionStore::pop_last`] — the overflow-retry discard.
    PopLast,
}

impl WriteFault {
    /// Apply the plan armed for `op`, consuming one failure when one matches.
    ///
    /// `None` means "let the real write happen": no plan for this operation, or one that has
    /// spent its failures. A spent plan is removed, so a test that arms one failure cannot
    /// leave the store failing for the assertions after it — and plans for other operations
    /// are left alone, so arming the pop does not disarm the append.
    ///
    /// A plan is *counted* for every matching write and *fires* from its `occurrence` on: the
    /// positions before it are the ones a test needs to succeed for the run to reach the write
    /// it is aiming at. The counter therefore advances whether or not the write fails, which is
    /// the whole point — an earlier write must not shift the target.
    fn take_for(faults: &mut Vec<WriteFault>, op: FaultOp) -> Option<StoreError> {
        let at = faults.iter().position(|f| f.op == op)?;
        faults[at].seen += 1;
        if faults[at].seen < faults[at].occurrence {
            return None;
        }
        faults[at].remaining = faults[at].remaining.saturating_sub(1);
        let spent = faults[at].remaining == 0;
        let message = faults[at].message.clone();
        if spent {
            faults.remove(at);
        }
        Some(StoreError::Io(std::io::Error::other(message)))
    }
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
        Self::open_with_parent(path, cwd, None, |_, _| {})
    }

    /// [`SessionStore::open_in`], reporting each poll of the lease wait through
    /// `on_wait(waited, remaining)`.
    pub fn open_in_with_progress(
        path: impl AsRef<Path>,
        cwd: &Path,
        on_wait: impl Fn(std::time::Duration, std::time::Duration),
    ) -> Result<Self, StoreError> {
        Self::open_with_parent(path, cwd, None, on_wait)
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
        Self::open_with_parent(path, cwd, Some(parent_session), |_, _| {})
    }

    /// Shared constructor: `parent` lands in the header of a brand-new file.
    fn open_with_parent(
        path: impl AsRef<Path>,
        cwd: &Path,
        parent: Option<&str>,
        on_wait: impl Fn(std::time::Duration, std::time::Duration),
    ) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            // Session transcripts carry the conversation and any secret a tool
            // printed. They live under the workspace, where a broad `git add -A`
            // or a shared checkout can read them; owner-only is the floor.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
            }
        }
        // M61: one writer per session file. The check is `create_new` on
        // `<file>.lock` (atomic on NTFS and POSIX), and the holder heartbeats it
        // so a live process is never mistaken for a crash leftover.
        //
        // M136: `acquire` waits out the stale window before giving up, because
        // refusing immediately made the ordinary way to restart a TUI fail — kill
        // it, reopen, and get "another epsilon is already writing this session"
        // for up to 30 seconds. `waited` is carried into the error so the message
        // can say how long we tried, which is the difference between a user who
        // understands the refusal and a user who thinks the tool is broken.
        let lease = match SessionLease::acquire_with_progress(
            &path,
            crate::lease::STALE_AFTER,
            crate::lease::HEARTBEAT,
            crate::lease::ACQUIRE_WAIT,
            on_wait,
        )? {
            LeaseOutcome::Acquired(lease) => lease,
            LeaseOutcome::Held { holder, waited } => {
                return Err(StoreError::Leased {
                    holder: holder.unwrap_or_default(),
                    waited,
                });
            }
        };
        let preexisting = path.exists();
        let mut options = OpenOptions::new();
        options.read(true).append(true).create(true);
        // Owner-only at creation, not after it: the mode is part of the open.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path)?;
        // Tighten a file that already existed (an older session may have been
        // created 0644); `mode` above only covers the create case.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }

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
            lease,
            faults: Vec::new(),
            appends: 0,
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

    /// Refuse a write when this store is no longer the session's writer (M312).
    ///
    /// The other half of the acquisition in [`SessionStore::open_with_parent`]: that call decides
    /// *once* whether this process may write, which is enough to open with and not enough to
    /// keep writing — a lease can be taken at any moment afterwards, and until now nothing looked.
    ///
    /// **The two refusals are not the same decision, and that is why this lives in `lease.rs`.**
    /// A lock carrying someone else's token is evidence; a lock that cannot be read this instant is
    /// not, and `LeaseOwnership` is what tells them apart. Writing through the second would discard
    /// a message the caller has already been told was saved, over a sharing violation that lasts a
    /// millisecond.
    ///
    /// **The file is left exactly as it was**, which is what a refusal owes the reader and what
    /// `AGENTS.md` §6 asks of one: the check runs before any write, and every caller of
    /// [`SessionStore::append`] has a way to surface the error, so a refusal is reported rather
    /// than swallowed into a lost turn.
    fn refuse_if_lease_lost(&self) -> Result<(), StoreError> {
        match self.lease.ownership() {
            LeaseOwnership::Ours | LeaseOwnership::Unreadable => Ok(()),
            // An **empty** holder is `Missing`: the lock file is gone. `Display` carries its
            // own wording for that, because an empty holder is not "nobody left to name" — it
            // is two causes that read identically here, and it must assert neither.
            LeaseOwnership::Taken { holder } => Err(StoreError::LeaseLost { holder }),
            LeaseOwnership::Missing => Err(StoreError::LeaseLost {
                holder: String::new(),
            }),
        }
    }

    /// The session header (`None` only for an empty file with no entries yet).
    pub fn header(&self) -> Option<&SessionHeader> {
        self.header.as_ref()
    }

    /// The session's display name: the newest `Custom { "session_name" }` entry's `name`.
    ///
    /// The newest one wins, because `/name` and RPC `set_session_name` append and the
    /// entry can sit anywhere in the file — reading the first would report the name the
    /// session *started* with. This is the same rule pi's session listing applies, and it
    /// is the reader half of the writer in the `set_session_name` handler, which is why it
    /// lives here rather than in either frontend: a reader and a writer that disagree
    /// about the entry's shape would show a name the RPC could not have set.
    ///
    /// `None` when the session has never been named, which is a normal state and not an
    /// error — an unnamed session is what most sessions are.
    pub fn session_name(&self) -> Option<String> {
        let entries = self.read_all().ok()?;
        entries.iter().rev().find_map(|e| match &e.kind {
            EntryKind::Custom {
                custom_type,
                data: Some(data),
            } if custom_type == "session_name" => data
                .get("name")
                .and_then(|n| n.as_str())
                .map(str::to_string),
            _ => None,
        })
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

    /// Arm an injected write failure starting at the `occurrence`-th matching write, for `times`
    /// writes (F07).
    ///
    /// The seam M86's `Limits` recorded as missing: a caller's handling of a *failed durable
    /// write* cannot be tested by making the file unwritable, because the store's handle is
    /// already open and a read-only file, a revoked directory permission and a non-writable
    /// attribute all still accept writes through an open handle. Injecting at the write path
    /// is the only honest way to reach those branches.
    ///
    /// The injection is **before the write and after nothing**: the file, the index, the leaf
    /// and the read cache are left exactly as they were, so an injected failure has the shape
    /// of a write that never started — which is what a store that refuses a write reports, not
    /// a half-written line. The `Err` is a [`StoreError::Io`] whose message is `message`, so a
    /// test can assert on the text a caller would have to relay.
    ///
    /// `occurrence` is **which** matching write is the first to fail, counting from `1`, and
    /// `times` is **how many** fail from there (`usize::MAX` keeps failing). The first
    /// `occurrence - 1` matching writes are let through untouched, which is what makes a failure
    /// aimed at a *later* write expressible at all: every run appends its prompt before anything
    /// else, and since F07 a failed prompt append ends the run, so a plan that starts at the
    /// first append can never reach the second — the F07 salvage test needs "the second append
    /// fails, the first succeeds", and only the position can say that. `times: 0` arms nothing.
    ///
    /// Writes to the *other* operation are never affected, and arming a second operation does not
    /// disarm the first, so one run can have both an append and a pop fail.
    ///
    /// Test-facing by construction — nothing in the product calls it, and an empty list (the
    /// value every store is born with) is the entire production behaviour.
    #[doc(hidden)]
    pub fn set_write_fault_for_tests(
        &mut self,
        op: FaultOp,
        message: &str,
        occurrence: usize,
        times: usize,
    ) {
        if times == 0 {
            // Arm nothing at all: a plan that can never fire would otherwise sit in the list
            // forever, making `write_fault_pending_for_tests` report a pending failure that
            // cannot happen — the opposite of what that check is for.
            return;
        }
        self.faults.push(WriteFault {
            op,
            message: message.to_string(),
            occurrence,
            remaining: times,
            seen: 0,
        });
    }

    /// How many [`SessionStore::append`] calls reached the write path, armed-fault
    /// refusals included (F07's test seam).
    ///
    /// Counts *attempts*, not lines on disk, so it is the position an `occurrence` argument
    /// names: a test that arms a failure for "the second append" asserts this counter to prove
    /// the write it aimed at is the write that happened, rather than assuming the caller makes
    /// exactly one append first.
    ///
    /// **A lease refusal is deliberately not counted** (M312): it happens before the counter
    /// because nothing was written and nothing was attempted, and a test that wants to know a
    /// refused store is still refusing can read the unchanged count.
    #[doc(hidden)]
    pub fn appends(&self) -> usize {
        self.appends
    }

    /// Whether an armed fault is still pending — i.e. the caller never made the write it was
    /// armed for, so the failure a test expected never happened.
    #[doc(hidden)]
    pub fn write_fault_pending_for_tests(&self) -> bool {
        !self.faults.is_empty()
    }

    /// Rewrite the file with the migrated entries plus a current header
    /// (atomic tmp+rename, same discipline as the auth store).
    ///
    /// **A conversion leaves the original beside the file**, at the session path with
    /// `.pre-epsilon` appended (`s.jsonl` → `s.jsonl.pre-epsilon`). A pre-v2 file is
    /// often the only copy of that conversation in a format its own tool can still read
    /// — converting it in place is correct and, without this, unrecoverable. The copy is
    /// written once and never overwritten: an existing `.pre-epsilon` is the true
    /// original from an earlier conversion, so a second pass over an already-converted
    /// file must not replace it with a converted copy. A file this build can already read
    /// converts nothing, so it grows no copy — the ordinary path stays clean.
    ///
    /// The copy is taken *after* the new content is complete in the temp file and
    /// *before* the rename, so a failure on either side leaves nothing behind; a failed
    /// copy fails the migration rather than converting a file without a way back.
    pub fn persist_migration(&mut self) -> Result<(), StoreError> {
        // **The other writer, and it is the more dangerous of the two (M312).** A migration does
        // not add a line: it rewrites the whole file and renames it over the top. A store whose
        // lease was taken would therefore not interleave with the other writer — it would
        // *replace* its file, taking every entry that writer appended since with it. `append`
        // checks first, so this only matters for the direct callers (`build_runtime`'s launch and
        // resume paths), which is exactly why the check is here too rather than only there.
        self.refuse_if_lease_lost()?;
        // Only a file that was actually migrated has an original worth keeping: this is the
        // same flag that says the entries in hand are healed ones rather than the file's own.
        let converting = self.migrated;
        let entries = self.read_all()?;
        let header = self.header.clone().unwrap_or_else(|| SessionHeader {
            kind: SessionHeaderKind::Session,
            version: Some(CURRENT_SESSION_VERSION),
            id: epsilon_contracts::new_id("ses"),
            timestamp: iso_now(),
            cwd: String::new(),
            parent_session: None,
            preserved: BTreeMap::new(),
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
        // Keep the previous bytes before the conversion lands. Skipped when nothing is being
        // converted, so a current file never grows a sibling it has no use for.
        if converting && let Err(e) = keep_original_beside(&self.path) {
            // Fail rather than convert without a way back. The temp file is this call's own
            // scratch and must not survive the error (the fork's rule).
            let _ = std::fs::remove_file(&tmp);
            return Err(StoreError::Io(e));
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
            preserved: BTreeMap::new(),
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
        // **Refuse an oversized file before reading it.** `read_to_string`
        // would allocate the whole thing first; a repo-shipped or shared
        // multi-gigabyte `.jsonl` should get a sentence, not take the process
        // down. The refusal leaves the file untouched.
        let len = self.file.metadata()?.len();
        if len > crate::MAX_SESSION_FILE_BYTES {
            return Err(StoreError::Io(std::io::Error::other(format!(
                "session file is {len} bytes; refusing to load more than {} bytes",
                crate::MAX_SESSION_FILE_BYTES
            ))));
        }
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
                    //
                    // The error itself is not carried out of here: `SessionLine` is an
                    // *untagged* enum, so it reads `data did not match any variant`, which
                    // names nothing. `diagnose_line` re-decodes the same value through the
                    // typed entry parser to recover the field-level sentence.
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
                                // not reported as damage to a file that is perfectly intact, and
                                // so a body that failed to decode names the field it wanted.
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
    ///
    /// A short read on an indexed sequence is re-worded before it leaves here. The index is
    /// built once at open and never re-checked, so a file that shrank underneath a live store
    /// reads short; the raw `UnexpectedEof` names no entry, no sequence and no path, which is
    /// how a shrunken session becomes indistinguishable from a failing disk. The refusal names
    /// all three and leaves the file intact — the repair path's job, not this function's.
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
        f.read_exact(&mut buf).map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                // A whole buffer short means the file no longer holds what the index recorded:
                // name the entry and the path rather than reporting an anonymous EOF.
                StoreError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    format!(
                        "index and file disagree: entry #{seq} is in the index but not in {} \
                         — the file is shorter than the last scan recorded",
                        self.path.display()
                    ),
                ))
            } else {
                StoreError::Io(e)
            }
        })?;
        let line = String::from_utf8_lossy(&buf);
        Ok(Some(serde_json::from_str(line.trim_end()).map_err(
            |_| {
                // Reading one entry back: diagnose it the same way a scan would, so the
                // message names the cause rather than only the line.
                StoreError::corrupt(seq as usize + 1, diagnose_line(&line))
            },
        )?))
    }

    /// Whether an entry with this id exists in the file.
    ///
    /// Scans the whole file, which is what makes it honest: a label can point at
    /// an entry anywhere in the session, including on a branch the reader is not
    /// currently on, so a lookup against the active lane would refuse a valid
    /// target.
    pub fn has_entry(&self, id: &str) -> Result<bool, StoreError> {
        Ok(self.read_all()?.iter().any(|e| e.id == id))
    }

    /// Set or clear a **label** on an entry — pi's `appendLabelChange`
    /// (`session-manager.ts:1245-1267`).
    ///
    /// **M196: `EntryKind::Label` existed and was serialized, and nothing ever
    /// wrote or read one.** That is this repository's "faithful port, no
    /// production caller" class for the sixth time, and it is the reason the wire
    /// tree reported `label: null` for every node: the field was not missing
    /// data, it was a hardcoded constant standing in for a feature that had no
    /// writer. Reading pi's rule is what made the shape obvious — the label is
    /// carried **on the entry itself** (`target_id` + `label`), so it survives a
    /// reload, which a sidecar map would not.
    ///
    /// Two behaviours from pi that are easy to lose:
    ///
    /// - **An empty label clears** (`label: None`), it does not write an empty
    ///   string. pi's loader branches on `if (entry.label) ... else delete`, so a
    ///   written-but-empty label would clear the label *and* leave the entry in
    ///   the file saying so. Passing `None` writes that entry deliberately,
    ///   which is what pi does and what makes the clear auditable.
    /// - **The label is recorded ON the target; the `Label` entry itself lands on
    ///   the current lane and becomes the new leaf.** M196: I first documented
    ///   and tested the opposite — that labelling does not move the leaf — and
    ///   the test caught it. pi does exactly the same thing: `_appendEntry`
    ///   sets `leafId = entry.id` unconditionally
    ///   (`session-manager.ts:1058-1063`), so `appendLabelChange` moves the leaf
    ///   in pi too. Matching the reference beats my intuition about what *should*
    ///   happen, and the consequence is that the TUI re-projects after labelling
    ///   (it does), rather than pretending nothing moved.
    ///
    /// Returns the id of the entry that records the change, and refuses an
    /// unknown `target_id` the way pi does (`throw new Error(\`Entry ... not
    /// found\`)`) rather than writing a label pointing at nothing.
    pub fn set_label(
        &mut self,
        target_id: &str,
        label: Option<&str>,
    ) -> Result<String, StoreError> {
        if !self.has_entry(target_id)? {
            return Err(StoreError::NoSuchEntry {
                id: target_id.to_string(),
            });
        }
        // Normalise "" to a clear, so a caller cannot write a label that pi's own
        // loader would read as a clear while Epsilon's reader read it as a label.
        let label = label.filter(|s| !s.trim().is_empty());
        let entry = self.append(EntryKind::Label {
            target_id: target_id.to_string(),
            label: label.map(str::to_string),
        })?;
        Ok(entry.id)
    }

    /// The session's current goal, folded from its durable changes in file order.
    ///
    /// Mirrors [`SessionStore::labels`] in shape and in reason: the log is the only authority,
    /// so the answer is derived rather than stored, and a goal written by another frontend —
    /// or by an earlier build — folds the same way.
    ///
    /// **The fold is order-driven and lenient about sequence, strict about shape.** Shape is
    /// serde's job and a body it cannot read is refused at open (M108). Sequence is different:
    /// a `Set` whose revision does not follow the previous one is *applied* — the revision that
    /// matters is the one a mutation's compare-and-set is checked against, and refusing to
    /// derive a state because a number skipped would turn a writer's bug into an unopenable
    /// session. What the fold will not do is apply a change that contradicts the log's own
    /// story: a `Cleared` removes the goal only when it names the goal that is current, so a
    /// late tombstone cannot delete a newer objective.
    ///
    /// A goal is *not* scoped to the active branch, deliberately. Branch navigation rewinds the
    /// conversation; it does not un-set the objective a person typed, and the alternative —
    /// a goal that disappears because the user looked at another branch — is the kind of
    /// surprise that makes a feature untrustworthy. `labels` is scoped the same way for the same
    /// reason.
    pub fn goal(&self) -> Result<Option<GoalSnapshot>, StoreError> {
        let mut current: Option<GoalSnapshot> = None;
        for e in self.read_all()? {
            let EntryKind::Goal { change } = e.kind else {
                continue;
            };
            match change {
                GoalChange::Set { goal } => current = Some(goal),
                GoalChange::Cleared { id, .. } => {
                    if current.as_ref().is_some_and(|g| g.id == id) {
                        current = None;
                    }
                }
            }
        }
        Ok(current)
    }

    /// Append a goal change, stamped by the store like every other entry.
    ///
    /// A thin wrapper so a caller cannot write the entry with a hand-made kind by mistake: the
    /// only way to change a goal is to write one of these, and the only reader is
    /// [`SessionStore::goal`].
    pub fn append_goal(&mut self, change: GoalChange) -> Result<SessionEntry, StoreError> {
        self.append(EntryKind::Goal { change })
    }

    /// Resolve the label for every entry, in file order, as pi's loader does.
    ///
    /// pi keeps `labelsById` as a map rebuilt on load (`session-manager.ts:975-990`)
    /// and reads it in `getTree`. This returns the resolved map rather than
    /// exposing pi's mutable field, so a caller cannot put the map and the file
    /// out of step — which is the whole failure this design avoids.
    ///
    /// **Last write wins**, including a clear: scanning in order and applying each
    /// `Label` entry reproduces pi exactly, so a session written by pi and one
    /// written by Epsilon resolve identically.
    pub fn labels(&self) -> Result<BTreeMap<String, (String, u64)>, StoreError> {
        let mut out: BTreeMap<String, (String, u64)> = BTreeMap::new();
        for e in self.read_all()? {
            if let EntryKind::Label { target_id, label } = e.kind {
                match label {
                    Some(text) => {
                        out.insert(target_id, (text, e.timestamp_ms));
                    }
                    None => {
                        out.remove(&target_id);
                    }
                }
            }
        }
        Ok(out)
    }

    /// Append a new entry on the current lane; assigns seq/parent/timestamp/id and flushes.
    ///
    /// M41: the first append to a headerless preexisting file stamps the header
    /// first (lazy creation — `open` only stamps brand-new files, so a resumed
    /// empty file and a fresh file converge on the same bytes).
    ///
    /// **M312: refuses once this store's own lease is no longer its own.** Every write path in
    /// the product funnels through here — `append_goal`, `label`, `record_turn_end` and the
    /// branch summaries are all this function with a different [`EntryKind`] — so one check covers
    /// them all rather than five, and the refusal cannot be routed around by a caller that picked
    /// a typed wrapper.
    pub fn append(&mut self, kind: EntryKind) -> Result<SessionEntry, StoreError> {
        // **First, and before the counter.** A refusal is not a write attempt: it must not
        // consume one of a fault plan's `times`, and `appends()` must stay a count of what was
        // written. It is also before everything else in this function on purpose — the lazy
        // header stamp and the migration both write, so a check further down would let a store
        // whose lock is gone rewrite the whole file first.
        self.refuse_if_lease_lost()?;
        // Counted before the fault check: the seam counts *attempts*, which is what a `times`
        // argument is spent against (F07 — see [`SessionStore::appends`]).
        self.appends += 1;
        if let Some(e) = WriteFault::take_for(&mut self.faults, FaultOp::Append) {
            return Err(e);
        }
        if self.header.is_none() {
            // **The copy comes first, and the order is the whole point (M291).**
            //
            // This branch stamps a header onto a file that has none, and on a migrating file it
            // then converts the file — and the conversion is what takes the `.pre-epsilon`
            // copy. Taken after the stamp, the copy captured `original + header`: the handle is
            // `O_APPEND` (`open_with_parent`), so on a file that already had entries the header
            // line landed at EOF rather than at offset 0, and a header is legal only as the
            // **first** line. The one artefact meant to be the way back from the conversion
            // therefore opened as `Corrupt { SecondHeader }` — a recovery copy nothing could
            // recover with. `a_backup_taken_from_append_is_a_file_this_build_can_open` is the
            // test that failed on exactly that, byte for byte.
            //
            // Taking it here changes no other byte: `persist_migration` calls
            // `keep_original_beside` again a few lines later, and that is a no-op because the
            // copy is already there (the once-only rule inside `keep_original_beside`). It also
            // moves the failure earlier — a copy that cannot be taken now fails *before* the
            // header is stamped, so the session file is left exactly as it was found rather
            // than carrying a header it has no conversion for.
            if self.migrated {
                keep_original_beside(&self.path).map_err(StoreError::Io)?;
            }
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
        // **`flush()` on a `File` is a no-op**, which is the whole content of this line's
        // predecessor: `std::fs::File` is unbuffered, so `Write::flush` has nothing to push —
        // it is not "durability", it is a no-op that reads like durability. Everything
        // `AGENTS.md` promises about a resumable session therefore rested on the OS page
        // cache surviving a power cut, and it does not: an acknowledged user turn can be
        // gone after a hard reset, and the repair path then silently drops the torn tail,
        // which takes the rest of that turn with it.
        //
        // Hermes fsyncs for the same reason and says why in one line
        // (`batch_runner.py:295`: "so a crash never loses an acknowledged prompt").
        //
        // `sync_data` rather than `sync_all` deliberately: it is `fdatasync`, which skips the
        // inode metadata a fresh file would want and a growing append mostly does not. On
        // Windows the two are the same call, and the cost is a few milliseconds per append --
        // paid only when an entry is actually written, which is why it is here rather than in
        // a flush the caller has to remember.
        self.file.sync_data()?;

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
    /// is no leaf — an empty session. A session with entries always has one.
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

    /// One line naming the entry kinds this store kept without understanding, or `None` when it
    /// kept none — the sentence a frontend prints after it opens the session.
    ///
    /// [`SessionStore::preserved_unknown`] is the fact; this is the sentence, for the same reason
    /// [`crate::repair::RepairKind::describe`] sits beside [`crate::repair::RepairReport`]: a
    /// report each frontend has to phrase for itself is a report that ends up unprinted, and a
    /// preserved entry is invisible by construction — it never reaches the model view, so a
    /// transcript missing one looks exactly like a complete one.
    ///
    /// The kinds are deduplicated **in file order**, because the question the line answers is
    /// "what is missing from this session" and the answer is a set of kinds, not one line per
    /// entry. `None` rather than an empty string: "nothing was kept" is the absence of a report,
    /// and a caller that prints it anyway would announce nothing.
    pub fn preserved_unknown_notice(&self) -> Option<String> {
        let kept = self.preserved_unknown();
        if kept.is_empty() {
            return None;
        }
        // `kept` is already in file order, so first-seen order is file order.
        let mut kinds: Vec<&str> = Vec::new();
        for (_, kind) in &kept {
            if !kinds.contains(&kind.as_str()) {
                kinds.push(kind.as_str());
            }
        }
        let list = kinds
            .iter()
            .map(|kind| format!("`{kind}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let n = kept.len();
        Some(if n == 1 {
            format!(
                "1 entry of a kind this build does not know ({list}) is in the file but not in \
                 the transcript — kept, not dropped"
            )
        } else {
            format!(
                "{n} entries of kinds this build does not know ({list}) are in the file but not \
                 in the transcript — kept, not dropped"
            )
        })
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
        if let Some(e) = WriteFault::take_for(&mut self.faults, FaultOp::PopLast) {
            return Err(e);
        }
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
    ///
    /// **A file that no longer matches the index is a named refusal** — not a panic, not a
    /// silent skip, and not an anonymous `unexpected end of file`.
    ///
    /// The index is built once, at open, and never re-checked, so anything that shortens the
    /// file behind a live store — another process truncating it, a `set_len` repair cutting too
    /// far — leaves the scan's picture of the tail pointing past what is actually there. Three
    /// wrong answers are within reach, and the pre-fix code took two of them at once:
    ///
    /// - `.expect("index and file agree")` killed the process from inside a library;
    /// - skipping the missing entry reported a transcript shorter than the one the user wrote,
    ///   as though nothing was missing;
    /// - and the route a real short file *does* take, a short read inside [`Self::read_entry`],
    ///   surfaced as `UnexpectedEof` — a message naming no entry, no sequence and no path, so a
    ///   caller could not tell "this file shrank" from "the disk failed".
    ///
    /// The refusal now names the entry, the sequence and the path ([`Self::read_entry`]), and it
    /// is an [`StoreError`] the repair path already knows how to treat. It leaves the file
    /// untouched (`AGENTS.md` §6: never lose a user's work).
    pub fn read_all(&self) -> Result<Vec<SessionEntry>, StoreError> {
        let mut out = Vec::with_capacity(self.index.len());
        for s in 0..self.index.len() as u64 {
            let Some(entry) = self.read_entry(s)? else {
                // The arm the `expect` guarded. Unreachable today — the loop's bound is the
                // index's own length — but the correct answer when it is reached, and the reason
                // it is an `Err` and not an `unwrap`.
                return Err(StoreError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "index and file disagree: entry #{s} is in the index but not in {}",
                        self.path.display()
                    ),
                )));
            };
            out.push(entry);
        }
        Ok(out)
    }

    /// The whole session as a forest: every entry in the file, not just the active branch.
    ///
    /// This is pi's `getTree` (`core/session-manager.ts:1324-1362`) and it exists here
    /// because [`SessionStore::branch_entries`] cannot answer the question `/tree` asks.
    /// `branch_entries` walks from the leaf, so an abandoned branch is **invisible** to
    /// it — which means the TUI could not see a branch to summarise, label, or return to.
    /// That is the whole reason `branch_with_summary` had no production caller: not a
    /// missing primitive, a missing *view*. (`reset_leaf` was deleted at M221 — `/new`
    /// starts a new file, so no append ever needs to start a second root in one.)
    ///
    /// Three ways an entry is a root, matching pi exactly:
    ///
    /// 1. no parent (`parent_id == None`) — the start of the session;
    /// 2. it is its own parent — a hand-edited file;
    /// 3. its parent is not in the file — an orphan, which pi promotes rather than drops,
    ///    because a child whose parent is missing is still something the user wrote.
    ///
    /// A parent that is a **descendant** of the entry would close a cycle
    /// (`a → b → a`). pi cannot produce that, but a hand-edited file can, and an
    /// infinitely recursive tree is a stack overflow in a renderer. Such an entry becomes
    /// a root, so it is still drawn — exactly once — rather than lost.
    ///
    /// Children are sorted by `timestamp_ms` ascending, as in pi, so the order a reader
    /// sees is chronological rather than file-order by accident of branching.
    pub fn tree(&self) -> Result<Vec<SessionTreeNode>, StoreError> {
        let all = self.read_all()?;
        Ok(build_forest_from_entries(&all))
    }

    /// Every billed summarization call in the file, grouped by the model that was charged.
    ///
    /// M144/F22. pi's `getSessionStats` adds `compaction` and `branch_summary` usage
    /// explicitly, "so token/cost totals reflect what was actually billed"
    /// (`agent-session.ts:3340-3348`). Epsilon had nowhere to read that from, so a
    /// compaction — a real provider call that resends the whole history — was free in
    /// every number the user could see, worst in exactly the long sessions where a user
    /// compacts at all.
    ///
    /// Grouped by model because **price is a property of the model that consumed the
    /// tokens** (the reason M141 had to split the session's accounting the same way). A
    /// summarizer billed to a different model than the one in the header cannot be priced
    /// from the header's model, and guessing would be the M141 defect all over again.
    ///
    /// Reads the bill **on** each `Compaction`/`BranchSummary` entry (M145) — never a
    /// sidecar and never a second entry. That is pi's shape (`session-manager.ts:68-92`)
    /// and it is load-bearing: a separate accounting row becomes the file's *last* entry,
    /// which `Agent::pop_last_message` pops to discard a failed turn, so a rollback could
    /// separate a summary from its cost (F23). One entry cannot be half-removed.
    ///
    /// **A bill with no `model_id` is skipped, not guessed.** Sessions written before M145
    /// carry a compaction's `usage` and no charging model, and pi's v1 files carry neither.
    /// Counting those would mean inventing a rate, which is the M141 defect one layer up;
    /// skipping them means an old session reports a slightly lower total, which is the
    /// honest direction to be wrong in. Both arms are asserted in
    /// `a_legacy_compaction_with_usage_but_no_accounting_entry_is_not_counted`: the same
    /// entry without a model is skipped, and the same entry *with* one is counted — so a
    /// reader that skipped everything could not pass.
    ///
    /// `read_all` rather than `branch_entries`: billing belongs to the whole file. A
    /// compaction performed on a branch the user later navigated away from was still paid
    /// for, and a total that forgets it is wrong regardless of where the user is now.
    pub fn usage_accounts(&self) -> Result<BTreeMap<String, BTreeMap<String, Usage>>, StoreError> {
        let mut by_model: BTreeMap<String, BTreeMap<String, Usage>> = BTreeMap::new();
        for entry in self.read_all()? {
            // Both bill-carrying kinds, matched on their own fields rather than destructured,
            // so a new field on either is a compile error here instead of a silently
            // dropped bill.
            let (model_id, reason, usage) = match entry.kind {
                EntryKind::Compaction {
                    usage, model_id, ..
                } => (model_id, "compaction", usage),
                EntryKind::BranchSummary {
                    usage, model_id, ..
                } => (model_id, "branch_summary", usage),
                _ => continue,
            };
            // A bill with no model cannot be priced, and a bill with no usage is nothing to
            // count. Sessions written before M145 have the first case: the summarizer's usage
            // was recorded but nothing recorded which model ran it. Skipping under-reports
            // an old session, which is the honest direction to be wrong in; the alternative
            // is M141's defect, inventing a rate.
            let (Some(model_id), Some(usage)) = (model_id, usage) else {
                continue;
            };
            let by_reason = by_model.entry(model_id).or_default();
            match by_reason.get_mut(reason) {
                Some(seen) => *seen = seen.combine(usage),
                None => {
                    by_reason.insert(reason.to_string(), usage);
                }
            }
        }
        Ok(by_model)
    }

    /// One digest per compaction on this branch, oldest first
    /// ([`CompactionDigest`]: why branch order, not whole-file order).
    ///
    /// A digest with no `usage` still replays its row: the live path draws the same cost
    /// line with "no usage reported" (pi v1 files carry none), so the reader degrades to
    /// the live shape rather than refusing to draw — the row a resumed session shows for
    /// an old compaction is the row a live session showed for one.
    pub fn compaction_digests(&self) -> Result<Vec<CompactionDigest>, StoreError> {
        let mut out = Vec::new();
        for entry in self.branch_entries()? {
            // Fields by name, so a renamed field breaks here instead of silently
            // dropping the digest; `..` tolerates an added field the digest does not read.
            match entry.kind {
                EntryKind::Compaction {
                    summary,
                    tokens_before,
                    usage,
                    model_id,
                    ..
                } => out.push(CompactionDigest {
                    summary,
                    tokens_before,
                    usage,
                    model_id,
                }),
                _ => continue,
            }
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
    /// Navigating to the position *before the first entry* — pi's `resetLeaf` — is not
    /// an id and has no entry to name. Since M221 it is also not something this harness
    /// needs: `/new` starts a new file, so no append chains onto a leaf the user
    /// meant to abandon.
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
        model_id: Option<String>,
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
            // M145: the model that produced the summary, recorded beside the usage because
            // Epsilon prices from the catalog when it tallies. A caller with no bill passes
            // no model, and the reader skips the row either way.
            model_id,
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
            // A fork is a NEW session, so it inherits none of the source's unmodelled
            // header fields: carrying another session's `provider`/`branchedFrom` into a
            // file with a fresh id would describe a session that is not this one.
            preserved: BTreeMap::new(),
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

    /// Build the path a NEW session file gets, in this store's own naming convention:
    /// `<ISO-8601 timestamp with separators flattened>_<fresh ses_ id>.jsonl`.
    ///
    /// **Why a method rather than a caller-side `format!`.** `fork_branch_at` already
    /// follows this convention, and a second caller that invented its own name produced
    /// two spellings in the same directory — found by driving the installed binary and
    /// listing the files, where a fork read `2026-09-28T10-28-30-472Z_ses_….jsonl` and
    /// the RPC `new_session` read `ses_….jsonl`. Nothing broke; a session list would just
    /// sort them into two groups and a human would have no way to know why.
    ///
    /// The convention is not arbitrary: pi names sessions the same way, and the timestamp
    /// prefix is what makes a directory listing read in chronological order. The `ses_` id
    /// is what makes it unique — the timestamp alone collides within a millisecond.
    ///
    /// Returns a path only. Creating the file is the caller's business, so this cannot
    /// half-create a session as a side effect of computing where it goes.
    pub fn new_session_path(&self) -> PathBuf {
        self.path()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!(
                "{}_{}.jsonl",
                iso_now().replace([':', '.'], "-"),
                epsilon_contracts::new_id("ses")
            ))
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

    /// Which model and thinking level a resumed session should come back on.
    ///
    /// Port of pi's `getSessionContextSettings`
    /// (`session-manager.ts:362-374`). Three properties of the original are load-bearing
    /// and each is easy to get wrong in a port:
    ///
    /// 1. **It walks the SELECTED PATH**, not the file. `branch_entries()` is what makes
    ///    "switch model, branch back, get the earlier model" work: the answer belongs to
    ///    the branch, not to the session.
    /// 2. **There is no `break`.** The last change on the path wins. A port that returns
    ///    the first `model_change` gets the wrong answer for any session that switched
    ///    twice, which is the normal case after a correction.
    /// 3. **An assistant message sets the model too.** This is the one a
    ///    `model_change`-only implementation misses entirely, and it is what lets a
    ///    session that never had a `model_change` entry — written by an older build, or
    ///    simply never switched — still come back on the model it actually used.
    ///
    /// `thinking_level` defaults to pi's `"off"` when the path never says otherwise, so
    /// the caller distinguishes "off" from "never recorded" by `Some`/`None` on the
    /// model and by the value itself here.
    pub fn context_settings(&self) -> Result<SessionContextSettings, StoreError> {
        let branch = self.branch_entries()?;
        let mut model: Option<(String, String)> = None;
        // pi initialises to "off" (`session-manager.ts:364`) and only overwrites on an
        // entry, so a path with no thinking entry is explicitly off, not unknown.
        let mut thinking_level = "off".to_string();

        for e in &branch {
            match &e.kind {
                EntryKind::ModelChange {
                    provider, model_id, ..
                } => {
                    model = Some((provider.clone(), model_id.clone()));
                }
                EntryKind::ThinkingLevelChange { level, .. } => {
                    thinking_level = level.clone();
                }
                // pi's third arm: `entry.type === "message" && role === "assistant"`.
                EntryKind::Message {
                    message: Message::Assistant(a),
                    ..
                } => model = Some((a.provider.clone(), a.model_id.clone())),
                _ => {}
            }
        }

        Ok(SessionContextSettings {
            model,
            thinking_level: Some(thinking_level),
        })
    }

    /// Append a typed turn-end marker (Epsilon addition; sticky reason lives in the log).
    pub fn record_turn_end(&mut self, reason: TurnEndReason) -> Result<(), StoreError> {
        self.append(EntryKind::TurnEnd { reason })?;
        Ok(())
    }

    /// Append a turn anchor: the immutable map from turn N to its workspace
    /// footprint (turn-scoped undo substrate).
    ///
    /// The anchor is an [`EntryKind::Custom`] entry with `custom_type`
    /// [`TURN_ANCHOR_CUSTOM_TYPE`], so it needs **no schema change**: per-file
    /// checkpoint entries keep their shape, and the session file stays readable
    /// by any build that knows `Custom`. The payload is a complete snapshot
    /// (turn, ledger seq range, leaf-before), which is what makes the undo fold
    /// a lookup rather than a replay.
    ///
    /// `ledger_start..=ledger_end` names the checkpoint-ledger seqs this turn
    /// wrote (`ledger_start > ledger_end` means the turn touched no files).
    /// `leaf_before` is the active-branch tip before the turn's first append;
    /// `None` only when the turn began on an empty session. The anchor itself
    /// lands on the active lane and becomes the new leaf, like every append.
    ///
    /// Anchors are invisible to [`SessionStore::model_view`]: the projection
    /// ignores `Custom` entries, so an anchor never reaches the provider.
    pub fn append_turn_anchor(
        &mut self,
        turn: u64,
        ledger_start: u64,
        ledger_end: u64,
        leaf_before: Option<String>,
    ) -> Result<SessionEntry, StoreError> {
        let data = serde_json::json!({
            "turn": turn,
            "ledger_start": ledger_start,
            "ledger_end": ledger_end,
            "leaf_before": leaf_before,
        });
        self.append(EntryKind::Custom {
            custom_type: TURN_ANCHOR_CUSTOM_TYPE.to_string(),
            data: Some(data),
        })
    }

    /// Every turn anchor in the file, oldest first.
    ///
    /// Reads the whole file rather than the active branch on purpose: an undo
    /// moves the leaf back past later anchors, and a listing that walked the
    /// branch would forget the turns it just hid. A goal folds the same way
    /// for the same reason.
    pub fn turn_anchor_entries(&self) -> Result<Vec<SessionEntry>, StoreError> {
        Ok(self
            .read_all()?
            .into_iter()
            .filter(|e| matches!(&e.kind, EntryKind::Custom { custom_type, .. } if custom_type == TURN_ANCHOR_CUSTOM_TYPE))
            .collect())
    }

    /// Parse a turn-anchor entry into `(turn, ledger_start, ledger_end,
    /// leaf_before)`. `None` when the entry is not an anchor or its payload
    /// does not carry the four fields in the expected shapes.
    pub fn parse_turn_anchor(entry: &SessionEntry) -> Option<(u64, u64, u64, Option<String>)> {
        let EntryKind::Custom { custom_type, data } = &entry.kind else {
            return None;
        };
        if custom_type != TURN_ANCHOR_CUSTOM_TYPE {
            return None;
        }
        let data = data.as_ref()?;
        let turn = data.get("turn")?.as_u64()?;
        let ledger_start = data.get("ledger_start")?.as_u64()?;
        let ledger_end = data.get("ledger_end")?.as_u64()?;
        let leaf_before = match data.get("leaf_before") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(s)) => Some(s.clone()),
            Some(_) => return None,
        };
        Some((turn, ledger_start, ledger_end, leaf_before))
    }

    /// The anchor for turn `turn`, when one was recorded.
    pub fn turn_anchor_for(&self, turn: u64) -> Result<Option<SessionEntry>, StoreError> {
        Ok(self
            .turn_anchor_entries()?
            .into_iter()
            .find(|e| Self::parse_turn_anchor(e).is_some_and(|(t, ..)| t == turn)))
    }

    /// The next turn number: one past the highest anchored turn, `1` when no
    /// turn was ever anchored. Gaps never occur in practice (every run
    /// anchors), so the count and the max agree; the max is what matters.
    pub fn next_turn_number(&self) -> Result<u64, StoreError> {
        let mut max = 0u64;
        for e in self.turn_anchor_entries()? {
            if let Some((t, ..)) = Self::parse_turn_anchor(&e) {
                max = max.max(t);
            }
        }
        Ok(max + 1)
    }
}

/// The `custom_type` of a turn-anchor entry (turn-scoped undo).
///
/// One new append-only kind, and the only one this change adds: per-file
/// checkpoint entries are reused untouched, so there is deliberately no
/// second constant here.
pub const TURN_ANCHOR_CUSTOM_TYPE: &str = "turn_anchor";

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

/// Suffix of the copy [`persist_migration`](SessionStore::persist_migration) keeps of a
/// converted file, appended to the whole name so the original's own extension survives.
const PRE_MIGRATION_SUFFIX: &str = ".pre-epsilon";

/// The path [`SessionStore::persist_migration`] keeps this file's previous bytes at.
///
/// Public because the *caller* has to be able to name it: a conversion rewrites the user's
/// file in place, and a message that says "migrated" without saying where the old bytes went
/// leaves them to find out by looking. The rule lives here rather than in each frontend so
/// the name a message prints is the name the conversion wrote — a duplicated suffix is how
/// the two drift apart.
pub fn original_backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(PRE_MIGRATION_SUFFIX);
    PathBuf::from(name)
}

/// Copy `path` to [`original_backup_path`], once.
///
/// No-op when the copy is already there: it is the true original from an earlier
/// conversion, and a second migration pass over an already-converted file must not
/// replace it with a converted copy (which would leave no way back at all). The copy is
/// never treated as scratch — a name already on disk is left exactly as it is.
///
/// **Through a scratch name, because `fs::copy` is not atomic (M292).** The `exists()`
/// above is what makes the copy once-only, and that is also what made a *partial* copy
/// permanent: a copy interrupted by a full disk, or by the antivirus this machine
/// documents in `AGENTS.md` §4, left a truncated file at the real name — and every later
/// call short-circuited on it, so the artefact meant to be the way back was a fragment
/// for ever, with nothing reporting it. This is the same tmp-then-rename discipline
/// [`persist_migration`] already uses for the converted file, applied to the copy of the
/// original; the scratch name is removed on every failure path, so a failed copy leaves
/// no file at all rather than a wrong one.
///
/// **What is not tested here, and should be named rather than implied.** Forcing a copy
/// to fail mid-way needs a fault seam this function does not pass through (the store's
/// `WriteFault` covers append and migrate operations, not `fs::copy`), so the failure
/// branch is exercised only by inspection. What a test *does* pin is the other half: that
/// a successful conversion leaves no scratch file behind
/// (`a_conversion_leaves_no_scratch_file_beside_the_backup`).
fn keep_original_beside(path: &Path) -> std::io::Result<()> {
    let backup = original_backup_path(path);
    if backup.exists() {
        return Ok(());
    }
    let mut scratch_name = backup.as_os_str().to_os_string();
    scratch_name.push(".partial");
    let scratch = PathBuf::from(scratch_name);
    if let Err(e) = std::fs::copy(path, &scratch) {
        let _ = std::fs::remove_file(&scratch);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&scratch, &backup) {
        let _ = std::fs::remove_file(&scratch);
        return Err(e);
    }
    Ok(())
}

/// ISO-8601 UTC timestamp for new headers (pi's `new Date().toISOString()`).
fn iso_now() -> String {
    iso_from_ms(now_ms())
}

/// Epoch milliseconds as `YYYY-MM-DDTHH:MM:SS.mmmZ` — the spelling pi writes and this build's
/// [`parse_iso8601_ms`] reads.
///
/// No date crate: [`days_from_civil`] goes the other way, so the civil-date conversion here is
/// its inverse (`civil_from_days`, Howard Hinnant's algorithm), and a round trip through the
/// two is what the tests pin. The pi importer needs this direction because a header this build
/// writes carries an ISO string — importing an old session under `now_ms()` would re-date the
/// header to the moment of the import, which is the defect M114 fixed for entries, one layer
/// up.
pub(crate) fn iso_from_ms(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let millis = ms % 1000;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Civil date for a day count since 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11], Mar=0
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
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
/// rather than be fast — and to *re-decode* the line through the typed entry parser, which is
/// where the useful sentence comes from. The first version passed the caller's error in and got
/// ``data did not match any variant of untagged enum SessionLine``: `SessionLine` is untagged, so
/// serde reports the outermost failure and throws away the field-level one. Decoding
/// [`SessionEntry`] directly from the same value recovers it.
fn diagnose_line(fragment: &str) -> LineDiagnosis {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(fragment.trim_end()) else {
        return LineDiagnosis::NotJson;
    };
    // An entry's kind is `{"type": "…"}`; anything else is either a header or a legacy shape.
    // The tag is copied out rather than borrowed, because the decode below consumes the value.
    let kind_type = value
        .get("kind")
        .and_then(|k| k.get("type"))
        .and_then(|t| t.as_str())
        .map(str::to_string);
    if let Some(kind_type) = kind_type {
        // The list of kinds this build knows decides which of the two sentences the reader
        // gets. Without it, an entry of a kind we understand but could not read was reported
        // as "probably written by a newer Epsilon" — the wrong conclusion, and the one
        // M108's Limits recorded as owed.
        return if epsilon_contracts::session_entry::KNOWN_ENTRY_KINDS.contains(&kind_type.as_str())
        {
            // The field-level cause, from the one decode that reports it. `None` when the
            // typed decode unexpectedly succeeds, which would mean the line was refused for
            // some other reason and the sentence should not pretend to know which.
            let detail = serde_json::from_value::<SessionEntry>(value)
                .err()
                .map(|e| e.to_string());
            LineDiagnosis::MalformedEntry {
                entry_type: kind_type,
                detail,
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
///
/// The v2 importer's counterpart is [`crate::import::is_pi_v2_entry_shape`]: same
/// `type` + `timestamp` pair, but *with* an `id`, which is what pi's own
/// `migrateV1ToV2` adds (`references/pi-0.85.1/…/session-manager.ts:231-242`). Keeping
/// the two predicates apart is what stops a pi file from being healed in place: v2 heals
/// into a *new* file, v1 heals in place behind a backup.
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
    let timestamp_ms = healed_entry_timestamp(map);
    SessionEntry {
        id: epsilon_contracts::new_id("ent"),
        seq: 0,          // reassigned gaplessly by the caller after migration
        parent_id: None, // chain-linked by the caller after migration
        timestamp_ms,
        kind: heal_legacy_kind(&leg.value, timestamp_ms, legacy_compaction_target(map, ids)),
    }
}

/// The instant one v1/v2 entry line carries, in epoch milliseconds.
///
/// pi stamps a message line twice: an ISO string (or epoch number) on the line and epoch
/// milliseconds inside the message, milliseconds apart. The millisecond one is the finer
/// spelling of that instant, so it wins (`heal_legacy_message` reads the same field), and
/// the entry and its message can never disagree about when the turn happened.
///
/// Shared by both healers — v1's migration and the v2 importer — so that "which stamp is
/// the real one" has one answer rather than two that drift.
pub(crate) fn healed_entry_timestamp(
    map: Option<&serde_json::Map<String, serde_json::Value>>,
) -> u64 {
    let line_ts = map
        .and_then(|m| m.get("timestamp"))
        .map_or_else(now_ms, legacy_timestamp_ms);
    map.and_then(|m| m.get("message"))
        .and_then(|m| m.get("timestamp"))
        .and_then(|t| t.as_u64())
        .unwrap_or(line_ts)
}

/// Pi's v1 compaction target: a *position* (`firstKeptEntryIndex`) resolved against the ids
/// assigned so far. `None` when pi wrote no index, or wrote one that lands nowhere.
fn legacy_compaction_target(
    map: Option<&serde_json::Map<String, serde_json::Value>>,
    ids: &[String],
) -> Option<String> {
    map.and_then(|m| m.get("firstKeptEntryIndex"))
        .and_then(|i| i.as_u64())
        .and_then(|i| ids.get(i as usize).cloned())
}

/// One pi entry line's `type` tag, as an Epsilon [`EntryKind`].
///
/// The single conversion both healers call: v1's in-place migration and the v2 importer. It
/// exists as one function on purpose — a second converter is how the two drift, and the drift
/// is not cosmetic: the last one emptied every message while reporting success, because it read
/// an array of parts with `as_str()` (G19,
/// `audit/evidence/pi-import-empties-messages-2026-09-22.md`).
///
/// `raw` is the whole entry object; `compaction_target` is the already-resolved
/// `firstKeptEntryId` (v2 hands one over; v1 resolves a position — see
/// [`legacy_compaction_target`]). Anything this build cannot express heals to
/// [`EntryKind::Custom`] holding `raw`, so the human log stays complete and nothing is
/// silently dropped.
pub(crate) fn heal_legacy_kind(
    raw: &serde_json::Value,
    timestamp_ms: u64,
    compaction_target: Option<String>,
) -> EntryKind {
    let map = raw.as_object();
    let kind = map.and_then(|m| m.get("type")).and_then(|t| t.as_str());
    match kind {
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
            // pi v1→v2: `firstKeptEntryIndex` (positional) becomes `firstKeptEntryId`. The
            // healed compaction keeps its summary/tokens and the resolved target rides in
            // `details` for future tooling, whichever spelling the source used.
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
                details: compaction_target.map(|id| serde_json::json!({"firstKeptEntryId": id})),
                // pi's v1 format records no usage on a compaction, so an imported session
                // has a summary and no bill at all. `None` is the honest value, and the
                // reader skips it rather than pricing a summary nobody was charged for
                // (M145).
                usage: None,
                // And no charging model, for the same reason. pi's file does not name one,
                // and inventing it would be pricing a bill that does not exist.
                model_id: None,
            }
        }
        // The two entry kinds whose pi shape lines up with an Epsilon variant field for field,
        // which is what makes healing them typed *faithful* rather than interpretive: nothing is
        // renamed but the field spelling (`modelId` → `model_id`, `thinkingLevel` → `level`), no
        // value is reinterpreted, and neither variant has a field pi's line leaves out. Typed,
        // the entry can be applied by whoever reads the session; `Custom` cannot be applied by
        // anyone. Verified against pi's own fixture lines
        // (`references/pi-0.85.1/packages/coding-agent/test/fixtures/before-compaction.jsonl`):
        // `{"type":"model_change","timestamp":…,"provider":"anthropic","modelId":"claude-3-opus-20240229"}`
        // and `{"type":"thinking_level_change","timestamp":…,"thinkingLevel":"high"}` — note the
        // camelCase, which is the fixture's spelling and therefore this reader's field name.
        //
        // A line missing a field its variant needs heals to `Custom` below instead of being
        // half-typed: `ModelChange { provider: "" }` would be a lie that looks like data, and no
        // reader could tell it from a session that genuinely has no provider.
        Some("model_change") => match (legacy_str(map, "provider"), legacy_str(map, "modelId")) {
            (Some(provider), Some(model_id)) => EntryKind::ModelChange {
                provider: provider.to_string(),
                model_id: model_id.to_string(),
            },
            _ => legacy_custom(raw),
        },
        Some("thinking_level_change") => match legacy_str(map, "thinkingLevel") {
            Some(level) => EntryKind::ThinkingLevelChange {
                // Kept as pi's word: `ThinkingLevel` is parsed where the level is applied, so a
                // level this build does not know stays visible rather than being silently
                // clamped to `off`.
                level: level.to_string(),
            },
            None => legacy_custom(raw),
        },
        // `label` and `session_info` have Epsilon variants too, but no fixture here shows
        // pi's v1 spelling of them and both carry id references that a *positional* v1 line
        // may not be able to resolve; guessing would put a label on the wrong entry, so
        // they stay `Custom` (lossless, merely unapplied) until a real file settles it.
        _ => legacy_custom(raw),
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
///
/// Crate-visible because the pi importer counts the preserved parts it wrote — the count a
/// report prints has to come from the same key the healer used, or the two disagree about
/// whether anything was lost.
pub(crate) const LEGACY_EXTRAS_KEY: &str = "legacy_unmapped_parts";

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
            // The provider signature rides along when pi wrote one. This is the *one* helper both
            // readers convert a content part through — `heal_legacy_message` serves the v1
            // migration and `heal_pi_entry_kind` serves the v2 import (`import.rs`) — so carrying
            // it here is what makes the importer carry it, with no second conversion to drift.
            //
            // Absent and empty stay apart, as everywhere else in this file: pi writing
            // `"thinkingSignature": ""` is a part that has a signature and it is empty, and a
            // reader that turned that into `None` would be editing the source rather than
            // reading it.
            signature: part
                .get("thinkingSignature")
                .and_then(|s| s.as_str())
                .map(str::to_string),
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
        ContentBlock::Thinking { text, .. } => UserContent::Text {
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

/// The same for a legacy line whose object may not exist at all, borrowed rather than cloned.
///
/// Absent, `null` and a non-string are one answer, `None`, because they are one fact to the
/// mappings that ask: the field is not there to be read. `Some("")` would be a different fact —
/// present and empty — and a typed variant built from it claims something pi never wrote.
fn legacy_str<'a>(
    map: Option<&'a serde_json::Map<String, serde_json::Value>>,
    key: &str,
) -> Option<&'a str> {
    map.and_then(|m| m.get(key)).and_then(|v| v.as_str())
}

/// The `Custom` fallback for a foreign line this build cannot state exactly: every kind with no
/// Epsilon variant, and every line missing a field a typed variant requires.
///
/// Lossless by construction — the whole line rides in `data` — so the human log stays complete
/// and the model view skips it; unapplied is the honest outcome where typed would be a guess.
/// Takes the raw value rather than a [`LegacyLine`] because the v2 importer has no legacy line
/// to hand: the fallback is shared, so both readers preserve in exactly the same shape.
fn legacy_custom(raw: &serde_json::Value) -> EntryKind {
    EntryKind::Custom {
        custom_type: "legacy".into(),
        data: Some(raw.clone()),
    }
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
        // K1: the keys being *present* is the signal, not their value. pi writes
        // this object for every call, so a replayed `cacheRead: 0` is a measured
        // zero and must not come back as "not measurable".
        cache_reported: value.get("cacheRead").is_some() || value.get("cacheWrite").is_some(),
        // Same presence rule for pi's `reasoning` split (subset of output):
        // replayed with the key it stays measured, without it stays silent.
        reasoning: value.get("reasoning").and_then(|v| v.as_u64()),
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
///
/// Crate-visible because the pi importer normalises a header's stamp with it: one reader for
/// "what instant does this field name" is the difference between two readers that agree and
/// two that drift.
pub(crate) fn legacy_timestamp_ms(value: &serde_json::Value) -> u64 {
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
/// `YYYY-MM-DDTHH:MM:SS[.frac][Z|±HH:MM]` is what pi writes, and [`iso_from_ms`] reads back
/// what [`iso_now`] writes. Anything else — a bare `"1765233665292"`, a hand-edited
/// `"2026-09-18T00-00-00-000Z"` — names no date, so `None` lets the caller fall back instead of
/// inventing 1970.
///
/// Crate-visible for the pi importer, which has to tell "a timestamp I can keep verbatim" from
/// one it must re-stamp, and must use the same parser that decision is about.
pub(crate) fn parse_iso8601_ms(s: &str) -> Option<u64> {
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

/// Unix seconds of a session-header timestamp, for comparison with a checkpoint's
/// `at` (also unix seconds).
///
/// `None` when `iso` is not a timestamp this crate writes, so a caller falls back
/// to "every checkpoint" instead of inventing 1970. The division is exact for the
/// comparison: checkpoints store seconds, headers store milliseconds.
#[must_use]
pub fn timestamp_unix_secs(iso: &str) -> Option<u64> {
    parse_iso8601_ms(iso).map(|ms| ms / 1000)
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

/// Order a flat entry list into a forest.
///
/// Shared by [`SessionStore::tree`] and every consumer that has to render the
/// session as a tree, so the TUI and the RPC surface cannot disagree about what
/// the tree is — D27/D29's lesson, which is that two implementations of one
/// reference type are one defect waiting to happen.
///
/// Children sort by `timestamp_ms` ascending, which is pi's rule
/// (`session-manager.ts:1355-1360`): the order a reader sees is chronological,
/// not an artefact of which branch happened to be written first.
/// Build the parentage forest from a flat entry list.
///
/// **Public because there are two consumers and one of them lives in another
/// crate**: [`SessionStore::tree`] reads the file and calls this, and the RPC
/// layer calls it directly to project the same structure onto pi's wire shape
/// without going back to disk. M196 deleted a second, hand-written tree builder
/// in `rpc_mode.rs` that had drifted toward a different answer to "what does
/// this session look like".
///
/// The three root cases are pi's (`core/session-manager.ts`, `getTree()`): no
/// parent, a self-parent, or a parent absent from the file. Children are sorted
/// by timestamp at every level. Both are load-bearing: a root case is what keeps
/// a truncated or hand-edited file rendering everything instead of silently
/// dropping its tail, and the sort is what makes a branch read in the order it
/// happened.
pub fn build_forest_from_entries(entries: &[SessionEntry]) -> Vec<SessionTreeNode> {
    let index: std::collections::HashMap<&str, usize> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.id.as_str(), i))
        .collect();

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); entries.len()];
    let mut is_root = vec![false; entries.len()];
    for (i, e) in entries.iter().enumerate() {
        // pi's three root cases, in one place: no parent, a self-parent, or a
        // parent that is not in this file. An orphan is promoted rather than
        // dropped, so a truncated or hand-edited session still renders.
        let parent = e.parent_id.as_deref().filter(|p| *p != e.id.as_str());
        is_root[i] = match parent {
            None => true,
            Some(p) => match index.get(p) {
                // A parent that is a descendant of this entry would close a cycle
                // (`a -> b -> a`). pi cannot write that; a hand-edited file can, and
                // the alternatives to promoting the entry are losing it or recursing
                // forever. Root keeps it, exactly once.
                Some(&pi) => reaches(entries, &index, pi, i),
                None => true,
            },
        };
        if !is_root[i] {
            children[index[parent.expect("a non-root has a resolved parent")]].push(i);
        }
    }
    for kids in &mut children {
        kids.sort_by_key(|&c| entries[c].timestamp_ms);
    }

    fn emit(entries: &[SessionEntry], i: usize, children: &[Vec<usize>]) -> SessionTreeNode {
        SessionTreeNode {
            entry: entries[i].clone(),
            children: children[i]
                .iter()
                .map(|&c| emit(entries, c, children))
                .collect(),
            // Labels are not written yet, so there is nothing to resolve. The
            // fields exist because pi's node has them and a client reading
            // `node.label` must get null rather than undefined.
            label: None,
            label_timestamp: None,
        }
    }

    let mut roots: Vec<usize> = (0..entries.len()).filter(|&i| is_root[i]).collect();
    roots.sort_by_key(|&i| entries[i].timestamp_ms);
    roots
        .into_iter()
        .map(|i| emit(entries, i, &children))
        .collect()
}

/// Does `from` appear anywhere in `target`'s parent chain? True means making
/// `target` a child of `from` would close a cycle.
///
/// Linking an entry under a parent that is one of its own ancestors is unsafe: a
/// hand-edited file can contain `a -> b -> a`, and without this check both
/// entries are unreachable from any root, the emitter builds them as nodes that
/// are never attached, and **the session silently loses them**. The caller
/// promotes such an entry to a root instead, so it is still rendered, exactly
/// once.
///
/// The walk is bounded by the entry count. A longer walk means a cycle, which by
/// definition means the goal *is* reachable, so the bound cannot produce a wrong
/// answer -- it only stops a walk that would otherwise not terminate.
///
/// M196: this doc was written on the `rpc_mode.rs` copy, which was deleted when
/// that file's second tree builder was replaced by a call to this one. The
/// reasoning moved here rather than being deleted with the code.
fn reaches(
    entries: &[SessionEntry],
    index: &std::collections::HashMap<&str, usize>,
    from: usize,
    target: usize,
) -> bool {
    let mut cursor = Some(from);
    let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
    while let Some(i) = cursor {
        if i == target {
            return true;
        }
        if !seen.insert(i) {
            // A pre-existing cycle elsewhere in the file. Stop rather than loop;
            // the entry is already not a root for some other reason.
            return false;
        }
        cursor = entries[i]
            .parent_id
            .as_deref()
            .and_then(|p| index.get(p).copied());
    }
    false
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

#[cfg(test)]
mod turn_anchor_tests {
    use super::*;

    fn tmp_store() -> (tempfile::TempDir, SessionStore) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let store = SessionStore::open_in(&path, dir.path()).expect("a fresh session opens");
        (dir, store)
    }

    /// RED-first: fails while no turn-anchor kind exists (`turn_anchor_entries`
    /// unresolved), which is the shape under test.
    #[test]
    fn an_anchor_round_trips_and_lists_in_file_order() {
        let (_dir, mut st) = tmp_store();
        assert_eq!(st.next_turn_number().unwrap(), 1, "no anchors yet");
        let leaf_before = st.leaf_id();
        let first = st
            .append_turn_anchor(1, 7, 9, leaf_before.clone())
            .expect("anchor appends");
        assert_eq!(
            SessionStore::parse_turn_anchor(&first),
            Some((1, 7, 9, leaf_before)),
            "the appended entry parses back to what was written"
        );
        st.append(EntryKind::Custom {
            custom_type: "session_name".into(),
            data: None,
        })
        .unwrap();
        st.append_turn_anchor(2, 10, 10, Some(first.id.clone()))
            .unwrap();
        let anchors = st.turn_anchor_entries().unwrap();
        assert_eq!(anchors.len(), 2, "only anchors list, in file order");
        assert_eq!(st.next_turn_number().unwrap(), 3);
        assert_eq!(
            st.turn_anchor_for(2).unwrap().expect("turn 2 anchored").id,
            anchors[1].id
        );
        assert!(st.turn_anchor_for(9).unwrap().is_none());
    }

    /// An anchor never reaches the provider: the model view skips `Custom`.
    #[test]
    fn an_anchor_is_excluded_from_the_model_view() {
        let (_dir, mut st) = tmp_store();
        st.append_turn_anchor(1, 1, 0, None).unwrap();
        let view = st.model_view().unwrap();
        assert!(
            view.is_empty(),
            "an anchor-only branch shows nothing: {view:?}"
        );
    }

    /// A non-anchor `Custom` entry is not an anchor, and a mangled payload is
    /// not parsed rather than misread.
    #[test]
    fn only_well_formed_anchors_parse() {
        let (_dir, mut st) = tmp_store();
        let other = st
            .append(EntryKind::Custom {
                custom_type: "session_name".into(),
                data: None,
            })
            .unwrap();
        assert_eq!(SessionStore::parse_turn_anchor(&other), None);
        let mut mangled = st.append_turn_anchor(1, 1, 1, None).unwrap();
        mangled.kind = EntryKind::Custom {
            custom_type: TURN_ANCHOR_CUSTOM_TYPE.into(),
            data: Some(serde_json::json!({"turn": "one"})),
        };
        assert_eq!(SessionStore::parse_turn_anchor(&mangled), None);
    }
}
