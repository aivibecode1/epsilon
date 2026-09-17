//! Stale-read guard (defect D-15; Hermes `tools/file_state.py` parity).
//!
//! `write` and `edit` are the only tools that can destroy work, and the model decides to call
//! them from what it read — possibly many turns ago, and possibly before something else
//! rewrote the file: a `task` child, a background script, the user's own editor. The mutation
//! queue in [`crate::mutation`] serializes *harness* writers against each other; it cannot know
//! that the bytes on disk changed since the model last looked.
//!
//! This module records what each path looked like when the model last read it, and refuses a
//! mutation whose target no longer matches — naming the path and the reason, so the model can
//! re-read and proceed. It is a *known-change* guard, not a lock: an external writer between
//! the check and the write still wins the race, which is why the check runs inside the
//! mutation queue and why the diagnosis tells the model what to do rather than only what
//! happened.
//!
//! Observations belong to an opaque agent/session-view lifetime, never to a shared
//! session-name string. Children share mutation locks, not read history (F02).
//! Weak owners allow retired maps to be reclaimed on the next observation access.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

/// What the model last saw at a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Snapshot {
    /// A successful sibling mutation invalidated this observation, even if metadata matches.
    invalidated: bool,
    /// Modification time as reported by the filesystem.
    mtime: Option<std::time::SystemTime>,
    /// File length in bytes at read time.
    len: u64,
    /// Whether the read covered the whole file. A partial read cannot justify an overwrite:
    /// the model would be replacing text it has never seen.
    partial: bool,
    /// Whether the read stopped at the harness's byte cap rather than at a boundary the caller
    /// chose. Such a file *cannot* be read any further through this tool, so refusing on
    /// `partial` alone would leave the model unable to ever rewrite it.
    capped: bool,
}

/// Why a mutation was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stale {
    /// The file changed since the model read it.
    Modified {
        /// Path as the caller named it (display form).
        path: String,
        /// Bytes when the model read it.
        read_len: u64,
        /// Bytes now.
        now_len: u64,
    },
    /// The model only ever read part of the file and is now overwriting all of it.
    PartialRead {
        /// Path as the caller named it (display form).
        path: String,
    },
    /// The file exists and nothing in this session has read it.
    NeverRead {
        /// Path as the caller named it (display form).
        path: String,
    },
}

impl Stale {
    /// The diagnosis the model reads: what is wrong, and what to do about it.
    pub fn diagnosis(&self, tool: &str) -> String {
        match self {
            Stale::Modified {
                path,
                read_len,
                now_len,
            } => format!(
                "refused: {path} changed on disk since it was last read ({read_len} bytes then, \
                 {now_len} bytes now) — read it again, then apply your {tool} to what is \
                 actually there now"
            ),
            Stale::PartialRead { path } => format!(
                "refused: {path} was only read in part; overwriting the whole file would discard \
                 the lines nobody has seen — read it through first (or edit the specific text \
                 instead)"
            ),
            Stale::NeverRead { path } => format!(
                "refused: {path} exists but has not been read in this session; read it before \
                 overwriting, so you are not replacing work you have never seen"
            ),
        }
    }
}

#[derive(Default)]
struct Observations {
    scopes: HashMap<usize, Scope>,
}

struct Scope {
    owner: Weak<()>,
    paths: HashMap<PathBuf, Snapshot>,
}

impl Observations {
    fn paths(&mut self, owner: &Arc<()>) -> &mut HashMap<PathBuf, Snapshot> {
        // Dead owners cannot donate observations to a reused allocation address.
        // Reclaim on every access, so retired agent maps do not accumulate.
        self.scopes
            .retain(|_, scope| scope.owner.strong_count() > 0);
        &mut self
            .scopes
            .entry(Arc::as_ptr(owner) as usize)
            .or_insert_with(|| Scope {
                owner: Arc::downgrade(owner),
                paths: HashMap::new(),
            })
            .paths
    }
}

fn log() -> &'static Mutex<Observations> {
    static LOG: OnceLock<Mutex<Observations>> = OnceLock::new();
    LOG.get_or_init(|| Mutex::new(Observations::default()))
}

/// Normalized absolute key. Lexical (no `realpath`): the same rule the mutation queue uses, so
/// a path and its equivalent spelling address one entry.
fn key_of(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn stat(path: &Path) -> (Option<std::time::SystemTime>, u64) {
    match std::fs::metadata(path) {
        Ok(md) => (md.modified().ok(), md.len()),
        Err(_) => (None, 0),
    }
}

/// Record that the model has now seen `path` as it is on disk.
///
/// `partial` must be true whenever the content handed to the model was windowed: the whole
/// point is to distinguish "I know this file" from "I have seen some of it". `capped` says the
/// window was the harness's byte cap rather than the caller's choice — no further read through
/// this tool can widen it, so a `partial` view from the cap does not block a later rewrite.
pub fn record_read(owner: &Arc<()>, path: &Path, partial: bool, capped: bool) {
    let (mtime, len) = stat(path);
    if let Ok(mut map) = log().lock() {
        map.paths(owner).insert(
            key_of(path),
            Snapshot {
                invalidated: false,
                mtime,
                len,
                partial,
                capped,
            },
        );
    }
}

/// Record the state a successful mutation just produced, so the model's own change does not
/// look like someone else's.
pub fn record_write(owner: &Arc<()>, path: &Path) {
    let key = key_of(path);
    let (mtime, len) = stat(path);
    if let Ok(mut map) = log().lock() {
        // Keep sibling observations stale even for same-length writes on coarse clocks.
        for scope in map.scopes.values_mut() {
            if let Some(snapshot) = scope.paths.get_mut(&key) {
                snapshot.invalidated = true;
            }
        }
        map.paths(owner).insert(
            key,
            Snapshot {
                invalidated: false,
                mtime,
                len,
                partial: false,
                capped: false,
            },
        );
    }
}

/// Check a mutation against what has been read of this path.
///
/// Rules, in the order they are checked:
/// 1. **never read, and the file exists** → refuse a full overwrite (`Stale::NeverRead`).
///    Replacing a file nobody has looked at is how a model destroys work it never knew about.
/// 2. **changed since the last read** → refuse (`Stale::Modified`).
/// 3. **only read in part, and now overwriting wholesale** → refuse (`Stale::PartialRead`) —
///    unless the part it could read was cut by the harness's own byte cap, since no amount of
///    re-reading reveals more through this tool.
///
/// `Ok(())` otherwise: a file that does not exist yet, one that was read completely and has not
/// moved, or any targeted edit (which matches exact text the caller had to have seen).
pub fn verify(owner: &Arc<()>, path: &Path, full_overwrite: bool) -> Result<(), Stale> {
    let snapshot = match log().lock() {
        Ok(mut map) => map.paths(owner).get(&key_of(path)).copied(),
        Err(_) => None,
    };
    let Some(snapshot) = snapshot else {
        if full_overwrite && path.exists() {
            return Err(Stale::NeverRead {
                path: path.display().to_string(),
            });
        }
        return Ok(());
    };
    let (mtime, len) = stat(path);
    if snapshot.invalidated || mtime != snapshot.mtime || len != snapshot.len {
        return Err(Stale::Modified {
            path: path.display().to_string(),
            read_len: snapshot.len,
            now_len: len,
        });
    }
    if full_overwrite && snapshot.partial && !snapshot.capped {
        return Err(Stale::PartialRead {
            path: path.display().to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A path unique to this test. The log is process-global and tests run in parallel, so
    /// uniqueness has to come from the name — clearing the map would race the other tests.
    fn tmp(name: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("eps-readlog-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("{name}-{}", std::process::id()));
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        f.sync_all().unwrap();
        p
    }

    #[test]
    fn retired_observation_scopes_are_reclaimed_on_access() {
        let mut observations = Observations::default();
        let owner = Arc::new(());
        let clone = owner.clone();
        observations.paths(&owner);
        assert_eq!(observations.scopes.len(), 1);
        drop(owner);
        observations.paths(&clone);
        assert_eq!(observations.scopes.len(), 1);
        drop(clone);
        let fresh = Arc::new(());
        assert!(observations.paths(&fresh).is_empty());
        assert_eq!(observations.scopes.len(), 1);
    }

    #[test]
    fn a_file_that_does_not_exist_yet_may_be_created() {
        let owner = Arc::new(());
        let dir = std::env::temp_dir().join("eps-readlog-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("absent-{}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        assert_eq!(
            verify(&owner, &p, true),
            Ok(()),
            "creating a file needs no history"
        );
    }

    #[test]
    fn an_existing_file_nobody_read_is_not_overwritten_blind() {
        let owner = Arc::new(());
        // Hermes' write-without-read rule and DSH's `FS_NOT_OBSERVED`: the model is about to
        // replace content it has never seen.
        let p = tmp("unread", "content nobody looked at");
        match verify(&owner, &p, true) {
            Err(Stale::NeverRead { .. }) => {}
            other => panic!("expected a never-read refusal, got {other:?}"),
        }
        let text = verify(&owner, &p, true).unwrap_err().diagnosis("write");
        assert!(text.contains("has not been read"), "{text}");
        assert!(text.contains("read it before overwriting"), "{text}");
        // Reading it once — even partially — is enough to lift this specific refusal, and a
        // targeted edit was never blocked by it.
        record_read(&owner, &p, false, false);
        assert_eq!(
            verify(&owner, &p, true),
            Ok(()),
            "after a read the path is known"
        );
    }

    #[test]
    fn a_window_the_harness_imposed_does_not_block_a_rewrite_forever() {
        let owner = Arc::new(());
        // The 256 KB read cap: no amount of re-reading shows more, so a file larger than it
        // must still be rewritable — otherwise the guard would be a trap.
        let p = tmp("capped", &"x".repeat(4_000));
        record_read(&owner, &p, true, true); // windowed, and windowed *by the cap*
        assert_eq!(
            verify(&owner, &p, true),
            Ok(()),
            "a cap the model cannot lift is not a reason to refuse"
        );
    }

    #[test]
    fn an_unchanged_file_may_be_written() {
        let owner = Arc::new(());
        let p = tmp("unchanged", "hello");
        record_read(&owner, &p, false, false);
        assert_eq!(verify(&owner, &p, true), Ok(()));
    }

    #[test]
    fn a_file_that_changed_underneath_is_refused_with_both_sizes() {
        let owner = Arc::new(());
        let p = tmp("changed", "hello");
        record_read(&owner, &p, false, false);
        // Someone else writes it: longer content, so both mtime and length move.
        let replacement = "hello, and then some";
        std::fs::write(&p, replacement).unwrap();

        match verify(&owner, &p, true) {
            Err(Stale::Modified {
                read_len, now_len, ..
            }) => {
                assert_eq!(read_len, 5);
                assert_eq!(now_len, replacement.len() as u64);
            }
            other => panic!("expected a stale modification, got {other:?}"),
        }
        let text = verify(&owner, &p, true).unwrap_err().diagnosis("write");
        assert!(text.contains("changed on disk"), "{text}");
        assert!(
            text.contains("read it again"),
            "the diagnosis must be actionable: {text}"
        );
        assert!(
            !text.contains("you read"),
            "the reader may have been a sibling agent, so the wording must not assume: {text}"
        );
    }

    #[test]
    fn edits_are_guarded_too() {
        let owner = Arc::new(());
        let p = tmp("edited", "one");
        record_read(&owner, &p, false, false);
        std::fs::write(&p, "two").unwrap();
        assert!(
            matches!(verify(&owner, &p, false), Err(Stale::Modified { .. })),
            "an edit applies old_string to unknown content, so it needs the same guard"
        );
    }

    #[test]
    fn a_partial_read_cannot_justify_a_full_overwrite() {
        let owner = Arc::new(());
        let p = tmp("partial", "line one\nline two\nline three\n");
        record_read(&owner, &p, true, false); // the model saw a window of it
        match verify(&owner, &p, true) {
            Err(Stale::PartialRead { .. }) => {}
            other => panic!("expected a partial-read refusal, got {other:?}"),
        }
        // ...but a targeted edit is still allowed: the model is matching exact text it saw.
        assert_eq!(verify(&owner, &p, false), Ok(()));
        let text = verify(&owner, &p, true).unwrap_err().diagnosis("write");
        assert!(text.contains("only read in part"), "{text}");
        assert!(
            text.contains("read it through first"),
            "and it must say what lifts the refusal: {text}"
        );
    }

    #[test]
    fn our_own_write_does_not_look_like_someone_elses_change() {
        let owner = Arc::new(());
        let p = tmp("selfwrite", "before");
        record_read(&owner, &p, false, false);
        assert_eq!(verify(&owner, &p, true), Ok(()));
        std::fs::write(&p, "after we wrote it").unwrap();
        record_write(&owner, &p);
        assert_eq!(
            verify(&owner, &p, true),
            Ok(()),
            "the harness's own mutation updates the snapshot"
        );
    }
}
