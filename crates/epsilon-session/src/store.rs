//! Session file store: append, branch navigation, and derived model view.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use epsilon_contracts::agent_event::TurnEndReason;
use epsilon_contracts::message::{Message, UserMessage};
use epsilon_contracts::session_entry::{EntryKind, SessionEntry};

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

/// Errors surfaced by the store. Failures are values, not panics — a corrupt session must be
/// repairable or at least inspectable, never fatal to the process.
#[derive(Debug)]
pub enum StoreError {
    /// Underlying filesystem error.
    Io(std::io::Error),
    /// A JSONL line failed to parse; carries the 1-based line number.
    CorruptLine(usize),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Io(e) => write!(f, "io error: {e}"),
            StoreError::CorruptLine(n) => {
                write!(f, "corrupt session line #{n} (crash-truncated tail?)")
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
pub struct SessionStore {
    path: PathBuf,
    file: File,
    /// seq -> location of the entry's line.
    index: Vec<EntryLocation>,
    /// Parsed leaf entry (id + seq); `None` for an empty session.
    leaf: Option<(String, u64)>,
    next_seq: u64,
}

impl SessionStore {
    /// Open (creating if absent) a session file.
    ///
    /// A crash-truncated final line is detected and physically removed automatically —
    /// recovery is the expected path after a kill, not an exceptional error.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
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
        };
        if !store.reindex()? {
            store.truncate_to_index()?;
        }
        Ok(store)
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
    /// Returns `true` when the file was fully clean, `false` when a crash-truncated tail
    /// line was found (indexing stopped there; call [`Self::truncate_to_index`] to remove it).
    pub fn reindex(&mut self) -> Result<bool, StoreError> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut reader = BufReader::new(&self.file);
        self.index.clear();
        self.leaf = None;
        self.next_seq = 0;
        let mut offset = 0u64;

        let mut last_good_offset = 0u64;
        let mut clean = true;

        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line)?;
            if n == 0 {
                break;
            }
            if line.trim().is_empty() {
                offset += n as u64;
                continue;
            }

            match serde_json::from_str::<SessionEntry>(line.trim_end()) {
                Ok(entry) => {
                    self.index.push(EntryLocation {
                        offset,
                        len: line.len(),
                        id_hash: hash_id(&entry.id),
                    });
                    self.leaf = Some((entry.id.clone(), entry.seq));
                    self.next_seq = entry.seq + 1;
                    last_good_offset = offset + n as u64;
                }
                Err(_) => {
                    // Crash mid-write: stop indexing here; everything after is garbage.
                    clean = false;
                    break;
                }
            }
            offset += n as u64;
        }
        drop(reader);
        self.file.seek(SeekFrom::Start(last_good_offset))?;
        Ok(clean)
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

    /// Read one entry by sequence number (lazy: seeks and parses just that line).
    pub fn read_entry(&self, seq: u64) -> Result<Option<SessionEntry>, StoreError> {
        let Some(loc) = self.index.get(seq as usize) else {
            return Ok(None);
        };
        let mut buf = vec![0u8; loc.len];
        let mut f = &self.file;
        f.seek(SeekFrom::Start(loc.offset))?;
        f.read_exact(&mut buf)?;
        let line = String::from_utf8_lossy(&buf);
        Ok(Some(
            serde_json::from_str(line.trim_end())
                .map_err(|_| StoreError::CorruptLine(seq as usize + 1))?,
        ))
    }

    /// Append a new entry on the current lane; assigns seq/parent/timestamp/id and flushes.
    pub fn append(&mut self, kind: EntryKind) -> Result<SessionEntry, StoreError> {
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
