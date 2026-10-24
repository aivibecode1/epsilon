//! Append-only JSONL tree session store.
//!
//! One `.jsonl` file per session; each line is a ``SessionEntry`` assigned a monotonic `seq`,
//! a parent link (branch tree), and an append timestamp. The human transcript is the file;
//! the model-visible history is a derived view where compaction entries shadow the ranges they
//! summarize (DSH's surface-shadowing principle over pi's entry format).

pub mod archive;
pub mod delete;
pub mod discover;
/// Session export rendering (HTML/JSON writers over read entries), shared by
/// every front end that offers `/export` or `/share`.
pub mod export;
/// Importing a current (v2/v3) pi session file into a new Epsilon session file, as a copy.
pub mod import;
mod lease;
// Wired into the CLI resume path (--resume/--continue) since M19's ponytail-audit
// follow-up; integration tests cover the crash scenarios.
pub mod repair;
pub mod search;
mod store;
/// One timestamp formatter, used by the RPC surface and by session file naming alike — see the
/// module docs for why it is not written twice.
pub mod time;

pub use archive::{
    ARCHIVED_DIR, archive_session_file, archived_dir, list_archived, list_archived_for_cwd,
    unarchive_session_file,
};
pub use delete::{DeleteOutcome, delete_session_file};
pub use discover::{
    SessionScope, SessionSummary, count_sessions_for_cwd, latest_session, latest_session_for_cwd,
    list_sessions, list_sessions_all, list_sessions_for_cwd, summarize,
};
pub use import::{
    ImportReport, import_output_path, import_pi_session, is_pi_v2_entry_shape,
    looks_like_pi_session,
};
pub use lease::{LeaseOutcome, SessionLease, sweep_stale_locks};
pub use search::{SearchOptions, SessionSearchHit, search_session};
pub use store::{
    CompactionDigest, FaultOp, LineDiagnosis, ModelViewEntry, SessionStore, StoreError,
    build_forest_from_entries, original_backup_path, timestamp_unix_secs,
};

/// The largest session file this build will load or import.
///
/// A session is a conversation, not a database dump: 256 MiB is already two
/// orders of magnitude above a long chat, and it bounds the allocation a
/// repo-shipped or shared `.jsonl` can demand before the store refuses it. pi
/// bounds its header scan (`MAX_SESSION_HEADER_SCAN_BYTES = 1 MiB`) for the
/// same reason; this is the whole-file version.
pub const MAX_SESSION_FILE_BYTES: u64 = 256 * 1024 * 1024;
