//! The durable session mirror: attachment, history, resume, repair.

use std::sync::{Arc, Mutex};

use epsilon_contracts::agent_event::AgentEvent;
use epsilon_contracts::message::Message;
use epsilon_contracts::session_entry::EntryKind;

use crate::{Agent, RunError, SessionStore, now_ms, store_lock_failure};

impl Agent {
    /// Seed the transcript from a previous session (M12 `--resume`).
    ///
    /// Messages are restored into runtime history WITHOUT re-persisting (they already
    /// live in the durable mirror); subsequent messages append normally. Call before
    /// the first `run()`.
    pub fn restore_history(&self, messages: Vec<Message>) {
        *self
            .observation_scope
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Arc::new(());
        if let Ok(mut h) = self.history.lock() {
            *h = messages;
        }
        // Restored history did not come from a request this process made, so any
        // anchor held for the previous transcript is meaningless here.
        self.clear_usage_anchor();
        // ...and so is any run-scoped replacement built on top of it (CORE-02).
        self.bump_history_epoch();
    }

    /// Re-derive the model view from the store's *current* leaf and restore it into
    /// runtime history. Returns the number of messages now visible.
    ///
    /// This is what makes a session-tree jump a navigation rather than an
    /// inspection. `/tree` has always moved the store's leaf
    /// (`store.navigate_to`) and printed the resulting branch — which is why the
    /// audit called it a viewer: the file changed, the conversation did not. The
    /// user was left reading a transcript that no longer described the session.
    ///
    /// Deliberately **not** [`Self::live_resume`]. That reopens the file, which
    /// means dropping and re-acquiring the cross-process write lease — a jump
    /// within one session does not need that, and doing it would briefly release
    /// the lease on a file we are the only writer for, which is precisely the
    /// window another epsilon is waiting to steal (see the lease's dead-holder
    /// rule). It also runs `repair_on_open`, which is for a file that has been
    /// closed by someone else, not for one we are mid-conversation with.
    ///
    /// The leaf was already moved by the caller, so this only reads.
    ///
    /// Fails without touching runtime history if the store is absent or locked: a
    /// half-restored transcript is worse than the stale one the user was reading,
    /// because it is neither the old state nor the new one.
    pub fn reproject_current_branch(&self) -> Result<usize, String> {
        let Some(handle) = self.store() else {
            return Err("no session store attached".into());
        };
        // Derive the view under the store's lock, then release it before touching
        // `self.history` — two different locks, and holding both at once is how a
        // future caller deadlocks this.
        let messages: Vec<Message> = {
            let st = handle.lock().map_err(|_| "store lock".to_string())?;
            st.model_view()
                .map_err(|e| format!("read the session tree: {e}"))?
                .into_iter()
                .map(|v| v.into_message())
                .collect()
        };
        let count = messages.len();
        self.restore_history(messages);
        Ok(count)
    }

    /// Live in-TUI session switch (pi parity: `/resume` + `/import` without restart).
    ///
    /// Opens `path` as a [`SessionStore`], repairs dangling tool calls (same as CLI
    /// `--resume`), derives the shadowed model view, restores it into runtime
    /// history, and re-attaches the new store. The TUI then rehydrates the visible
    /// transcript from [`Self::history`]. Ponytail: reuse existing repair + model_view.
    pub fn live_resume(&self, path: &std::path::Path) -> Result<usize, String> {
        // A session you are resuming has to **exist**. `SessionStore::open` creates what
        // is missing, so without this a mistyped path silently produces an empty session —
        // and the next thing written goes into that new file, while the reader believes
        // they are continuing the conversation they named. That is data loss dressed as
        // success, and it is the one failure mode here that keeps no trace.
        if !path.is_file() {
            return Err(format!(
                "no session at {} — `resume` continues a conversation that exists; \
                 use a new session to start one",
                path.display()
            ));
        }
        // M61: the store owns a cross-process write lease, so re-resuming the
        // session this process already holds would conflict with ourselves.
        // Release first — and keep the old handle so a failed open (locked by
        // ANOTHER process, or corrupt) leaves the current session attached.
        let previous = self.store();
        let already_holding = previous
            .as_ref()
            .is_some_and(|h| h.lock().map(|s| s.path() == path).unwrap_or(false));
        // When we already hold this file the old store must be gone before the
        // reopen — including the clone kept for the rollback path, which would
        // otherwise keep the Arc (and its lease) alive through the open.
        let restore_target = if already_holding {
            self.detach_session_store();
            // The rollback clone would keep the store — and its lease — alive
            // through the reopen, so it must go too.
            drop(previous);
            None
        } else {
            previous
        };
        let opened = SessionStore::open(path);
        let store = match opened {
            Ok(store) => store,
            Err(e) => {
                if let Some(previous) = restore_target {
                    self.attach_session_store(previous);
                }
                return Err(format!("open {}: {e}", path.display()));
            }
        };
        // M169: the store is attached BEFORE anything else can fail. Above, the old one was
        // detached and dropped (to release its lease) when this is the file we already hold,
        // so an early return here used to leave the agent with no store at all - and every
        // TUI command that reads the session would then panic rather than describe the state.
        // Attaching first means a failed repair still costs the user their command, not the
        // whole session store.
        let handle = Arc::new(Mutex::new(store));
        self.attach_session_store(Arc::clone(&handle));
        // The attach above swaps in the new handle, dropping the previous store and with it
        // the previous lease. We keep our own clone so we can lock while repairing.
        let repaired = {
            let mut st = handle.lock().map_err(|_| "store lock".to_string())?;
            epsilon_session::repair::repair_on_open(&mut st)
        };
        match repaired {
            Ok(report) => {
                if !report.kinds().is_empty() {
                    // The kinds, not just a count: "a call had no result" and "the session ended
                    // mid-run" are different things to have lost (M75).
                    for line in report.summary() {
                        eprintln!("repaired {}: {line}", path.display());
                    }
                    if let Ok(mut slot) = self.last_repair.lock() {
                        *slot = Some(report);
                    }
                }
            }
            Err(e) => return Err(format!("repair {}: {e}", path.display())),
        }
        let msgs: Vec<Message> = {
            let st = handle.lock().map_err(|_| "store lock".to_string())?;
            st.model_view()
                .map_err(|e| format!("read {}: {e}", path.display()))?
                .into_iter()
                .map(|v| v.into_message())
                .collect()
        };
        let count = msgs.len();
        self.restore_history(msgs);
        Ok(count)
    }

    /// Name a recorded-vs-running directory mismatch, if there is one.
    ///
    /// Resuming (or switching into) a session recorded in another directory
    /// silently reinterprets every relative path against the running one —
    /// and a recorded directory that no longer exists makes it certain. The
    /// launch stays where it is (moving the process is not on offer); this
    /// names the fact so the reader, not the tooling, decides what it means.
    /// Empty recording (headerless v1 files) stays silent: no information,
    /// no noise.
    pub fn cwd_mismatch_note(recorded: &str, current: &std::path::Path) -> Option<String> {
        let recorded = recorded.trim();
        if recorded.is_empty() {
            return None;
        }
        if Self::same_directory(std::path::Path::new(recorded), current) {
            return None;
        }
        if !std::path::Path::new(recorded).is_dir() {
            return Some(format!(
                "session was recorded in {recorded}, which no longer exists — tools run in {}",
                current.display()
            ));
        }
        Some(format!(
            "session was recorded in {recorded} — tools run in {}",
            current.display()
        ))
    }

    /// Read `path`'s recorded cwd and name a mismatch against `current`.
    ///
    /// The one call both front ends make after a resume lands: headerless or
    /// unreadable headers stay silent (no information, no noise), everything
    /// else goes through [`Self::cwd_mismatch_note`].
    pub fn resume_cwd_note(path: &std::path::Path, current: &std::path::Path) -> Option<String> {
        let recorded = epsilon_session::discover::read_session_header(path)?.cwd;
        Self::cwd_mismatch_note(&recorded, current)
    }

    /// Whether two paths name the same directory.
    ///
    /// Canonicalized comparison when both exist (which settles separators,
    /// case, and symlinks on every platform); a normalized string fallback
    /// when they do not, so a gone directory still compares sanely.
    fn same_directory(a: &std::path::Path, b: &std::path::Path) -> bool {
        if let (Ok(ca), Ok(cb)) = (a.canonicalize(), b.canonicalize()) {
            return ca == cb;
        }
        fn norm(p: &std::path::Path) -> String {
            let mut s = p.to_string_lossy().into_owned();
            while s.len() > 1 && (s.ends_with('/') || s.ends_with('\\')) {
                s.pop();
            }
            #[cfg(windows)]
            {
                s.to_ascii_lowercase()
            }
            #[cfg(not(windows))]
            {
                s
            }
        }
        norm(a) == norm(b)
    }

    /// Attach a durable session mirror. Once attached:
    /// - every transcript message is appended to the JSONL tree,
    /// - compaction entries are persisted,
    /// - the provider context comes from the store's SHADOWED model view, so summaries
    ///   shrink what the model sees while the human log stays complete.
    pub fn attach_session_store(&self, store: Arc<Mutex<SessionStore>>) {
        if let Ok(mut slot) = self.session_store.lock() {
            *slot = Some(store);
        }
        // A new store is a different file, so a failure recorded against the old one no longer
        // describes what requests would be built from (F07). The epoch bump below is the same
        // idea for run-scoped replacements.
        if let Ok(mut slot) = self.durable_write_failure.lock() {
            *slot = None;
        }
        // The provider view's source just changed (CORE-02): a run-scoped replacement
        // built against the previous source must not outlive it, or the rest of the run
        // would keep reading a pre-swap view — the same class of staleness a committed
        // compaction creates, handled by the epoch check before each request.
        self.bump_history_epoch();
    }

    /// Drop the attached durable mirror, releasing its write lease (M61). The
    /// agent keeps running — a caller that releases the store has decided this
    /// transcript is no longer written by this process.
    pub fn detach_session_store(&self) {
        if let Ok(mut slot) = self.session_store.lock() {
            *slot = None;
        }
        // Same reason as `attach_session_store`: the request path's source changes here.
        self.bump_history_epoch();
    }

    pub(crate) fn store(&self) -> Option<Arc<Mutex<SessionStore>>> {
        self.session_store.lock().ok().and_then(|s| s.clone())
    }

    /// Test/inspection handle to the attached durable mirror.
    #[doc(hidden)]
    pub fn store_handle_for_tests(&self) -> Arc<Mutex<SessionStore>> {
        self.store().expect("no session store attached")
    }

    /// The attached durable mirror, when there is one.
    ///
    /// The non-panicking counterpart of [`Agent::store_handle_for_tests`], for frontends: an
    /// agent with no session file (a plain `--tui` run, a `task` child before its store exists)
    /// is a normal state, and a command like `/session` has to describe it rather than crash.
    pub fn session_store(&self) -> Option<Arc<Mutex<SessionStore>>> {
        self.store()
    }

    /// Path of the attached session file, when one is attached (M57: the TUI's
    /// session browser marks the current file — it must not panic on an agent
    /// built without a durable mirror).
    pub fn session_path(&self) -> Option<std::path::PathBuf> {
        let store = self.store()?;
        store.lock().ok().map(|st| st.path().to_path_buf())
    }

    /// The attached session's own id, from its header (`SessionStore::header`).
    ///
    /// `None` without a durable mirror, and `None` for a store whose header could not be
    /// read — the same "absent rather than fiction" rule the rest of this snapshot keeps.
    /// A client identifies a conversation by this, not by [`Agent::session_path`]: a fork
    /// moves the conversation to a new file, and a rename moves the file under the
    /// conversation.
    pub fn session_id(&self) -> Option<String> {
        let store = self.store()?;
        let guard = store.lock().ok()?;
        guard.header().map(|h| h.id.clone())
    }

    /// The session's display name, as `/name` or RPC `set_session_name` last set it.
    ///
    /// Read from the newest `Custom { "session_name" }` entry, because the name can be
    /// written anywhere in the file and the newest one wins — the same rule pi's session
    /// listing uses. `None` when no name was ever set, which is not an error.
    pub fn session_name(&self) -> Option<String> {
        let store = self.store()?;
        let guard = store.lock().ok()?;
        guard.session_name()
    }

    /// Build the live state snapshot both wire surfaces serve.
    ///
    /// **Why this is a method and not a free function in each frontend.** `rpc_mode.rs`
    /// and `epsilon-server` each grew their own `SessionState { … }` literal, and they
    /// were identical — until a field was added and only one of them learned it. That is
    /// D29's mechanism applied to a *shared* struct: the payload exists in one place
    /// precisely so the two transports cannot disagree, and two builders put that back
    /// on a knife edge. One constructor, two callers, no third spelling.
    ///
    /// It lives here rather than in `epsilon-contracts` because it needs an `Agent`, and
    /// contracts may not depend on core — so this is the only crate that can hold it.
    /// The alternative, a `SessionState::from(&Agent)` in contracts, is forbidden by the
    /// architecture law and would be the `cli -> server` edge in disguise.
    pub fn session_state(&self) -> epsilon_contracts::SessionState {
        epsilon_contracts::SessionState {
            model: self.model_id(),
            thinking_level: self.thinking_level(),
            is_streaming: self.is_running(),
            steering_mode: self.steering_mode(),
            follow_up_mode: self.follow_up_mode(),
            message_count: self.history().len(),
            pending_message_count: self.steer_queue_len(),
            session_file: self.session_path().map(|p| p.display().to_string()),
            session_id: self.session_id(),
            session_name: self.session_name(),
            estimated_context_tokens: self.estimated_context_tokens(),
            // Additive to pi's payload (M206): a goal is this harness's own concept, and a client
            // that renders one needs the phase *and* the activation, because an `active` goal in a
            // fresh process is disarmed and will not continue by itself.
            goal: self
                .goal()
                .map(|view| epsilon_contracts::session_state::SessionGoal {
                    id: view.goal.id,
                    revision: view.goal.revision,
                    objective: view.goal.objective,
                    status: view.goal.status,
                    rounds_started: view.goal.rounds_started,
                    max_rounds: view.goal.max_rounds,
                    armed: view.armed,
                    blocker: view.goal.blocker,
                }),
        }
    }

    /// Search the attached session log (pi `search/scanning.ts` parity).
    ///
    /// Case-insensitive substring scan over user/assistant/tool-result text, newest
    /// hits first; `None` when no durable mirror is attached. Locking note: the
    /// store mutex is held only for the scan duration and the scan never writes,
    /// so a concurrent run can proceed on other locks.
    pub fn search_history(
        &self,
        query: &str,
        opts: &epsilon_session::SearchOptions,
    ) -> Option<Result<Vec<epsilon_session::SessionSearchHit>, epsilon_session::StoreError>> {
        let store = self.store()?;
        let guard = store.lock().ok()?;
        Some(epsilon_session::search_session(&guard, query, opts))
    }

    /// Append a message to the durable mirror when one is attached.
    ///
    /// Fallible, and deliberately loud (F07). The provider request reads the store's
    /// *projection*, so a durable append that fails while runtime history keeps the message
    /// leaves the two views describing different conversations: the assistant believes the turn
    /// was recorded and the next request is built from a transcript that does not contain it.
    /// The old code discarded the result (`let _ = st.append(..)`), which is exactly the silent
    /// divergence F07 names.
    ///
    /// On failure this emits an [`AgentEvent::Notice`] — every frontend renders notices, and the
    /// `Err` alone would reach only the callers that look at it (the TUI discards `run()`'s
    /// result, so an error nobody prints is not a report) — and returns [`RunError::Internal`]
    /// for the caller to propagate. Callers that can propagate stop the run; see `run()`'s
    /// prompt append and `run_inner`'s call sites.
    fn persist(&self, m: &Message) -> Result<(), RunError> {
        let Some(store) = self.store() else {
            return Ok(()); // no durable mirror attached: nothing can diverge
        };
        let mut st = store.lock().map_err(|_| store_lock_failure())?;
        st.append(EntryKind::Message {
            message: m.clone(),
            api_request_sha256: None,
        })
        .map_err(|e| self.durable_write_failed(format!("append: {e}")))?;
        Ok(())
    }

    /// Record a failed durable write: announce it, remember it, and return the run error.
    ///
    /// The notice is what makes the failure visible — the TUI prints notices and throws away the
    /// `Err`, so a silent `Err` would be the same invisible failure in a different shape. The
    /// sentence names the consequence the user needs ("the session may be missing a turn") and
    /// what to do next, rather than only the operation that failed.
    ///
    /// The latch closes the second half of F07. Stopping the run is only possible on the paths
    /// that return a `Result`, and four of them cannot (the abort/EOF salvage arms return the
    /// message, not an error). A later request on this agent must not be built from a store view
    /// that is now missing a message the loop believes it recorded, so [`Agent::try_context_view`]
    /// asks this latch first and fails closed. The next provider request therefore never runs
    /// against a transcript that disagrees with the loop's own record.
    pub(crate) fn durable_write_failed(&self, what: String) -> RunError {
        let detail = format!(
            "the session file could not record this turn ({what}). The turn is in this \
             session's memory but not on disk: a resume, or the next request, may be missing it. \
             Check the disk and the session file, then retry the turn."
        );
        self.emit(AgentEvent::Notice {
            text: detail.clone(),
        });
        if let Ok(mut slot) = self.durable_write_failure.lock() {
            *slot = Some(what);
        }
        RunError::Internal(detail)
    }

    /// Remove the most recently pushed message (overflow-retry discards failed attempts).
    ///
    /// Both views must lose the message together for the same reason [`Agent::persist`] exists:
    /// the store's projection is what the next request is built from, so a pop that fails on the
    /// store while runtime history drops the message leaves the store *ahead* — still carrying
    /// the attempt the loop just discarded as oversized, which the retry would send again.
    pub(crate) fn pop_last_message(&self) -> Result<(), RunError> {
        if let Ok(mut h) = self.history.lock() {
            h.pop();
        }
        if let Ok(store) = self.session_store.lock()
            && let Some(st) = store.as_ref()
        {
            let mut s = st.lock().map_err(|_| store_lock_failure())?;
            s.pop_last()
                .map_err(|e| self.durable_write_failed(format!("pop last: {e}")))?;
        }
        Ok(())
    }

    /// Current transcript snapshot (all roles, including harness-only messages).
    ///
    /// Returns an owned `Vec` because most callers want to keep or filter it, and
    /// a deep clone of a long session is not free. For callers that only need to
    /// *look*, prefer [`Self::with_history`]: this is a full clone of every
    /// message, tool arguments and all, and one such call per drawn frame is
    /// enough to make an input-latency problem out of a rendering problem.
    pub fn history(&self) -> Vec<Message> {
        self.history.lock().map(|h| h.clone()).unwrap_or_default()
    }

    /// Run `f` over a borrowed view of the transcript, without cloning it.
    ///
    /// This exists for the per-frame callers — the TUI's context mirror, and any
    /// other read that happens on a redraw. Holding the lock for the duration of
    /// `f` is what makes the borrow sound; `f` therefore cannot itself call back
    /// into a method that takes the same lock, which is the one real constraint
    /// and is a good one, because that would be a deadlock rather than a slow
    /// path.
    pub fn with_history<T>(&self, f: impl FnOnce(&[Message]) -> T) -> T {
        match self.history.lock() {
            Ok(h) => f(&h),
            // A poisoned lock still holds readable data; `history()` degrades to
            // empty in the same situation, and an empty transcript would make the
            // context gauge read `0/164k`, which is a lie of the same kind this
            // function exists to prevent.
            Err(poisoned) => f(&poisoned.into_inner()),
        }
    }

    pub(crate) fn push_message(&self, m: &Message) -> Result<(), RunError> {
        if let Ok(mut h) = self.history.lock() {
            h.push(m.clone());
        }
        self.persist(m)
    }

    /// What the session resume had to repair, if anything (M75): the *kinds*, so a frontend can
    /// say what was missing rather than printing a bare count.
    pub fn repair_report(&self) -> Option<epsilon_session::repair::RepairReport> {
        self.last_repair.lock().ok().and_then(|r| r.clone())
    }

    /// Remember that a tool call acted on `path`, so its directory's instructions can be
    /// loaded (M73). Cheap by construction: it records a directory and reads nothing — the
    /// prompt asks [`crate::prompt::load_subdirectory_hints`] at assembly time.
    pub fn note_touched(&self, path: &std::path::Path) {
        let Some(dir) = path.parent() else { return };
        if !dir.starts_with(&self.workspace_root) || dir == self.workspace_root {
            return; // outside the workspace, or the root — already loaded as project context
        }
        if let Ok(mut set) = self.touched_dirs.lock() {
            set.insert(dir.to_path_buf());
        }
    }

    /// Directories touched so far, sorted: the hint loader must not see a walk order.
    pub(crate) fn touched_dirs(&self) -> Vec<std::path::PathBuf> {
        let mut v: Vec<_> = self
            .touched_dirs
            .lock()
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        v.sort();
        v
    }

    /// Give every unanswered tool call a result that says what happened.
    ///
    /// A call with no result is not a cosmetic problem: providers validate tool-call pairing, so a
    /// transcript that carries one is rejected outright, and the failure arrives on the *next*
    /// request, far from its cause. The synthesized result is an error and says it was never
    /// executed — a pretend success would be worse than the invalid transcript.
    ///
    /// Returns how many calls were closed.
    pub(crate) fn close_dangling_tool_calls(&self) -> Result<usize, RunError> {
        let mut pending: Vec<(String, String)> = Vec::new();
        for m in self.history() {
            match m {
                Message::Assistant(a) => {
                    for (id, name, _) in a.tool_calls() {
                        pending.push((id.to_string(), name.to_string()));
                    }
                }
                Message::ToolResult(t) => pending.retain(|(id, _)| id != &t.tool_call_id),
                _ => {}
            }
        }
        if pending.is_empty() {
            return Ok(0);
        }
        let closed = pending.len();
        for (id, name) in pending {
            let m = Message::ToolResult(epsilon_contracts::message::ToolResultMessage {
                id: epsilon_contracts::new_id("msg"),
                timestamp_ms: now_ms(),
                tool_call_id: id,
                tool_name: name,
                content: vec![epsilon_contracts::content::ContentBlock::text(
                    "[harness] this call was never executed: the previous run ended before its \
                     result arrived. Re-issue it if the work is still needed.",
                )],
                details: serde_json::json!({"interrupted": true}),
                usage: None,
                is_error: true,
            });
            self.push_message(&m)?;
        }
        Ok(closed)
    }
}
