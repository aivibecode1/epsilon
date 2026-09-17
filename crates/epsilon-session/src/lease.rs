//! Cross-process session leases (Hermes' turn-lease pattern).
//!
//! A session file has exactly ONE writer across the machine; other processes can
//! read freely (session discovery never opens a store) but get [`LeaseOutcome::Held`]
//! on write attempts. V1 protocol is deliberately simple:
//!
//! - acquire = create `<session>.lock` with `create_new(true)` (atomic on NTFS and POSIX);
//! - the holder's pid is recorded;
//! - the holder **heartbeats** the file (M61), so a live process is never mistaken
//!   for a crash leftover;
//! - release = remove the file (Drop);
//! - a lock whose mtime is older than [`STALE_AFTER`] is stealable — crash
//!   leftovers must not wedge sessions.
//!
//! The heartbeat is what makes the stale-steal window honest: without it, a TUI
//! open for an hour would look identical to a crashed one after 30 seconds, and
//! stealing would recreate exactly the second-writer corruption the lease exists
//! to prevent.
//!
//! Tradeoff documented: advisory locking via fs4/flock would be race-free on all
//! platforms but needs a dependency and platform-specific review; the mtime-steal
//! window here is acceptable because concurrent writers to one session are a user
//! error we surface loudly rather than serialize silently.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How old a lock file must be before another process may steal it. A live
/// holder re-touches the file every [`HEARTBEAT`], so this bounds how long a
/// *crashed* session stays wedged — not how long a busy one may run.
pub const STALE_AFTER: Duration = Duration::from_secs(30);

/// How often a live holder re-touches its lock file. Comfortably below
/// [`STALE_AFTER`] so ordinary scheduling jitter can never make a working
/// process look dead.
pub const HEARTBEAT: Duration = Duration::from_secs(10);

/// Outcome of trying to acquire a lease.
#[derive(Debug)]
pub enum LeaseOutcome {
    /// We hold it.
    Acquired(SessionLease),
    /// Someone else holds it (and it is not stale).
    Held {
        /// Recorded holder info, when parseable.
        holder: Option<String>,
    },
}

/// An acquired lease; heartbeats while held and removes the lock file on drop.
#[derive(Debug)]
pub struct SessionLease {
    path: PathBuf,
    /// Signals the heartbeat thread to stop, waking it immediately.
    stop: Option<std::sync::mpsc::Sender<()>>,
    heartbeat: Option<std::thread::JoinHandle<()>>,
}

impl SessionLease {
    /// Try to acquire the lease for `session_path` (the `.jsonl`) with the
    /// production [`STALE_AFTER`]/[`HEARTBEAT`] timings.
    pub fn acquire(session_path: &Path) -> std::io::Result<LeaseOutcome> {
        Self::acquire_with(session_path, STALE_AFTER, HEARTBEAT)
    }

    /// [`SessionLease::acquire`] with explicit timings — the lease's own tests
    /// must not spend 30 seconds proving a 30-second window.
    pub fn acquire_with(
        session_path: &Path,
        stale_after: Duration,
        heartbeat: Duration,
    ) -> std::io::Result<LeaseOutcome> {
        let lock_path = session_path.with_extension("lock");

        if lock_path.exists()
            && let Ok(meta) = std::fs::metadata(&lock_path)
        {
            let age = meta
                .modified()
                .ok()
                .and_then(|m| SystemTime::now().duration_since(m).ok())
                .unwrap_or(Duration::ZERO);
            if age < stale_after {
                let holder = std::fs::read_to_string(&lock_path).ok();
                return Ok(LeaseOutcome::Held { holder });
            }
        }
        // Stale or unreadable metadata: steal. The stale file must actually be
        // removed — `create_new` below can only succeed once it is gone, so
        // without this the "steal" path silently degraded to `Held` and a crash
        // leftover wedged the session forever (M61, caught by the lease tests the
        // moment the lease was finally wired into a caller).
        if lock_path.exists() {
            let _ = std::fs::remove_file(&lock_path);
        }

        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut f) => {
                let _ = writeln!(f, "pid={}", std::process::id());
                let _ = f.flush();
                drop(f);
                Ok(LeaseOutcome::Acquired(Self::start_heartbeat(
                    lock_path,
                    stale_after,
                    heartbeat,
                )))
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(LeaseOutcome::Held {
                holder: std::fs::read_to_string(&lock_path).ok(),
            }),
            Err(e) => Err(e),
        }
    }

    /// Start (or take over) the lock at `lock_path` and begin heartbeating it.
    /// The FIRST touch happens immediately: a lease is never born stale.
    fn start_heartbeat(lock_path: PathBuf, stale_after: Duration, heartbeat: Duration) -> Self {
        // Guard against a pathological configuration instead of spinning.
        let interval = heartbeat.max(Duration::from_millis(5)).min(stale_after / 2);
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let thread_path = lock_path.clone();
        let handle = std::thread::Builder::new()
            .name("epsilon-session-lease".into())
            .spawn(move || {
                let _ = touch(&thread_path);
                while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
                    rx.recv_timeout(interval)
                {
                    let _ = touch(&thread_path);
                }
            })
            .ok();
        Self {
            path: lock_path,
            stop: Some(tx),
            heartbeat: handle,
        }
    }

    /// Touch the lock now (the heartbeat does this on a timer; this is for
    /// callers that want the guarantee at a specific moment, e.g. right before a
    /// long write).
    pub fn refresh(&self) -> std::io::Result<()> {
        touch(&self.path)
    }

    /// The lock file this lease owns (`<session>.lock`).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Explicitly release (also happens on Drop).
    pub fn release(self) {}
}

/// Re-stamp the lock's mtime. `set_modified` (std, 1.75+) needs a writable
/// handle; opening with `.write(true)` rather than `.append(true)` is the M4
/// lesson — an append-only handle cannot rewrite metadata-bearing state.
fn touch(lock_path: &Path) -> std::io::Result<()> {
    let f = std::fs::OpenOptions::new().write(true).open(lock_path)?;
    f.set_modified(SystemTime::now())
}

impl Drop for SessionLease {
    fn drop(&mut self) {
        // Wake the heartbeat first so it cannot re-touch a file we are deleting
        // (which would leave a fresh-looking lock behind with nobody holding it).
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.heartbeat.take() {
            let _ = handle.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_is_exclusive_and_released_on_drop() {
        let dir = tempfile::tempdir().expect("tmp");
        let session = dir.path().join("s.jsonl");
        std::fs::write(&session, "").expect("seed");

        let first = SessionLease::acquire(&session).expect("acquire 1");
        let LeaseOutcome::Acquired(a) = first else {
            panic!("first acquisition must succeed");
        };

        let second = SessionLease::acquire(&session).expect("probe ok");
        assert!(matches!(second, LeaseOutcome::Held { .. }));

        drop(a); // released
        match SessionLease::acquire(&session).expect("acquire after drop") {
            LeaseOutcome::Acquired(_) => {}
            other => panic!("expected reacquisition, got {other:?}"),
        }
    }

    #[test]
    fn a_live_holder_is_never_stolen_from() {
        // The M61 defect this heartbeat exists for: with a 150 ms stale window
        // and no heartbeat, a process that is *alive* would be stealable after
        // 150 ms — recreating the second-writer corruption the lease prevents.
        let dir = tempfile::tempdir().expect("tmp");
        let session = dir.path().join("s.jsonl");
        std::fs::write(&session, "").expect("seed");
        let stale = Duration::from_millis(150);
        let beat = Duration::from_millis(20);

        let LeaseOutcome::Acquired(held) =
            SessionLease::acquire_with(&session, stale, beat).expect("acquire")
        else {
            panic!("first acquisition must succeed");
        };
        // Several stale windows pass while the holder is alive.
        std::thread::sleep(Duration::from_millis(450));
        let probe = SessionLease::acquire_with(&session, stale, beat).expect("probe");
        assert!(
            matches!(probe, LeaseOutcome::Held { .. }),
            "a heartbeating holder must not be stolen from: {probe:?}"
        );
        // ...and the lock really is being re-stamped, not merely young.
        let stamped = std::fs::metadata(held.path())
            .expect("lock")
            .modified()
            .expect("mtime");
        let fresh = SystemTime::now()
            .duration_since(stamped)
            .unwrap_or(Duration::ZERO);
        assert!(
            fresh < stale,
            "heartbeat kept the lock fresh (age {fresh:?})"
        );
    }

    #[test]
    fn a_dead_holders_lock_is_stolen_after_the_stale_window() {
        // A crash leaves the lock file behind with nobody to heartbeat it.
        let dir = tempfile::tempdir().expect("tmp");
        let session = dir.path().join("s.jsonl");
        std::fs::write(&session, "").expect("seed");
        let lock = session.with_extension("lock");
        std::fs::write(&lock, "pid=999999\n").expect("fake leftover");
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&lock)
            .expect("open");
        file.set_modified(SystemTime::now() - Duration::from_secs(120))
            .expect("age it");

        let outcome =
            SessionLease::acquire_with(&session, Duration::from_secs(30), Duration::from_secs(10))
                .expect("acquire");
        assert!(
            matches!(outcome, LeaseOutcome::Acquired(_)),
            "a crash leftover must be stealable, not wedging: {outcome:?}"
        );
    }

    #[test]
    fn dropping_a_lease_stops_the_heartbeat_and_removes_the_lock() {
        let dir = tempfile::tempdir().expect("tmp");
        let session = dir.path().join("s.jsonl");
        std::fs::write(&session, "").expect("seed");
        let lock = session.with_extension("lock");
        {
            // NB: binding matters — `Acquired(_)` would drop the lease (and its
            // lock file) at the end of the statement, which is Drop behaving
            // exactly as designed.
            let _held = match SessionLease::acquire_with(
                &session,
                Duration::from_secs(30),
                Duration::from_millis(20),
            )
            .expect("acquire")
            {
                LeaseOutcome::Acquired(held) => held,
                other => panic!("expected Acquired, got {other:?}"),
            };
            assert!(lock.exists());
        }
        assert!(!lock.exists(), "drop removes the lock");
        // Nothing is re-touching it: give a would-be heartbeat time to misfire.
        std::thread::sleep(Duration::from_millis(80));
        assert!(!lock.exists(), "the heartbeat thread stops with its lease");
    }

    #[test]
    fn held_reports_the_holder_pid() {
        let dir = tempfile::tempdir().expect("tmp");
        let session = dir.path().join("s.jsonl");
        std::fs::write(&session, "").expect("seed");
        let LeaseOutcome::Acquired(_held) = SessionLease::acquire(&session).expect("acquire")
        else {
            panic!("acquired");
        };
        match SessionLease::acquire(&session).expect("probe") {
            LeaseOutcome::Held { holder } => {
                let holder = holder.expect("holder text");
                assert!(
                    holder.contains(&format!("pid={}", std::process::id())),
                    "the conflicting process can name who holds it: {holder}"
                );
            }
            other => panic!("expected Held, got {other:?}"),
        }
    }
}
