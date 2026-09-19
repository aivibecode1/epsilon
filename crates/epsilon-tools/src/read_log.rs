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
//! What counts as seen is *coverage*, not a flag (M95): the read tool pages, so a record
//! states which lines a read covered, whether it reached the end of the file, and whether the
//! byte budget alone stopped it. Coverage accumulates only across contiguous reads of the same
//! bytes, and a full overwrite needs the whole file — a page of a 5 MB file is not a license
//! to replace it (F01).
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
    /// The whole file was seen: every line of the content this snapshot describes was shown
    /// to the model (a read that covered lines 1..=total, or a write of our own). A partial
    /// view cannot justify an overwrite — the model would be replacing text it never saw.
    complete: bool,
    /// Highest line number covered by reads contiguous from line 1 (0 = none). Only
    /// meaningful while `complete` is false; a gap at the top cannot be filled in from below.
    covered: usize,
    /// The file's exact line count, when a read scanned to EOF (`None` = never established).
    total: Option<usize>,
    /// The read that produced this snapshot was stopped by a single line longer than the read
    /// byte budget (pi's `firstLineExceedsLimit`, `read.ts:297-301`) — the one window no
    /// further `read` can widen, because every later read of that line hits the same cap.
    /// (M65 recorded this as "the harness's byte cap"; M95 narrowed it, since a *page* is a
    /// window the caller can widen by asking for the next offset and a file larger than a
    /// page is exactly the case pagination made readable.)
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

/// How much of a file one read covered (M95).
///
/// pi's read tool reads the whole file every time and can always say what it saw; a streamed
/// page knows only what it scanned, so coverage is stated as lines rather than a bool. Reads
/// accumulate: pages that are contiguous from line 1 extend the covered high-water mark, and
/// a read that reaches EOF establishes the file's exact line count, at which point coverage
/// becomes completeness. Out-of-order pages extend nothing — a page that starts below the
/// mark would have to be re-read, and a model that reads the tail first must still page from
/// the top (the conservative direction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coverage {
    /// First line number the read showed (0 = it showed nothing).
    pub from: usize,
    /// Last line number the read showed (0 = it showed nothing).
    pub to: usize,
    /// The file's exact line count, when the read scanned to EOF.
    pub total: Option<usize>,
    /// The read was stopped by a single line over the byte budget — pi's
    /// `firstLineExceedsLimit`, the one window no later read through this tool can widen.
    pub capped: bool,
}

impl Coverage {
    /// A read that showed lines `from..=to`. `total` is the file's exact line count when the
    /// scan reached EOF — `None` only for a read stopped by an over-cap line, which is
    /// [`Coverage::long_line`].
    pub fn page(from: usize, to: usize, total: Option<usize>) -> Self {
        Self {
            from,
            to,
            total,
            capped: false,
        }
    }

    /// A read stopped by a line longer than the byte budget: the page showed nothing.
    pub fn long_line() -> Self {
        Self {
            from: 0,
            to: 0,
            total: None,
            capped: true,
        }
    }
}

/// Record that the model has now seen `path` as it is on disk.
///
/// `coverage` says *how much* of it: the lines this read showed, whether it scanned to EOF,
/// and whether the byte budget alone is what stopped it. Coverage accumulates only when it is
/// honest to do so — a previous snapshot whose bytes are gone (the file changed, or a sibling
/// wrote it) contributes nothing, because its lines described other content.
pub fn record_read(owner: &Arc<()>, path: &Path, coverage: Coverage) {
    let (mtime, len) = stat(path);
    if let Ok(mut map) = log().lock() {
        let key = key_of(path);
        let prev = map.paths(owner).get(&key).copied();
        // A snapshot describes the bytes it was taken against. When the file moved since, or
        // a sibling write invalidated it, the old coverage says nothing about this content.
        let prev = match prev {
            Some(p) if !p.invalidated && p.mtime == mtime && p.len == len => p,
            _ => Snapshot {
                invalidated: false,
                mtime,
                len,
                complete: false,
                covered: 0,
                total: None,
                capped: false,
            },
        };
        let covered = if coverage.from == 0 {
            prev.covered
        } else if coverage.from <= prev.covered.saturating_add(1) {
            prev.covered.max(coverage.to)
        } else {
            prev.covered
        };
        let total = coverage.total.or(prev.total);
        // A window no further read can widen does not vanish because a later read said
        // something else: the over-cap line stays unreadable through this tool, and a guard
        // that can never be satisfied is a trap (M65's rule, kept).
        let capped = prev.capped || coverage.capped;
        // `complete` is sticky: the stat above says these are the same bytes the model
        // already saw in full. An empty file is complete from its first read (nothing to
        // see), which is why this is a line-coverage test and not "something was shown".
        let complete = prev.complete || Some(covered) == total;
        map.paths(owner).insert(
            key,
            Snapshot {
                invalidated: false,
                mtime,
                len,
                complete,
                covered,
                total,
                capped,
            },
        );
    }
}

/// Record the state a successful mutation just produced, so the model's own change does not
/// look like someone else's — and so the bytes it just wrote are known in full.
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
                complete: true,
                covered: 0,
                total: Some(0),
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
/// 3. **not seen in full, and now overwriting wholesale** → refuse (`Stale::PartialRead`).
///    "In full" means every line of the current content was covered by reads contiguous from
///    line 1 (`Snapshot::complete`) — a page of a larger file is precisely not that. The one
///    exemption is `capped`: the read was stopped by a single line longer than the read byte
///    budget, which no further read of this tool can ever show, and a guard that cannot be
///    satisfied is a trap (M65's rule, narrowed in M95 — the old byte-cap exemption died with
///    pagination, because the next offset now *can* widen the window).
///
/// `Ok(())` otherwise: a file that does not exist yet, one that was read to the end and has
/// not moved, or any targeted edit (which matches exact text the caller had to have seen).
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
    if full_overwrite && !snapshot.complete && !snapshot.capped {
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

    /// A whole-file coverage as `read_page` reports it for a file of `lines` lines.
    fn whole(lines: usize) -> Coverage {
        Coverage::page(1, lines, Some(lines))
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
        record_read(&owner, &p, whole(1));
        assert_eq!(
            verify(&owner, &p, true),
            Ok(()),
            "after a read the path is known"
        );
    }

    #[test]
    fn a_page_of_a_larger_file_cannot_justify_a_full_overwrite() {
        let owner = Arc::new(());
        // M65 exempted a file the harness's own 256 KB cap had cut, on the premise that no
        // further read could widen the window. Pagination made that premise false: the next
        // offset *can* widen it, so a page is a partial view like any other window.
        let body: String = (1..=300).map(|i| format!("line {i}\n")).collect();
        let p = tmp("paged", &body);
        record_read(&owner, &p, Coverage::page(1, 200, Some(300)));
        match verify(&owner, &p, true) {
            Err(Stale::PartialRead { .. }) => {}
            other => panic!("expected a partial-read refusal, got {other:?}"),
        }
        // The continuation page closes it: the model has now seen every line.
        record_read(&owner, &p, Coverage::page(201, 300, Some(300)));
        assert_eq!(
            verify(&owner, &p, true),
            Ok(()),
            "paging to the end makes the whole file known"
        );
    }

    #[test]
    fn a_line_longer_than_the_byte_budget_is_the_only_window_left_open() {
        let owner = Arc::new(());
        // pi answers an over-cap single line with a `sed` fallback (`read.ts:297-301`); no
        // read through this tool can ever show it, so refusing forever would be a trap.
        let body = format!("{}\nsecond\nthird\n", "x".repeat(4_000));
        let p = tmp("longline", &body);
        record_read(&owner, &p, Coverage::long_line());
        assert_eq!(
            verify(&owner, &p, true),
            Ok(()),
            "a window this tool can never widen is not a reason to refuse"
        );
        // ...and it stays open after later pages: the unreadable line cannot become readable.
        record_read(&owner, &p, Coverage::page(2, 3, Some(3)));
        assert_eq!(
            verify(&owner, &p, true),
            Ok(()),
            "a later page must not turn the unreadable line into a trap"
        );
    }

    #[test]
    fn pages_extend_coverage_only_when_they_are_contiguous() {
        let owner = Arc::new(());
        let body: String = (1..=10).map(|i| format!("line {i}\n")).collect();
        let p = tmp("coverage", &body);
        // Reading the tail first extends nothing: coverage is the high-water mark from line 1.
        record_read(&owner, &p, Coverage::page(5, 6, Some(10)));
        assert!(
            matches!(verify(&owner, &p, true), Err(Stale::PartialRead { .. })),
            "an out-of-order page is not coverage"
        );
        record_read(&owner, &p, Coverage::page(1, 4, Some(10)));
        assert!(
            matches!(verify(&owner, &p, true), Err(Stale::PartialRead { .. })),
            "lines 1-4 of 10 are still a partial view"
        );
        // The page that starts at the mark (5 = 4+1) closes the file.
        record_read(&owner, &p, Coverage::page(5, 10, Some(10)));
        assert_eq!(
            verify(&owner, &p, true),
            Ok(()),
            "contiguous pages from line 1 clear the partial state"
        );
    }

    #[test]
    fn coverage_of_one_content_does_not_cover_another() {
        let owner = Arc::new(());
        let p = tmp("fresh", "a\nb\nc\n");
        record_read(&owner, &p, whole(3));
        assert_eq!(verify(&owner, &p, true), Ok(()));
        // Someone else appends a line; the model re-reads only the first one.
        std::fs::write(&p, "a\nb\nc\nd\n").unwrap();
        record_read(&owner, &p, Coverage::page(1, 1, Some(4)));
        assert!(
            matches!(verify(&owner, &p, true), Err(Stale::PartialRead { .. })),
            "a whole-file read of the old bytes must not cover the new ones"
        );
    }

    #[test]
    fn our_own_write_is_not_downgraded_by_a_later_page_read() {
        let owner = Arc::new(());
        let p = tmp("selfwrite-paged", "before");
        record_write(&owner, &p);
        record_read(&owner, &p, Coverage::page(1, 1, Some(1)));
        assert_eq!(
            verify(&owner, &p, true),
            Ok(()),
            "the model authored the whole file; a page re-read does not take that back"
        );
    }

    #[test]
    fn an_unchanged_file_may_be_written() {
        let owner = Arc::new(());
        let p = tmp("unchanged", "hello");
        record_read(&owner, &p, whole(1));
        assert_eq!(verify(&owner, &p, true), Ok(()));
    }

    #[test]
    fn a_file_that_changed_underneath_is_refused_with_both_sizes() {
        let owner = Arc::new(());
        let p = tmp("changed", "hello");
        record_read(&owner, &p, whole(1));
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
        record_read(&owner, &p, whole(1));
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
        record_read(&owner, &p, Coverage::page(1, 1, Some(3))); // the model saw a window of it
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
        record_read(&owner, &p, whole(1));
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
