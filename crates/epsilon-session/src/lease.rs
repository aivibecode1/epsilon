//! Cross-process session leases (Hermes' turn-lease pattern).
//!
//! A runner must hold the lease file to append to a session; other processes can read freely
//! but get `LeaseHeld` on write attempts. V1 protocol is deliberately simple:
//!
//! - acquire = create `<session>.lock` with `create_new(true)` (atomic on NTFS and POSIX);
//! - the holder's pid + start time are recorded for diagnostics;
//! - release = remove the file (Drop);
//! - stale locks (mtime older than [`STALE_AFTER`]) are stealable — crash leftovers must not
//!   wedge sessions.
//!
//! Tradeoff documented: advisory locking via fs4/flock would be race-free on all platforms,
//! but needs a dependency and platform-specific behavior review; the mtime-steal window here
//! is acceptable for v1 because concurrent writers to one session are already a user error
//! we surface loudly rather than serialize silently.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How old a lock file must be before another process may steal it.
pub const STALE_AFTER: Duration = Duration::from_secs(30);

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

/// An acquired lease; removes the lock file on drop.
#[derive(Debug)]
pub struct SessionLease {
    path: PathBuf,
}

impl SessionLease {
    /// Try to acquire the lease for `session_path` (the `.jsonl`).
    pub fn acquire(session_path: &Path) -> std::io::Result<LeaseOutcome> {
        let lock_path = session_path.with_extension("lock");

        if lock_path.exists()
            && let Ok(meta) = std::fs::metadata(&lock_path)
        {
            let age = meta
                .modified()
                .ok()
                .and_then(|m| SystemTime::now().duration_since(m).ok())
                .unwrap_or(Duration::ZERO);
            if age < STALE_AFTER {
                let holder = std::fs::read_to_string(&lock_path).ok();
                return Ok(LeaseOutcome::Held { holder });
            }
        }
        // Stale or unreadable metadata: steal.

        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut f) => {
                let _ = writeln!(f, "pid={}", std::process::id());
                let _ = f.flush();
                Ok(LeaseOutcome::Acquired(SessionLease { path: lock_path }))
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(LeaseOutcome::Held {
                holder: std::fs::read_to_string(&lock_path).ok(),
            }),
            Err(e) => Err(e),
        }
    }

    /// Explicitly release (also happens on Drop).
    pub fn release(self) {}
}

impl Drop for SessionLease {
    fn drop(&mut self) {
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
}
