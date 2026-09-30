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
//!
//! # Why acquisition waits instead of refusing (M136)
//!
//! Because a live holder heartbeats, the mtime already answers the only question
//! that matters — *is anyone still there?* — and it answers it without a platform
//! call. So the refusal did not have to be immediate. Refusing straight away made
//! the ordinary way to restart a TUI fail: press Ctrl-C, close the tab, and get
//! "another epsilon is already writing this session" for up to [`STALE_AFTER`]
//! seconds, because the process you just killed is not heartbeating any more.
//! The console probe hit exactly this and had to work around it.
//!
//! So [`SessionLease::acquire`] **waits out the remaining window and then steals**.
//! The trade is obvious once stated: in the only case that costs anything, the
//! holder really is alive, and waiting is the correct behaviour anyway — the user
//! is looking at a second window that owns the session. The alternative considered
//! and rejected was a liveness probe of the recorded pid, which needs a platform
//! call (and `unsafe`, which this workspace forbids) to learn *less* than the
//! heartbeat already tells us, while adding pid-reuse as a new failure mode.
//!
//! # Why the wait now gives up early when the holder is provably gone
//!
//! The reasoning above is right and the consequence was still wrong. Waiting the
//! full window is only necessary when the holder *might* be alive. When it is
//! **provably** not, the wait buys nothing — and it costs the user the one thing
//! that made this the worst bug in the TUI.
//!
//! Measured on 2026-09-28 with a lock left by a `taskkill /F`, at 100x30 in a real
//! conhost: the process is alive, tokens are being consumed underneath, and the
//! screen shows one truncated line and no input for **30 seconds**:
//!
//! ```text
//!  waiting for this session to be released — 4s elapsed, ~26s left (another process may still be writ
//! ```
//!
//! That is precisely the shape of the reported typing-freeze. A user who closes a
//! terminal window pays it on every launch, and the difference from a real freeze
//! is not visible from outside the process.
//!
//! The fix keeps M136's property — a *live* holder is never stolen from — and
//! removes the dead-holder case from the wait entirely. Two conditions, both of
//! which must hold:
//!
//! 1. **The lock has not moved.** A holder heartbeats every [`HEARTBEAT`], so a
//!    lock whose mtime is unchanged across a full heartbeat period belongs to a
//!    process that is not running. This is the same evidence the steal already
//!    uses; it is simply used sooner.
//! 2. **The window is the crash case, not the contention case.** A *fresh* lock
//!    whose holder has been alive for less than a heartbeat is not stale enough to
//!    judge, so it is left to the normal wait.
//!
//! What is deliberately *not* done is probing the recorded pid. The original
//! design rejected it for good reasons — it needs `unsafe`, which this workspace
//! forbids, and pid reuse makes it wrong in a way that is hard to detect. The
//! mtime comparison gets the same answer with no platform call and no new failure
//! mode, which is why the reason the design gave for avoiding a pid probe does not
//! apply to it.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How old a lock file must be before another process may steal it. A live
/// holder re-touches the file every [`HEARTBEAT`], so this bounds how long a
/// *crashed* session stays wedged — not how long a busy one may run.
pub const STALE_AFTER: Duration = Duration::from_secs(30);

/// How long [`SessionLease::acquire`] waits before reporting `Held` (M136).
///
/// Deliberately equal to [`STALE_AFTER`]: the wait exists to let a *dead* holder's
/// lock go stale, so any shorter and the ordinary restart case would still be
/// refused. Naming it separately, rather than reusing `STALE_AFTER` at the call
/// site, is so that "wait at least as long as staleness" is a visible invariant
/// with a test on it rather than a coincidence two constants happen to share.
pub const ACQUIRE_WAIT: Duration = STALE_AFTER;

/// How often a live holder re-touches its lock file. Comfortably below
/// [`STALE_AFTER`] so ordinary scheduling jitter can never make a working
/// process look dead.
pub const HEARTBEAT: Duration = Duration::from_secs(2);

/// How long a fresh lock must sit **unmoved** before it is treated as a dead
/// holder's rather than a live one's.
///
/// **This must be at least one [`HEARTBEAT`], and the reason is a bug I wrote and
/// then caught.** The first attempt set it to three seconds while the heartbeat was
/// ten, on the reasoning that a settle window only has to be long enough for two
/// samples of a file to agree. That reasoning is wrong, and `session_store.rs`'s
/// `a_session_file_has_one_writer_at_a_time` caught it immediately: a live holder
/// re-stamps every ten seconds, so during a three-second window its lock does not
/// move, and the acquirer concluded the holder was dead. It stole the lock from a
/// running process — the exact second-writer corruption the lease exists to
/// prevent.
///
/// The rule is not "the file did not change". It is **"the file did not change for
/// longer than a live holder would have taken to change it."** Those are only the
/// same statement when the window is at least one heartbeat, so the heartbeat was
/// shortened instead: 2 s, with a 4 s settle window, is a comfortable factor of two
/// over it.
///
/// The cost is one `set_modified` on a ten-byte file every two seconds, which is
/// nothing, and the benefit is that a killed TUI recovers in ~4 seconds instead of
/// 30. A user who closes a terminal window is not left staring at
/// `waiting for this session to be released — 4s elapsed, ~26s left` with a live
/// process underneath and no input accepted, which is what the 2026-09-28
/// measurement recorded and what the reported typing-freeze looks like from outside.
pub const DEAD_HOLDER_SETTLE: Duration = Duration::from_secs(4);

/// How often a waiting acquirer re-reads the lock while it waits out the stale
/// window. Small enough that a holder which exits mid-wait is noticed at once,
/// large enough that a 30-second wait is not thousands of syscalls.
const WAIT_POLL: Duration = Duration::from_millis(100);

/// Delete lock files in `dir` whose holder has provably gone, and report how many were removed.
///
/// ## Why this is safe, in the same terms the steal is
///
/// The predicate is **exactly** the one [`SessionLease::acquire`] already uses to decide a lock may
/// be taken: an mtime that has not moved for [`STALE_AFTER`]. A live holder re-touches its lock
/// every [`HEARTBEAT`], so a lock untouched for thirty seconds has no live holder — and `acquire`
/// would steal it without hesitating. This function deletes precisely the files `acquire` would
/// otherwise have to work around, so it can neither take a lock from a living process nor change
/// any decision: removing a stealable lock only saves the next process from watching it.
///
/// ## Why it is worth doing at all
///
/// One session per file (M210) means one *lock* per session, and a session whose process was killed
/// — a closed terminal window, a `taskkill` — never removes its own. Without this, a workspace used
/// daily accumulates two entries per launch and half of them are dead, in a directory a user lists
/// when they go looking for their sessions. Discovery filters `*.jsonl` and is unaffected; a human
/// reading the directory is not.
///
/// Errors are deliberately invisible here: a directory that cannot be read, or a file that cannot be
/// removed (another process just took it, permissions, a Windows share), is not a reason to fail a
/// launch. The caller gets a count for logging and nothing else.
pub fn sweep_stale_locks(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_none_or(|x| x != "lock") {
            continue;
        }
        let stale = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > STALE_AFTER);
        if stale && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Outcome of trying to acquire a lease.
#[derive(Debug)]
pub enum LeaseOutcome {
    /// We hold it.
    Acquired(SessionLease),
    /// Someone else holds it, and they are still holding it.
    Held {
        /// Recorded holder info, when parseable.
        holder: Option<String>,
        /// How long we waited before giving up. `Duration::ZERO` when the caller
        /// asked not to wait.
        waited: Duration,
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
    /// Try to acquire the lease for `session_path` (the `.jsonl`).
    ///
    /// If another process holds it, this **waits up to `wait`** for that holder to
    /// finish or die, and takes the lock the moment it is no longer heartbeating.
    /// It returns [`LeaseOutcome::Held`] only when the holder is still alive at the
    /// end of the wait — a genuine second writer, which the caller must still
    /// refuse.
    pub fn acquire(session_path: &Path) -> std::io::Result<LeaseOutcome> {
        Self::acquire_with(session_path, STALE_AFTER, HEARTBEAT, ACQUIRE_WAIT)
    }

    /// Acquire without waiting at all: report `Held` the moment anyone else has it.
    /// The pre-M136 behaviour, kept because it is the only way to test both
    /// outcomes without spending real seconds, and because "refuse immediately" is
    /// the right answer for a caller that has its own timeout.
    pub fn acquire_now(session_path: &Path) -> std::io::Result<LeaseOutcome> {
        Self::acquire_with(session_path, STALE_AFTER, HEARTBEAT, Duration::ZERO)
    }

    /// [`SessionLease::acquire`] with explicit timings and wait budget — the
    /// lease's own tests must not spend 30 seconds proving a 30-second window.
    pub fn acquire_with(
        session_path: &Path,
        stale_after: Duration,
        heartbeat: Duration,
        wait: Duration,
    ) -> std::io::Result<LeaseOutcome> {
        Self::acquire_with_progress(session_path, stale_after, heartbeat, wait, |_, _| {})
    }

    /// [`SessionLease::acquire_with`], plus a progress callback.
    ///
    /// M136 made acquisition *wait* rather than refuse, and left the user with a
    /// blank screen for up to 30 seconds — the same "nothing is happening" shape as
    /// the freeze this harness is otherwise fixing. The session crate cannot own
    /// that display (it is a library, and `epsilon-tui` cannot be its dependency),
    /// so it reports what it is doing and lets the frontend draw it.
    ///
    /// `on_wait` is called once per poll with `(waited, remaining)`. It is a
    /// progress report, not a hook: returning `()` means the caller cannot
    /// influence the outcome, which is deliberate — a frontend must not be able to
    /// shorten or extend a correctness window.
    ///
    /// Callers that do not care pass `|_, _| {}`; that is what every test and
    /// every non-interactive path does, so the default costs nothing.
    pub fn acquire_with_progress(
        session_path: &Path,
        stale_after: Duration,
        heartbeat: Duration,
        wait: Duration,
        on_wait: impl Fn(Duration, Duration),
    ) -> std::io::Result<LeaseOutcome> {
        let lock_path = session_path.with_extension("lock");
        let started = std::time::Instant::now();
        // The mtime seen on the previous poll, so the next one can tell "still here"
        // from "nobody is re-stamping this". `None` on the first poll: one sighting
        // is not evidence of anything.
        let mut seen_mtime: Option<SystemTime> = None;
        // The elapsed time at which the lock was first observed to be *not moving*.
        // `Duration` and not `Instant`, because the comparison is against
        // `started.elapsed()`. `Duration::default()` is zero, which is not a
        // sentinel: on the first frozen sighting the elapsed time is tiny but
        // non-zero, and treating zero as "just now" is what stops a caller that
        // has already been waiting a long time from stealing on its second poll.
        let mut frozen_since: Option<Duration> = None;
        // How long a fresh-but-unmoved lock must be watched before it is called
        // dead. Scaled down alongside the heartbeat by tests that pass a heartbeat
        // shorter than the real one, so a 4-second production window costs
        // milliseconds in a unit test. `HEARTBEAT` is the floor: a window shorter
        // than one heartbeat cannot distinguish a dead holder from a live one.
        let settle = if heartbeat < HEARTBEAT {
            DEAD_HOLDER_SETTLE
                .min(heartbeat * 2)
                .max(Duration::from_millis(50))
        } else {
            DEAD_HOLDER_SETTLE
        };

        // One pass: is the lock free, stale, or freshly held? Re-reading on every
        // pass is what lets a holder that exits mid-wait be noticed immediately
        // rather than after the remainder of the window. Written as a named shape
        // rather than a nested `Option<Option<String>>` because the nesting is
        // exactly what makes the call site wrong — `Some(x)` hands back `x`, so
        // `if let Some(holder) = …` silently drops the inner Option and every
        // reader has to re-derive which layer is which.
        enum LockState {
            /// Absent or old enough to steal.
            Stealable,
            /// Someone has it, and it was still fresh. Carries the holder line
            /// when the file was readable.
            Held(Option<String>),
        }

        // `mut` because the closure records what it saw, so the next poll can
        // compare against it. A closure that could not remember would be a
        // single-sighting probe, which is the M136 behaviour this replaces.
        let mut probe = |lock_path: &Path| -> LockState {
            if !lock_path.exists() {
                return LockState::Stealable;
            }
            let age = std::fs::metadata(lock_path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|m| SystemTime::now().duration_since(m).ok())
                .unwrap_or(Duration::ZERO);
            if age >= stale_after {
                return LockState::Stealable;
            }
            // Fresh, so it is held. But "fresh" is not the same as "alive", and
            // those two cases deserve different treatment: a lock whose mtime has
            // not moved for longer than a live holder would have taken to move it
            // belongs to a process that is not running.
            //
            // The clock is `frozen_at`, NOT the start of the wait — and that
            // distinction is the whole correctness of this rule. Timestamps are
            // coarse (NTFS and ext4 both quantise, and Windows' FAT-family
            // resolutions are coarser still), so two samples taken 100ms apart can
            // legitimately carry the same mtime. Timing from the first *equal* pair
            // means the window measures how long the lock has been sitting still,
            // which is the question. Timing from `started` measures how long the
            // caller has been waiting, which is a different question and produced
            // a bug: it fired on the second poll of a lock that a live holder was
            // about to re-stamp, and stole from a running process.
            let frozen = mtime_of(lock_path);
            match (frozen, seen_mtime) {
                (Some(now), Some(previous)) if now == previous => {
                    // The clock runs from the first sighting of an unmoved lock.
                    // It is stored as an *elapsed* `Duration` rather than an
                    // `Instant`, because the two are not interchangeable: `started`
                    // is an absolute monotonic point, and subtracting one from the
                    // other compares system uptime against a session-local age. They
                    // agree only when the process happens to have started at boot,
                    // which on Windows it has not — that mismatch is why the first
                    // version of this rule silently never fired, and it is worth the
                    // extra type because the wrong version is a silent failure.
                    // The first frozen sighting stamps the clock; every later one
                    // keeps the original stamp, so the window measures how long the
                    // lock has actually been still.
                    let since = frozen_since.unwrap_or_else(|| started.elapsed());
                    if started.elapsed().saturating_sub(since) >= settle {
                        return LockState::Stealable;
                    }
                    frozen_since = Some(since);
                }
                _ => frozen_since = None,
            }
            seen_mtime = frozen;
            LockState::Held(std::fs::read_to_string(lock_path).ok())
        };

        let holder = loop {
            match probe(&lock_path) {
                // Free or stale: fall through to the steal-and-create below.
                LockState::Stealable => break None,
                LockState::Held(holder) => {
                    let elapsed = started.elapsed();
                    if elapsed >= wait {
                        break Some(holder);
                    }
                    // Never sleep past the deadline: a 30s wait must not become 30.1s.
                    let remaining = wait.saturating_sub(elapsed);
                    // Report before sleeping, so the first notice appears at once
                    // rather than one poll interval into a wait the user cannot see.
                    on_wait(elapsed, remaining);
                    std::thread::sleep(remaining.min(WAIT_POLL));
                }
            }
        };
        let waited = started.elapsed();

        // A `Held` we gave up on. Note this is decided by the wait budget, not by
        // the lock's age: a holder that heartbeats stays fresh forever, so waiting
        // is the only way to be sure, and a caller that does not want to wait gets
        // today's answer.
        if let Some(holder) = holder {
            return Ok(LeaseOutcome::Held { holder, waited });
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
                // Someone won the race between our last check and the create.
                waited: started.elapsed(),
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

/// The lock file's modification time, or `None` if it cannot be read.
///
/// A missing lock is not an error here: the caller has already established that
/// the file exists, and a race that removes it between that check and this read
/// is handled by the next poll.
fn mtime_of(lock_path: &Path) -> Option<SystemTime> {
    std::fs::metadata(lock_path).ok()?.modified().ok()
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
    fn the_sweep_removes_dead_locks_and_keeps_live_ones() {
        let dir = tempfile::tempdir().expect("tmp");
        let dead = dir.path().join("a.lock");
        let live = dir.path().join("b.lock");
        std::fs::write(&dead, "pid=999999\n").expect("dead holder left this");
        std::fs::write(&live, "pid=1\n").expect("a live holder's lock");
        // The dead one is aged past the window; the live one is fresh, exactly as a heartbeating
        // holder leaves it. Both shapes are what `acquire` would see.
        std::fs::OpenOptions::new()
            .write(true)
            .open(&dead)
            .expect("open")
            .set_modified(SystemTime::now() - STALE_AFTER - Duration::from_secs(1))
            .expect("age it past the window");

        let removed = sweep_stale_locks(dir.path());
        assert_eq!(removed, 1, "exactly the stale lock, not the fresh one");
        assert!(!dead.exists(), "a dead holder's lock is reaped");
        assert!(
            live.exists(),
            "a live holder's lock must survive — removing it would be the theft the \
             settle exists to prevent"
        );
    }

    /// A file that is not a lock is not ours to remove, however old it is: the sessions directory
    /// holds the session files themselves, and a sweep that matched `*` would delete a user's
    /// history.
    #[test]
    fn the_sweep_touches_nothing_but_lock_files() {
        let dir = tempfile::tempdir().expect("tmp");
        let session = dir.path().join("old.jsonl");
        std::fs::write(&session, "{}\n").expect("a session file");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&session)
            .expect("open")
            .set_modified(SystemTime::now() - STALE_AFTER - Duration::from_secs(60))
            .expect("age it");

        assert_eq!(sweep_stale_locks(dir.path()), 0);
        assert!(session.exists(), "the sweep must not touch session files");
    }

    #[test]
    fn lease_is_exclusive_and_released_on_drop() {
        let dir = tempfile::tempdir().expect("tmp");
        let session = dir.path().join("s.jsonl");
        std::fs::write(&session, "").expect("seed");

        let first = SessionLease::acquire(&session).expect("acquire 1");
        let LeaseOutcome::Acquired(a) = first else {
            panic!("first acquisition must succeed");
        };

        let second = SessionLease::acquire_now(&session).expect("probe ok");
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
            SessionLease::acquire_with(&session, stale, beat, Duration::ZERO).expect("acquire")
        else {
            panic!("first acquisition must succeed");
        };
        // Several stale windows pass while the holder is alive.
        std::thread::sleep(Duration::from_millis(450));
        let probe =
            SessionLease::acquire_with(&session, stale, beat, Duration::ZERO).expect("probe");
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

        let outcome = SessionLease::acquire_with(
            &session,
            Duration::from_secs(30),
            Duration::from_secs(10),
            Duration::ZERO,
        )
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
                Duration::ZERO,
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
        match SessionLease::acquire_now(&session).expect("probe") {
            LeaseOutcome::Held { holder, .. } => {
                let holder = holder.expect("holder text");
                assert!(
                    holder.contains(&format!("pid={}", std::process::id())),
                    "the conflicting process can name who holds it: {holder}"
                );
            }
            other => panic!("expected Held, got {other:?}"),
        }
    }

    // ---- M136: waiting out the stale window instead of refusing outright ----

    /// The defect this milestone fixes, in miniature: a lock nobody is heartbeating
    /// is a dead holder, and a caller that refuses immediately turns "reopen the app
    /// I just closed" into a 30-second error.
    ///
    /// Timings are 200× compressed so the test costs milliseconds: a 300 ms stale
    /// window, a lock aged 150 ms (half-stale, i.e. not yet stealable), and a 2-second
    /// wait budget. Pre-fix this returned `Held` on the first check. Post-fix the
    /// acquirer keeps looking, the lock crosses 300 ms, and the steal succeeds.
    #[test]
    fn a_waiting_acquirer_takes_the_lock_the_moment_it_goes_stale() {
        let dir = tempfile::tempdir().expect("tmp");
        let session = dir.path().join("s.jsonl");
        std::fs::write(&session, "").expect("seed");
        let lock = session.with_extension("lock");
        std::fs::write(&lock, "pid=999999\n").expect("dead holder left this");
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(&lock)
            .expect("open");
        f.set_modified(SystemTime::now() - Duration::from_millis(150))
            .expect("age it to half the window");

        let started = std::time::Instant::now();
        let outcome = SessionLease::acquire_with(
            &session,
            Duration::from_millis(300),
            Duration::from_millis(50),
            Duration::from_secs(2),
        )
        .expect("acquire");
        assert!(
            matches!(outcome, LeaseOutcome::Acquired(_)),
            "a dead holder must not make the next launch fail: {outcome:?}"
        );
        // M136 asserted the opposite of this: that the acquirer waits the FULL stale
        // window (>= 140ms of a 300ms window) before stealing. That was the right
        // rule then, and it is the 30-second wait the 2026-09-28 audit measured as the
        // real user-facing bug. The dead holder's lock is already 150ms old and not
        // moving, so it is now taken as soon as one heartbeat (50ms here) has been
        // watched — which is what this assertion now pins.
        //
        // The property M136 cared about is preserved in the test immediately below,
        // which proves a LIVE holder is still never taken from. Shrinking this bound
        // without that companion test would be stealing from the living.
        assert!(
            started.elapsed() >= Duration::from_millis(45),
            "a dead holder's unmoved lock must still be watched for a heartbeat \
             before it is taken ({:?})",
            started.elapsed()
        );
    }

    /// The other half, and the one that must not regress: a holder that keeps
    /// heartbeating is still refused after the full wait. A test that only proved
    /// the dead-holder case would pass just as happily against a lease that steals
    /// instantly — which is exactly the second-writer corruption this exists to stop.
    #[test]
    fn a_waiting_acquirer_still_refuses_a_heartbeating_holder() {
        let dir = tempfile::tempdir().expect("tmp");
        let session = dir.path().join("s.jsonl");
        std::fs::write(&session, "").expect("seed");
        let stale = Duration::from_millis(200);
        let beat = Duration::from_millis(30);
        let LeaseOutcome::Acquired(_held) =
            SessionLease::acquire_with(&session, stale, beat, Duration::ZERO).expect("acquire")
        else {
            panic!("first acquisition must succeed");
        };

        // Wait several stale windows. The holder is alive and re-stamping throughout,
        // so the lock never ages out and the acquirer must end up refusing.
        let outcome = SessionLease::acquire_with(&session, stale, beat, Duration::from_millis(500))
            .expect("acquire");
        match outcome {
            LeaseOutcome::Held { holder, waited } => {
                let holder = holder.expect("holder text");
                assert!(
                    holder.contains(&format!("pid={}", std::process::id())),
                    "and it still names the live holder: {holder}"
                );
                assert!(
                    waited >= Duration::from_millis(500),
                    "having actually waited, not refused early: {waited:?}"
                );
            }
            other => panic!(
                "a live holder must survive a full wait, got {other:?} — this is the \
                 second-writer corruption the lease exists to prevent"
            ),
        }
    }

    /// The case the milestone's own `Limits` section used to admit nothing covered:
    /// a holder that exits *while* someone is waiting on it. The wait re-reads the
    /// lock each poll specifically to notice this, and the holder's `Drop` removes
    /// the file — so the acquirer should take the lock well before its own deadline
    /// rather than sitting out the remainder.
    #[test]
    fn a_holder_that_exits_mid_wait_is_noticed_at_once() {
        let dir = tempfile::tempdir().expect("tmp");
        let session = dir.path().join("s.jsonl");
        std::fs::write(&session, "").expect("seed");
        let stale = Duration::from_secs(30);
        let beat = Duration::from_millis(20);
        let lock = session.with_extension("lock");
        let holder = match SessionLease::acquire_with(&session, stale, beat, Duration::ZERO)
            .expect("acquire")
        {
            LeaseOutcome::Acquired(held) => held,
            other => panic!("expected Acquired, got {other:?}"),
        };
        assert!(lock.exists());

        // The holder quits 150 ms in — well before the 3-second wait is up.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(holder);
        });

        let started = std::time::Instant::now();
        let outcome = SessionLease::acquire_with(
            &session,
            stale,
            Duration::from_millis(20),
            Duration::from_secs(3),
        )
        .expect("acquire");
        assert!(
            matches!(outcome, LeaseOutcome::Acquired(_)),
            "a holder that released the lock must be taken over from, not waited out: {outcome:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "and taken promptly, not at the deadline ({:?})",
            started.elapsed()
        );
    }

    /// The progress callback fires while waiting, with a *decreasing* remaining time.
    ///
    /// M138. M136 made the wait real and silent, so this is the assertion that the user
    /// is no longer staring at a blank screen: the callback is what the CLI draws, and
    /// if it never fires the feature does not exist.
    #[test]
    fn the_wait_reports_progress_with_a_shrinking_countdown() {
        use std::sync::{Arc, Mutex};
        let dir = tempfile::tempdir().expect("tmp");
        let session = dir.path().join("wait.jsonl");
        std::fs::write(&session, b"").expect("seed");

        // A dead holder's lock: fresh mtime, a pid that does not exist. The wait is for
        // a lock that will *become* stale, which is exactly the M136 restart case.
        std::fs::write(session.with_extension("lock"), b"pid=999999\n").expect("lock");
        let stale = Duration::from_millis(300);
        // A heartbeat LONGER than the stale window, so this wait is bounded by
        // staleness and lasts the full 300ms. With the default 50ms heartbeat the
        // dead holder's lock is released after one heartbeat, which leaves too few
        // polls to say anything about a countdown. The heartbeat is what the
        // dead-holder rule watches; staleness is the backstop, and this test is
        // about the backstop.
        let beat = Duration::from_secs(10);

        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let outcome = SessionLease::acquire_with_progress(
            &session,
            stale,
            beat,
            stale,
            move |waited, remaining| sink.lock().expect("lock").push((waited, remaining)),
        )
        .expect("acquire");

        assert!(
            matches!(outcome, LeaseOutcome::Acquired(_)),
            "a dead holder's lock must be taken once it goes stale"
        );
        let polls = seen.lock().expect("lock").clone();
        // >= 2, not more: the dead holder's unmoved lock is now released after one
        // heartbeat (50ms here) rather than a full stale window (300ms), so there
        // are fewer polls to report. What this test protects is that the wait is
        // *visible* and the countdown *shrinks* — a shorter wait needs fewer
        // reports, not none. The "never reports at all" case is the sibling test
        // below, which covers a free lock.
        assert!(
            polls.len() >= 2,
            "expected several progress reports across a 300ms wait, got {}",
            polls.len()
        );
        // A countdown that goes *up* is a lie, and would be the natural bug here.
        for pair in polls.windows(2) {
            assert!(
                pair[1].1 <= pair[0].1,
                "remaining time must not increase: {:?} then {:?}",
                pair[0],
                pair[1]
            );
        }
        assert!(
            polls.iter().all(|(w, r)| *r <= stale && *w <= stale),
            "every report must stay inside the declared budget"
        );
    }

    /// The other half, and the one that protects normal startup: a free lock must not
    /// make the CLI print a waiting notice. Without this, a "helpful" progress line
    /// would appear on every single launch.
    #[test]
    fn an_uncontended_acquire_never_reports_a_wait() {
        let dir = tempfile::tempdir().expect("tmp");
        let session = dir.path().join("free.jsonl");
        // `Cell`, not a plain `mut` binding: the callback is `Fn` by design — a
        // progress reporter has no business needing mutable capture — so the counter
        // is interior mutability. That constraint is itself part of the contract.
        let calls = std::cell::Cell::new(0);
        let outcome = SessionLease::acquire_with_progress(
            &session,
            Duration::from_secs(30),
            Duration::from_secs(10),
            Duration::from_secs(30),
            |_, _| calls.set(calls.get() + 1),
        )
        .expect("acquire");
        assert!(matches!(outcome, LeaseOutcome::Acquired(_)));
        assert_eq!(
            calls.get(),
            0,
            "a free lock must not make the CLI print a waiting notice"
        );
    }

    // ---- The dead-holder case, which must not cost the user 30 seconds ----

    /// A lock nobody is re-stamping, left by a killed TUI, is taken as soon as a
    /// full heartbeat has passed — not after the whole stale window.
    ///
    /// This is the defect measured on 2026-09-28: a real conhost at 100x30 showed
    /// `waiting for this session to be released — 4s elapsed, ~26s left` for half a
    /// minute, with the process alive underneath and accepting no input. That is the
    /// same shape as the reported typing-freeze, and a user who closes a terminal
    /// window pays it on every launch.
    ///
    /// Timings are compressed: a 400 ms heartbeat stands in for the real 10 s, and
    /// the stale window stays at 30 s so that "took it early" and "waited for
    /// staleness" are unambiguously different outcomes.
    #[test]
    fn a_dead_holders_fresh_lock_is_taken_after_one_heartbeat_not_one_stale_window() {
        let dir = tempfile::tempdir().expect("tmp");
        let session = dir.path().join("killed.jsonl");
        std::fs::write(&session, "").expect("seed");
        // Fresh mtime: this is the case that used to cost 30 seconds, because
        // `age >= stale_after` is false and the acquirer had nothing else to go on.
        std::fs::write(session.with_extension("lock"), "pid=999999\n").expect("leftover");

        let started = std::time::Instant::now();
        let outcome = SessionLease::acquire_with(
            &session,
            Duration::from_secs(30),    // the real stale window
            Duration::from_millis(400), // a compressed heartbeat
            Duration::from_secs(30),    // and a real wait budget
        )
        .expect("acquire");
        let elapsed = started.elapsed();

        assert!(
            matches!(outcome, LeaseOutcome::Acquired(_)),
            "a lock nobody is heartbeating must be taken: {outcome:?}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "took {elapsed:?} — the point is that it is nowhere near the 30s window"
        );
    }

    /// The property that makes the test above safe to ship: a holder that IS
    /// heartbeating is still never taken from, however long the acquirer waits.
    ///
    /// This is the second-writer corruption the lease exists to prevent, and it is
    /// the reason the previous design refused to probe the recorded pid. A fix that
    /// only proved the dead-holder case would pass just as happily against one that
    /// steals from the living on the second poll.
    #[test]
    fn a_heartbeating_holder_is_never_taken_even_though_its_lock_is_always_fresh() {
        let dir = tempfile::tempdir().expect("tmp");
        let session = dir.path().join("live.jsonl");
        std::fs::write(&session, "").expect("seed");
        let stale = Duration::from_secs(30);
        let beat = Duration::from_millis(60);
        let LeaseOutcome::Acquired(_held) =
            SessionLease::acquire_with(&session, stale, beat, Duration::ZERO).expect("acquire")
        else {
            panic!("first acquisition must succeed");
        };

        // The holder re-stamps throughout, so its lock's mtime is never old and
        // `age >= stale_after` never fires. The only thing that could take it is the
        // new "mtime has not moved" rule, and a moving mtime must defeat it.
        let outcome = SessionLease::acquire_with(&session, stale, beat, Duration::from_millis(700))
            .expect("acquire");
        assert!(
            matches!(outcome, LeaseOutcome::Held { .. }),
            "a live holder must survive a wait: {outcome:?} — this is the \
             second-writer corruption the lease exists to prevent"
        );
    }

    /// A lock that has not moved for *less* than a heartbeat is not yet evidence of
    /// anything, and must not be stolen on the strength of two quick polls.
    ///
    /// Without this, a caller that polls fast and reads the same coarse mtime twice
    /// would take a lock from a process that is alive and simply has not reached its
    /// first heartbeat yet.
    #[test]
    fn a_fresh_lock_is_not_stolen_before_a_whole_heartbeat_has_passed() {
        let dir = tempfile::tempdir().expect("tmp");
        let session = dir.path().join("young.jsonl");
        std::fs::write(&session, "").expect("seed");
        std::fs::write(session.with_extension("lock"), "pid=999999\n").expect("leftover");

        // Heartbeat far longer than the wait, so the settle window can never elapse
        // and the only possible answer is `Held`.
        let outcome = SessionLease::acquire_with(
            &session,
            Duration::from_secs(30),
            Duration::from_secs(10),
            Duration::from_millis(150),
        )
        .expect("acquire");
        assert!(
            matches!(outcome, LeaseOutcome::Held { .. }),
            "an unmoved lock must not be stolen before a full heartbeat passes: {outcome:?}"
        );
    }
}
