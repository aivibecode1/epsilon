//! The turn loop: one run, from prompt to outcome.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use epsilon_contracts::agent_event::{AgentEvent, TurnEndReason};
use epsilon_contracts::content::UserContent;
use epsilon_contracts::message::{AssistantMessage, COMPACTION_MARKER, Message, StopReason, Usage};
use epsilon_contracts::session_entry::EntryKind;
use epsilon_contracts::stream::{StreamEvent, StreamFolder};

use crate::exec;
use crate::prompt;

#[cfg(feature = "real")]
use crate::retry_notice_text;

use crate::{
    Agent, CompactionOutcome, RunError, RunGuard, RunOutcome, clamp_tail_tokens, contain_hook,
    estimate_one, looks_like_context_overflow, now_ms, store_lock_failure, today_string,
};

impl Agent {
    /// Whether a run is currently active (for in-TUI gate on live resume).
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// The real provider stream, wrapped in ADR-010's retry policy (M60).
    ///
    /// The policy existed — classification, backoff, a tested executor — but
    /// nothing ever wrapped a production stream with it, so a transient 429/503
    /// or a stalled socket failed the turn outright. Scheduled waits surface as
    /// [`AgentEvent::Notice`] so the pause is explained instead of looking hung.
    #[cfg(feature = "real")]
    pub fn retrying_real_stream(&self) -> Arc<dyn epsilon_llm::StreamFn> {
        let tx = self.events_tx.clone();
        Arc::new(
            epsilon_llm::Retrying::new(Arc::new(epsilon_llm::PanicGuard(
                epsilon_llm::DirectProvider::new(),
            )))
            .with_max_attempts(epsilon_llm::RETRY_MAX_ATTEMPTS)
            .with_on_retry(move |n| {
                let _ = tx.send(Arc::new(AgentEvent::Notice {
                    text: retry_notice_text(n),
                }));
            }),
        )
    }

    /// The run's context view, shared by the turn hooks and the request builder
    /// (CORE-02): the durable mirror's shadowed model view when one is attached,
    /// else runtime history. A `prepare_next_turn` hook sees exactly the messages
    /// the next provider request would start from.
    ///
    /// Fallible on purpose, and the ONE read both callers share: a poisoned store lock or a
    /// failed `model_view()` is `RunError::Internal` for the request path, and for the hook
    /// seam it means "no view this turn" — a hook must not be handed a view the request
    /// would refuse. Two implementations of this read (one lenient, one strict) is exactly
    /// how the two could ever disagree.
    ///
    /// A failed durable write poisons this read for the rest of the attached store's life
    /// (F07). The store's projection is what requests are built from, so once a write it should
    /// have recorded has failed, the projection is missing a message runtime history holds —
    /// and serving it would hand the provider a transcript that silently disagrees with the
    /// loop's own record. Refusing is the honest answer: the caller reports a run error, and
    /// the user has already been told by the notice the failure emitted.
    ///
    /// Read-only by construction: the store lock is held for the read and nothing
    /// here can write it (ADR-007 — hooks never touch the write path).
    fn try_context_view(&self) -> Result<Vec<Message>, RunError> {
        if let Ok(slot) = self.durable_write_failure.lock()
            && let Some(what) = slot.as_deref()
        {
            return Err(RunError::Internal(format!(
                "the session file is missing a turn this session recorded ({what}), so no \
                 request can be built from it. The transcript in memory is intact; resume \
                 this session after checking the disk to see what was lost."
            )));
        }
        match self.store() {
            Some(store) => {
                let st = store
                    .lock()
                    .map_err(|_| RunError::Internal("store poisoned".into()))?;
                Ok(st
                    .model_view()
                    .map_err(|e| RunError::Internal(format!("session view: {e}")))?
                    .into_iter()
                    .map(|v| v.into_message())
                    .collect())
            }
            None => Ok(self.history()),
        }
    }

    /// pi's `currentContext.messages.push` for Epsilon's run-scoped replacement context
    /// (CORE-02, `agent-loop.ts:186,219`): every message the loop adds to runtime history
    /// during a run — drained steering and nudges, the turn's assistant message, its tool
    /// results — lands in the replacement too, so the provider sees the run's own output
    /// instead of the frozen install-time snapshot. No-op while no replacement is active.
    fn append_to_turn_context(turn_context: &mut Option<Vec<Message>>, m: &Message) {
        if let Some(ctx) = turn_context.as_mut() {
            ctx.push(m.clone());
        }
    }

    /// Run one prompt to completion.
    ///
    /// Returns `AlreadyRunning` rather than queueing — concurrent runs on one agent are a
    /// caller bug, not a workload to silently serialize.
    pub async fn run(self: &Arc<Self>, prompt: Message) -> Result<RunOutcome, RunError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(RunError::AlreadyRunning);
        }
        // F07: the early failure paths below (a dangling-call repair that cannot be recorded, a
        // prompt that cannot be appended) return before the old cleanup at the end of this
        // function, which would have left `running` set and every later run answering
        // `AlreadyRunning` — a durable-write failure would have looked like a wedged agent. The
        // guard runs the same cleanup on every exit, including those.
        let _cleanup = RunGuard { agent: self };

        // Fresh run-scoped token (M12): tools observe it via ToolContext; abort ends
        // the run cooperatively instead of only via process exit.
        let token = tokio_util::sync::CancellationToken::new();
        if let Ok(mut slot) = self.run_token.lock() {
            *slot = Some(token.clone());
        }

        if let Ok(mut s) = self.doom_streaks.lock() {
            s.clear();
        }
        if let Ok(mut g) = self.progress.lock() {
            *g = crate::guards::ProgressGuard::new();
        }

        // An interrupted run leaves calls without results. Session repair handles that for a
        // *reopened* file (`epsilon_session::repair::repair_on_open`); this is the same shape in a
        // live process, where the next request would otherwise carry a tool call nothing answered
        // — structurally invalid for strict wires, and a 400 rather than a puzzle to solve later.
        //
        // `?` since F07: closing a dangling call is a durable append, and a run must not start on
        // a store that just failed to record one. The guard below still has to undo `running`.
        let closed = self.close_dangling_tool_calls()?;
        if closed > 0 {
            self.emit(AgentEvent::Notice {
                text: format!("closed {closed} tool call(s) left unanswered by the previous run"),
            });
        }
        // Overflow recovery (ADR-007 trigger 2) is once PER RUN, not per process:
        // reset the latch here or the first recovered overflow disables recovery for
        // every later run in a long-lived TUI/server session.
        self.overflow_retried.store(false, Ordering::SeqCst);
        self.compact_suppressed.store(false, Ordering::SeqCst);
        // F07: the prompt's own append is the first durable write of the run, and a turn whose
        // prompt never reached the file must not start — the store's projection would be missing
        // the very message the run is answering.
        self.push_message(&prompt)?;
        self.emit(AgentEvent::AgentStart);

        let outcome = self.run_inner(prompt).await;
        self.emit(AgentEvent::AgentEnd {
            messages: outcome
                .as_ref()
                .map(|o| o.produced.clone())
                .unwrap_or_default(),
        });
        outcome
    }

    /// Cancel the active run's token: in-flight tools see cancellation and the loop
    /// stops at the next checkpoint with `TurnEndReason::Aborted`.
    ///
    /// Returns false when no run is active. Idempotent.
    pub fn abort_current_run(&self) -> bool {
        let cancelled = if let Ok(slot) = self.run_token.lock()
            && let Some(token) = slot.as_ref()
        {
            token.cancel();
            true
        } else {
            // No run in flight: arm the flag so a run that is about to start
            // (a just-spawned child, or a submit racing the abort) still stops.
            self.abort_armed
                .store(true, std::sync::atomic::Ordering::SeqCst);
            false
        };
        // M54: a delegated child is its own Agent with its own run token, so
        // aborting the parent must reach the children too — otherwise a
        // background subagent keeps spending tokens after the user pressed Esc.
        self.abort_delegated_tasks();
        cancelled
    }

    /// Ask every running delegated child to stop; returns how many were asked.
    ///
    /// Cascaded from [`Agent::abort_current_run`]; callable directly by a
    /// frontend that wants to stop children without a parent run in flight.
    /// Finished tasks are left alone (their output is the point).
    pub fn abort_delegated_tasks(&self) -> usize {
        let children: Vec<Arc<Agent>> = self
            .tasks
            .lock()
            .map(|v| {
                v.iter()
                    .filter(|e| e.state == epsilon_contracts::tool::TaskState::Running)
                    .map(|e| Arc::clone(&e.child))
                    .collect()
            })
            .unwrap_or_default();
        let mut asked = 0;
        for child in children {
            // Cancels a live token or arms the child's pre-start flag — either
            // way the child was told to stop.
            child.abort_current_run();
            asked += 1;
        }
        asked
    }

    /// Clone of the active run's token for `ToolContext` distribution.
    pub(crate) fn current_run_token(&self) -> Option<tokio_util::sync::CancellationToken> {
        self.run_token
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned())
    }

    async fn run_inner(self: &Arc<Self>, _prompt: Message) -> Result<RunOutcome, RunError> {
        // M54: an abort that arrived before this run started still ends it —
        // consume the flag once, so it can never leak into a later run.
        if self
            .abort_armed
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Ok(RunOutcome {
                produced: Vec::new(),
                reason: TurnEndReason::Aborted,
                usage: Usage::default(),
            });
        }
        let mut produced: Vec<Message> = Vec::new();
        let mut usage_total = Usage::default();
        let mut reason = TurnEndReason::Completed;
        let mut turn_index: usize = 0;
        // Turns that came back empty or degenerate, for the bounded-retry guard below.
        let mut empty_turns: usize = 0;
        // Verification evidence for THIS run (port plan item 5): edits and the checks that ran
        // after them. Run-scoped: a fresh run is a fresh question.
        let mut evidence = crate::verify::VerificationLog::new();
        let mut verification_nudges = 0usize;

        // CORE-02 run-scoped hook state, mirroring pi's mutable `currentContext` /
        // `config` locals (`agent-loop.ts:163-164`): a `prepareNextTurn` snapshot applies
        // to the REST of this run only — the session's model, thinking level and
        // transcript are never rewritten by a hook.
        //
        // `turn_context` is pi's `currentContext.messages`: while it is active every message
        // the loop adds to the transcript is appended to it too, so it tracks the run instead
        // of freezing at its install-time snapshot. `turn_context_epoch` is the history epoch
        // the replacement was built from — a committed compaction makes it stale (blocker 2).
        let mut turn_context: Option<Vec<Message>> = None;
        let mut turn_context_epoch: u64 = self.history_epoch();
        let mut run_model: Option<String> = None;
        let mut run_thinking: Option<epsilon_contracts::ThinkingLevel> = None;

        // Prefilled steering covers "user typed while we were starting".
        let mut pending = self.drain_steering();

        'outer: loop {
            let mut has_more_tools = true;

            while has_more_tools || !pending.is_empty() {
                // Abort checkpoint (M12): a cancelled token ends the run cooperatively.
                if let Some(token) = self.current_run_token()
                    && token.is_cancelled()
                {
                    reason = reason.merge_sticky(TurnEndReason::Aborted);
                    // Plan 002: drained-but-not-yet-injected messages go back to the
                    // front of their queue so an abort never silently eats user input.
                    self.requeue_pending(&mut pending);
                    break 'outer;
                }

                if turn_index >= self.limits.max_turns {
                    reason = reason.merge_sticky(TurnEndReason::BudgetExhausted);
                    self.requeue_pending(&mut pending);
                    break 'outer;
                }

                // Drain point: inject queued messages before the next model call.
                //
                // `?` since F07: a queued message the store refused is a message the model
                // would not see on the next request even though the loop believes it recorded
                // it. Stopping here is the only answer that does not send a divergent view.
                for m in pending.drain(..) {
                    self.push_message(&m)?;
                    Agent::append_to_turn_context(&mut turn_context, &m);
                    produced.push(m);
                }

                // ---- compaction pressure check (ADR-007 trigger 1) ----------------
                // One pressure-triggered attempt per run after a failure: the
                // summarizer just failed once (provider error, empty answer, append
                // failure) and every retry is a paid provider call, so the next
                // attempt belongs to the next run.
                if !self.compact_suppressed.load(Ordering::SeqCst) && self.compaction_due() {
                    match self.compact_now(&mut produced).await? {
                        CompactionOutcome::Committed => {}
                        CompactionOutcome::Failed(why) => {
                            self.compact_suppressed.store(true, Ordering::SeqCst);
                            self.emit(AgentEvent::Notice {
                                text: format!("context compaction skipped: {why}"),
                            });
                        }
                        CompactionOutcome::Cancelled => {
                            reason = reason.merge_sticky(TurnEndReason::Aborted);
                            break 'outer;
                        }
                    }
                }

                // CORE-02 (blocker 2): a replacement is only valid while the history it was
                // built from is still the run's base. A committed compaction just replaced
                // that history (and restored it from the summary), so the run-scoped
                // replacement is stale: requests built from it would stop reading the store's
                // shadowed model view (the M86/ADR-007 invariant) and an overflow retry would
                // re-send the very request the compaction fixed. Drop it and rebuild from the
                // current source; the per-run model/thinking overrides are snapshot state, not
                // transcript state, and survive (pi keeps `config.model`/`reasoning` too).
                if turn_context.is_some() && self.history_epoch() != turn_context_epoch {
                    turn_context = None;
                }

                self.emit(AgentEvent::TurnStart { turn_index });

                // ---- one provider turn --------------------------------------------
                let assistant = match self
                    .stream_one_turn(
                        &mut usage_total,
                        turn_context.as_deref(),
                        run_model.as_deref(),
                        run_thinking,
                    )
                    .await
                {
                    Ok(a) => a,
                    Err(e) => {
                        // Error path: same requeue discipline as abort/budget.
                        self.requeue_pending(&mut pending);
                        // **The run is billed for whatever it spent before it failed.**
                        //
                        // `usage_total` accumulates on every `Done` inside
                        // `stream_one_turn`, and `stream_one_turn` also updates
                        // `session_usage_by_model` live — so on this arm the two totals
                        // disagreed by exactly what the run had already cost, and RPC's
                        // `get_session_stats` publishes both. A client that adds up the
                        // per-model rows and compares them against the flat total got two
                        // different answers for one session, on the sessions where the bill
                        // actually mattered.
                        //
                        // Reachable only on a durable-write or poisoned-lock failure, which
                        // is the F07 class: the turn before this one was answered, delivered,
                        // and paid for.
                        if let Ok(mut session) = self.session_usage.lock() {
                            *session = session.combine(usage_total);
                        }
                        return Err(e);
                    }
                };
                let m = Message::Assistant(assistant.clone());
                produced.push(m);
                // pi pushes the assistant message into the run's context the moment the
                // stream finishes (`streamAssistantResponse` → `context.messages.push`), so a
                // replacement installed on an earlier turn gets its own output back — without
                // this the provider would receive the same frozen request forever.
                Agent::append_to_turn_context(
                    &mut turn_context,
                    &Message::Assistant(assistant.clone()),
                );

                // ---- overflow recovery (ADR-007 trigger #2): compact once, retry ----
                if assistant.stop_reason == StopReason::Error
                    && assistant
                        .error_message
                        .as_deref()
                        .is_some_and(looks_like_context_overflow)
                    && !self.overflow_retried.swap(true, Ordering::SeqCst)
                {
                    // Discard the failed attempt, shrink, and retry this turn. The replacement
                    // must drop it too, or runtime history and the run's context would
                    // disagree by one message. F07: a pop the store refused leaves the store
                    // *ahead* (still holding the oversized attempt), so the retry stops here
                    // instead of re-sending it — `?` is the whole point of the fallible pop.
                    self.pop_last_message()?;
                    produced.pop();
                    if let Some(ctx) = turn_context.as_mut() {
                        ctx.pop();
                    }
                    match self.compact_now(&mut produced).await? {
                        CompactionOutcome::Committed => continue,
                        // Recovery could not summarize: retrying the same oversized
                        // request would just fail again. Report the real reason.
                        CompactionOutcome::Failed(why) => {
                            self.emit(AgentEvent::Notice {
                                text: format!("context overflow recovery failed: {why}"),
                            });
                            reason = reason.merge_sticky(TurnEndReason::Error(why));
                            break 'outer;
                        }
                        CompactionOutcome::Cancelled => {
                            reason = reason.merge_sticky(TurnEndReason::Aborted);
                            break 'outer;
                        }
                    }
                }

                if assistant.stop_reason.is_failure() {
                    reason = reason.merge_sticky(match assistant.stop_reason {
                        StopReason::Aborted => TurnEndReason::Aborted,
                        _ => TurnEndReason::Error(
                            assistant.error_message.clone().unwrap_or_default(),
                        ),
                    });
                    self.emit(AgentEvent::TurnEnd {
                        turn_index,
                        message: Box::new(Message::Assistant(assistant)),
                        tool_results: Vec::new(),
                    });
                    break 'outer;
                }

                // ---- tool batch -----------------------------------------------------
                let call_count = assistant.tool_calls().count();
                // Kept for the empty-turn guard below: `assistant` is moved into the TurnEnd
                // event further down.
                let assistant_text = assistant.text();
                let mut results: Vec<epsilon_contracts::message::ToolResultMessage> = Vec::new();
                let mut stuck_at: Option<usize> = None;
                has_more_tools = false;

                if call_count > 0 {
                    let batch = if assistant.stop_reason == StopReason::MaxTokens {
                        // pi's fail-all rule: truncated arguments are unsafe to execute.
                        exec::fail_all_truncated(&assistant)
                    } else {
                        exec::execute_batch(self, &assistant).await?
                    };
                    reason = reason.merge_sticky(batch.reason);
                    has_more_tools = !batch.terminate;
                    results = batch.results;
                }

                // ---- stuck-run guard (port plan item 7) --------------------------------
                // Each result is folded into the progress guard; when the model has received the
                // same thing several times in a row the guidance rides along with the result it
                // is about, which is the cheapest place to reach the model.
                if let Ok(mut guard) = self.progress.lock() {
                    let text = |r: &epsilon_contracts::message::ToolResultMessage| {
                        r.content
                            .iter()
                            .filter_map(|c| match c {
                                epsilon_contracts::content::ContentBlock::Text { text } => {
                                    Some(text.as_str())
                                }
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    };
                    for r in results.iter_mut() {
                        let repeats = match guard.observe(r.is_error, &text(r)) {
                            crate::guards::Progress::New => continue,
                            crate::guards::Progress::Repeated { repeats } => repeats,
                        };
                        let threshold = self.limits.no_progress_threshold;
                        if guard.stalled_at(threshold) {
                            let hard = threshold > 0 && repeats >= threshold.saturating_mul(2);
                            let note = crate::guards::stuck_guidance(repeats, hard);
                            r.content
                                .push(epsilon_contracts::content::ContentBlock::text(note));
                            stuck_at = Some(repeats);
                        }
                    }
                }

                // ---- verification evidence (port plan item 5) ---------------------------
                // Which files changed, and whether a check has passed since. The record is what
                // the stop-time nudge reads, and what a reader of the session can audit.
                for (call_id, tool_name, args) in assistant.tool_calls() {
                    let result = results.iter().find(|r| r.tool_call_id == call_id);
                    let delivered = result.map(|r| !r.is_error).unwrap_or(false);
                    // F05: "the tool call was delivered" is not "the command passed". A shell
                    // command that exits nonzero arrives as a successful call carrying a failed
                    // command, so a check is only evidence when it exited zero. Edits keep the
                    // plain test — a refused write changed nothing, which is what `delivered`
                    // already means for them.
                    let ok = match (tool_name, result) {
                        ("bash", Some(r)) => delivered && crate::verify::check_passed(&r.details),
                        _ => delivered,
                    };
                    evidence.observe(tool_name, args, ok, turn_index);
                }

                for r in &results {
                    // M229 / A11: **a tool's own provider call is this turn's spend too.** The
                    // `task` tool runs a subagent, which makes real billed requests on the
                    // parent's account; before this, that usage rode in on the tool result
                    // (`ToolResultMessage.usage`, persisted) and was aggregated nowhere — the
                    // provider stream below sums assistant messages only, so the turn total,
                    // `session_usage()` and the per-model cost map each under-reported by
                    // exactly the child's cost.
                    //
                    // Folded here rather than at the tool boundary because this is the point where
                    // the turn's own total is live, and a fold anywhere else would leave the
                    // transcript, the header and the cost panel disagreeing about the same tokens.
                    if let Some(u) = r.usage {
                        usage_total = usage_total.combine(u);
                        // Per-model only when the tool says which model it ran on: a bill with no
                        // model cannot be priced without inventing a rate, which is M141's defect.
                        // It still counts in the total above, so an unattributed bill under-reports
                        // *where* it went, never *whether* it was spent.
                        if let Some(model) = r.details.get("model").and_then(|v| v.as_str())
                            && let Ok(mut by_model) = self.session_usage_by_model.lock()
                        {
                            let row = by_model.entry(model.to_string()).or_default();
                            *row = row.combine(u);
                        }
                    }
                    let m = Message::ToolResult(r.clone());
                    // pi emits full message lifecycle for tool results too — frontends
                    // render from these events, not by scraping internals.
                    self.emit(AgentEvent::MessageStart {
                        message: Box::new(m.clone()),
                    });
                    // F07: stop rather than continue with a result the store does not have.
                    // The results were already computed, but the next request is built from the
                    // store's projection — carrying on would announce a tool outcome to the user
                    // that the model's transcript does not contain.
                    self.push_message(&m)?;
                    // ...and the run's context carries them like any transcript message (pi
                    // appends each result to `currentContext.messages` as it completes), so a
                    // replacement installed earlier keeps tracking the turn's output.
                    Agent::append_to_turn_context(&mut turn_context, &m);
                    produced.push(m);
                    self.emit(AgentEvent::MessageEnd {
                        message: Box::new(Message::ToolResult(r.clone())),
                    });
                }

                // CORE-02: the hook context carries the same messages the transcript just
                // recorded, so a replacement context can be built from the turn's output.
                let turn_message = Message::Assistant(assistant);
                let tool_messages: Vec<Message> =
                    results.iter().cloned().map(Message::ToolResult).collect();
                self.emit(AgentEvent::TurnEnd {
                    turn_index,
                    message: Box::new(turn_message.clone()),
                    tool_results: results,
                });
                turn_index += 1;

                // ---- turn hooks (CORE-02; pi `prepareNextTurn` / `shouldStopAfterTurn`) ----
                // Order follows pi `agent-loop.ts:224-257` exactly: `prepare_next_turn` runs
                // right after `turn_end`, its snapshot replaces the run's context/model/
                // thinking, and `should_stop_after_turn` — evaluated on the REPLACED context,
                // as pi does — runs BEFORE any queue polling. A true stop ends the run here:
                // no steering drain, no follow-up drain (those wait for the next run), no
                // stuck-run notice, and `agent_end` is emitted by `run()` as for any exit.
                // Read-only: neither hook can see the store lock or write history (ADR-007).
                //
                // The context a hook sees is the run's replacement when it holds one, else ONE
                // shared read with the request builder (`try_context_view`). No replacement and
                // no readable view (poisoned lock, failed model view) means the seams are
                // skipped for this turn: a hook must not be handed a view the request would
                // refuse, and the request below raises that failure as its own `RunError`.
                let hooks = self.turn_hooks();
                let owned_view: Option<Vec<Message>> = match turn_context {
                    Some(_) => None,
                    None => self.try_context_view().ok(),
                };
                let prepared = match turn_context.as_deref().or(owned_view.as_deref()) {
                    Some(seen) => {
                        contain_hook(
                            hooks.prepare_next_turn(&epsilon_contracts::turn::NextTurnContext {
                                message: &turn_message,
                                tool_results: &tool_messages,
                                context: seen,
                                new_messages: &produced,
                            }),
                            None,
                        )
                        .await
                    }
                    None => None,
                };
                if let Some(update) = prepared {
                    if let Some(ctx) = update.context {
                        turn_context = Some(ctx);
                        // The replacement is derived from the history as it is NOW; a later
                        // wholesale replacement of that history makes it stale (blocker 2).
                        turn_context_epoch = self.history_epoch();
                    }
                    if let Some(model) = update.model {
                        run_model = Some(model.id);
                    }
                    if update.thinking.is_some() {
                        run_thinking = update.thinking;
                    }
                }
                // pi evaluates the stop decision on the REPLACED context (`agent-loop.ts:248-253`
                // passes `currentContext`, which the snapshot above may have swapped).
                let stop = match turn_context.as_deref().or(owned_view.as_deref()) {
                    Some(seen) => {
                        contain_hook(
                            hooks.should_stop_after_turn(
                                &epsilon_contracts::turn::NextTurnContext {
                                    message: &turn_message,
                                    tool_results: &tool_messages,
                                    context: seen,
                                    new_messages: &produced,
                                },
                            ),
                            false,
                        )
                        .await
                    }
                    None => false,
                };
                if stop {
                    break 'outer;
                }
                if let Some(repeats) = stuck_at {
                    self.emit(AgentEvent::Notice {
                        text: format!("stuck-run guard: {repeats} identical tool results in a row"),
                    });
                }

                pending = self.drain_steering();

                // ---- empty / degenerate turn (port plan item 7) ------------------------
                // A run may end on a turn that said nothing at all, or on one that repeated a
                // single line until it hit the token limit. Both look like success to the loop
                // and like a stuck agent to the user. One nudge, then a clean failure.
                if call_count == 0
                    && let Some(why) = crate::guards::empty_turn_reason(&assistant_text, 0)
                {
                    empty_turns += 1;
                    if empty_turns <= self.limits.empty_turn_retries {
                        let nudge = crate::guards::nudge_for(&why, empty_turns);
                        let m = Message::User(epsilon_contracts::message::UserMessage {
                            id: epsilon_contracts::new_id("msg"),
                            timestamp_ms: now_ms(),
                            content: vec![UserContent::Text { text: nudge }],
                        });
                        // The next iteration's drain point puts it in history and `produced`,
                        // so the nudge is part of the record rather than a whisper.
                        pending.push(m);
                        continue;
                    }
                    reason = reason.merge_sticky(TurnEndReason::Error(why.describe()));
                    break 'outer;
                }

                // ---- stop-time verification nudge (port plan item 5) --------------------
                // The run is about to end. If it changed files and nothing has passed since, say
                // so once (twice at most) — the model decides, and the record stays in the
                // transcript either way.
                if call_count == 0
                    && evidence.has_edits()
                    && verification_nudges < self.limits.verification_nudges
                    && let Some(nudge) = evidence.stop_nudge()
                {
                    verification_nudges += 1;
                    self.emit(AgentEvent::Notice {
                        text: format!("unverified edits: {}", evidence.unverified().join(", ")),
                    });
                    pending.push(Message::User(epsilon_contracts::message::UserMessage {
                        id: epsilon_contracts::new_id("msg"),
                        timestamp_ms: now_ms(),
                        content: vec![UserContent::Text { text: nudge }],
                    }));
                    continue;
                }
            }

            // The agent would stop here; honor queued follow-ups before exiting.
            let followups = self.drain_followups();
            if followups.is_empty() {
                break;
            }
            pending = followups;
        }

        // Fold this run's usage into the session total BEFORE handing it back, so there is
        // one commit point rather than two places that could each be forgotten. A caller
        // that reads `session_usage()` immediately after `run()` returns therefore sees
        // the run it just made.
        if let Ok(mut session) = self.session_usage.lock() {
            *session = session.combine(usage_total);
        }

        Ok(RunOutcome {
            produced,
            reason,
            usage: usage_total,
        })
    }

    /// Provider usage summed over every run this agent has served.
    ///
    /// Read by pi's `get_session_stats` and by anything else that needs the session's
    /// spend without owning a `UiState`. Deliberately independent of the TUI's own
    /// accumulator: a fresh agent that has run one turn reports it, and an agent restored
    /// by `/resume` reports only what ran in this process — which is why the TUI keeps its
    /// hydration path and this is not a replacement for it.
    pub fn session_usage(&self) -> Usage {
        self.session_usage.lock().map(|u| *u).unwrap_or_default()
    }

    /// [`Agent::session_usage`] split by the model that produced each response.
    ///
    /// A clone rather than a borrow because the lock is a `Mutex` and the caller prices
    /// the rows; at one call per RPC request the copy is not worth a guard's lifetime.
    pub fn session_usage_by_model(&self) -> std::collections::BTreeMap<String, Usage> {
        self.session_usage_by_model
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    /// Pressure trigger (ADR-007 #1): estimated context exceeds the live budget.
    ///
    /// Pi parity (`shouldCompact`): the budget is window-relative —
    /// `context_window - reserve` — whenever the active model's window is known,
    /// gated on the `enabled` switch. Unknown windows fall back to the fixed
    /// threshold, preserving the pre-M34 behavior for custom/faux models.
    fn compaction_due(&self) -> bool {
        if !self.limits.compaction_enabled {
            return false;
        }
        let tokens = self.estimated_context_tokens();
        let window = self.model_window_tokens.lock().ok().and_then(|slot| *slot);
        let limit = window
            .map(|w| w.saturating_sub(self.limits.compaction_reserve_tokens))
            .unwrap_or(self.limits.compaction_threshold_tokens);
        tokens > limit
    }

    /// Best estimate of how many tokens the next request will carry.
    ///
    /// Measured where a measurement exists: the provider reports the true prompt
    /// size on every response, so the estimate is that number plus a heuristic
    /// over whatever has been appended since (`compact::anchored_tokens`). Before
    /// this, the trigger was a `chars / 4` guess over the whole transcript, which
    /// over-counts prose and under-counts code and CJK by the same factor —
    /// compacting early wastes a summary call and a cache miss, compacting late
    /// overflows the window.
    ///
    /// Public because frontends show it: the footer's context percentage should
    /// agree with the number the loop actually decides on.
    pub fn estimated_context_tokens(&self) -> usize {
        let anchor = self.usage_anchor.lock().ok().and_then(|slot| *slot);
        // Borrowed, not `&self.history()`. This is called once per drawn frame
        // from the TUI's event loop — the same thread that must read the
        // keyboard — and a deep clone of the whole transcript on that thread
        // scales with session length. Measured at 13µs for 20 messages and
        // 144µs for 200 (see `crates/epsilon-core/tests/context_mirror_cost.rs`),
        // i.e. 10.8x for a 10x larger transcript: the signature of a whole-history
        // scan rather than the anchored walk this intends to be. The tripwire in
        // that test fails until the clone is gone.
        let fixed = self.fixed_prefix_tokens();
        self.with_history(|h| crate::compact::anchored_tokens(h, anchor, fixed))
    }

    /// Tokens every request carries that are **not** in the transcript: the system
    /// prompt and the tool schemas.
    ///
    /// Needed only by the no-anchor fallback in [`Self::estimated_context_tokens`],
    /// where nothing else has counted them — the anchored arm uses the provider's own
    /// `prompt_tokens`, which already includes them. See
    /// [`crate::compact::anchored_tokens`] for why adding them twice would be worse
    /// than the omission.
    ///
    /// Measured from the same inputs the real request is built from, so it cannot
    /// drift from what is actually sent: the descriptors are serialized exactly as a
    /// provider receives them, and the system prompt uses the same `prompt::build`
    /// path the turn takes.
    fn fixed_prefix_tokens(&self) -> usize {
        let tools = self.tools.descriptors();
        let schema_tokens = tools
            .iter()
            .map(|d| {
                let name = crate::compact::estimate_tokens(&d.name);
                let description = crate::compact::estimate_tokens(&d.description);
                let schema = d
                    .parameters_schema
                    .to_string()
                    .len()
                    .div_ceil(4)
                    .saturating_add(8);
                name + description + schema
            })
            .sum::<usize>();
        // The agent contract is the floor: it is what `prompt::build` emits when no
        // overlay replaces it, and it is ~6.5 KB, so omitting it understated the
        // prefix by more than everything else here.
        let contract = crate::compact::estimate_tokens(crate::prompt::AGENT_CONTRACT);
        contract + schema_tokens
    }

    /// Record what the provider billed for the request that covered `covered`
    /// messages (called after a completed turn; cleared when history is rewritten).
    fn set_usage_anchor(&self, prompt_tokens: usize, covered: usize) {
        if let Ok(mut slot) = self.usage_anchor.lock() {
            *slot = Some(crate::compact::UsageAnchor {
                prompt_tokens,
                covered,
            });
        }
    }

    /// The cache-miss attribution report (`crate::cache_stats`), one line per fact.
    ///
    /// Public because it is a user-facing diagnostic: the footer's hit rate says *whether* the
    /// prompt cache is paying off, and this says *why* when it is not.
    pub fn cache_report(&self) -> Vec<String> {
        self.cache
            .lock()
            .map(|stats| stats.report())
            .unwrap_or_else(|_| vec!["cache: unavailable".to_string()])
    }

    /// How many consecutive tool results have been identical, when that is more than one
    /// (`crate::guards::ProgressGuard` — the stuck-run brake reads this before each call).
    pub fn progress_streak(&self) -> Option<usize> {
        let guard = self.progress.lock().ok()?;
        let streak = guard.streak();
        (streak > 1).then_some(streak)
    }

    /// Drop the anchor: the history it describes no longer exists.
    /// Whether the context figure is a provider measurement or a local estimate.
    ///
    /// The same number either way, and the difference is exactly what a reader cannot
    /// infer from it. `anchored_tokens` prefers provider truth for the prefix and
    /// estimates only the tail; when no anchor survives it estimates everything, from a
    /// 4-chars-per-token heuristic that over-counts prose and under-counts code. The
    /// result is still the best available figure — but on turn one, after every
    /// compaction, and after `/resume` it is a *guess*, and the header should say so
    /// rather than presenting a guess and a measurement in the same typeface.
    ///
    /// Hermes ships this on every status payload for the same reason
    /// (`context_breakdown.py:94-115`), and DSH is blunter: its projection doc calls the
    /// occupancy figure "a user-facing reference, not a billing or gating input" — which
    /// is only useful to a user if the figure says how it was made.
    pub fn context_is_estimated(&self) -> bool {
        let anchor = self.usage_anchor.lock().ok().and_then(|slot| *slot);
        // The same guard the estimator applies, kept in step deliberately: if the
        // anchor cannot describe this history, the number is an estimate. Deriving it
        // twice from one rule is better than a flag that can disagree with the value.
        let Some(a) = anchor else { return true };
        let len = self.history.lock().map(|h| h.len()).unwrap_or(0);
        a.prompt_tokens == 0 || a.covered > len
    }

    pub(crate) fn clear_usage_anchor(&self) {
        if let Ok(mut slot) = self.usage_anchor.lock() {
            *slot = None;
        }
    }

    /// Bumped by every wholesale replacement of runtime history (`compact_now` on commit,
    /// `restore_history`). A run's `prepare_next_turn` replacement is only valid while the
    /// history it was derived from is still the run's base — see the field docs.
    pub(crate) fn bump_history_epoch(&self) {
        self.history_epoch
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Current epoch of runtime history (see [`Agent::bump_history_epoch`]).
    fn history_epoch(&self) -> u64 {
        self.history_epoch.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Run one full compaction pass: prune → summarize (isolated call, amend-mode when a
    /// prior summary exists) → append the durable entry (store attached) → replace runtime
    /// history → emit events.
    ///
    /// Commits only after a usable summary arrived, cancellation was checked at the
    /// last moment, and the durable entry (when a store is attached) appended
    /// successfully; every other path leaves history, store and provider view as
    /// they were and says why. Provider failures arrive as outcomes rather than
    /// `RunError`s because a failed summarizer is a compaction problem, not an
    /// agent-invariant violation.
    ///
    /// Event shape: `CompactionStarted` is emitted for every attempt, `CompactionFinished`
    /// only when the pass committed. `Failed` is surfaced to frontends by its `Notice`
    /// (which replaces any "compacting…" status); `Cancelled` ends the run immediately, so
    /// the frontends' run-end handling clears the status instead.
    pub async fn compact_now(
        self: &Arc<Self>,
        produced: &mut Vec<Message>,
    ) -> Result<CompactionOutcome, RunError> {
        use crate::compact;
        self.emit(AgentEvent::CompactionStarted);

        let history = self.history();
        let tail = clamp_tail_tokens(self.limits.compaction_tail_tokens);
        let (pruned_hist, pruned_count) = compact::prune_old_tool_results(&history, tail);

        // Split into [compactable | protected tail] by token budget from the END.
        let mut acc = 0usize;
        let mut split = pruned_hist.len();
        for i in (0..pruned_hist.len()).rev() {
            acc += estimate_one(&pruned_hist[i]);
            if acc >= tail {
                split = i;
                break;
            }
            split = i;
        }
        // ...and then to the nearest *legal* boundary. A tail that begins with a
        // tool result has lost its call to the summary; providers reject that
        // shape, so the turn would fail after the summary was already paid for.
        let split = compact::legal_cut_point(&pruned_hist, split);
        let (middle, kept_tail): (&[Message], &[Message]) = if split >= pruned_hist.len() {
            (&[], &pruned_hist[..])
        } else {
            (&pruned_hist[..split], &pruned_hist[split..])
        };

        // Amend mode when a previous summary is already in the middle range.
        let prior = middle.iter().find_map(|m| match m {
            Message::User(u)
                if u.content.iter().any(|c| {
                    matches!(c,
                    UserContent::Text { text } if text.starts_with("<compaction_summary>"))
                }) =>
            {
                u.content.iter().find_map(|c| match c {
                    UserContent::Text { text } => Some(text.clone()),
                    _ => None,
                })
            }
            _ => None,
        });

        // Ledger: everything the previous summary recorded, plus the files this
        // pass is about to summarize away. Facts from the tool calls themselves,
        // so the model keeps knowing what it already read and changed.
        let mut ledger = prior
            .as_deref()
            .map(compact::FileLedger::from_summary)
            .unwrap_or_default();
        ledger.merge(&compact::FileLedger::scan(middle));

        // Isolated summarization call (fresh request; no tools).
        let sctx = compact::summary_context(middle, prior.as_deref(), &ledger);
        let model_id = self
            .model_id
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| "faux-1".into());
        let model = epsilon_llm::ModelRef::new(model_id);
        // Summarization is a real provider call (isolated on the aux stream, but still
        // billable and still mine to cancel): cancellation must reach it the way it
        // reaches the main turn, and a cancelled pass commits nothing (pi checks the
        // signal before the append — `agent-session.ts:1854-1881`). Biased select:
        // an already-cancelled token wins over a ready frame.
        let run_token = self.current_run_token();
        // Bound to a local before the select: `summarizer_stream()` returns a
        // temporary `Arc`, and the stream future borrows it (E0716 otherwise — the
        // same shape `stream_one_turn` uses for the main provider).
        let summarizer = self.summarizer_stream();
        let mut stream = tokio::select! {
            biased;
            _ = async {
                match run_token {
                    Some(ref t) => t.cancelled().await,
                    None => std::future::pending::<()>().await,
                }
            } => return Ok(CompactionOutcome::Cancelled),
            s = summarizer.stream(&model, sctx) => s,
        };
        let answer = loop {
            tokio::select! {
                biased;
                _ = async {
                    match run_token {
                        Some(ref t) => t.cancelled().await,
                        None => std::future::pending::<()>().await,
                    }
                } => return Ok(CompactionOutcome::Cancelled),
                ev = stream.next() => {
                    match ev {
                        Some(StreamEvent::Done { message }) => break message,
                        Some(_) => continue,
                        None => {
                            return Ok(CompactionOutcome::Failed(
                                "summarizer stream ended early".into(),
                            ));
                        }
                    }
                }
            }
        };
        // A failed provider turn is not a summary: pi rejects `stopReason === "error"`
        // before the append (`compaction.ts:670-685`). Accepting it would replace
        // useful context with failure prose — the opposite of what compaction is for.
        if answer.stop_reason.is_failure() {
            return Ok(CompactionOutcome::Failed(format!(
                "summarization turn failed: {}",
                answer
                    .error_message
                    .as_deref()
                    .unwrap_or("provider reported an error")
            )));
        }
        // No usable text, for one of two reasons — and they need different
        // words, because they need different responses from whoever reads the
        // log. An empty answer is refused rather than padded: the template
        // fallback used to guarantee "valid-looking" output, which meant a blank
        // summary could silently replace the context it failed to summarize. A
        // summary cut off by the output limit is refused for the harder reason:
        // it looks complete, so committing it would replace the real transcript
        // with a plausible-looking record of a shorter session.
        let Some(text) = compact::summary_text(&answer) else {
            let why = if answer.stop_reason == epsilon_contracts::message::StopReason::MaxTokens {
                "summarization hit the output limit and was cut off; a truncated summary \
                 is worse than none because it reads as complete"
            } else {
                "summarization returned no text"
            };
            return Ok(CompactionOutcome::Failed(why.into()));
        };
        let summary = compact::summary_with_ledger(&text, &ledger);
        // ...plus the user's own words from the range just summarized, verbatim: a paraphrase of
        // an instruction is an instruction followed approximately (Hermes micro-compaction).
        let summary =
            compact::summary_with_instructions(&summary, &compact::user_instruction_block(middle));

        // New runtime history: summary-as-context-message + retained tail.
        let mut new_history = Vec::with_capacity(kept_tail.len() + 1);
        new_history.push(Message::User(epsilon_contracts::message::UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: now_ms(),
            content: vec![UserContent::Text {
                text: format!("<compaction_summary>\n{summary}\n</compaction_summary>"),
            }],
        }));
        new_history.extend(kept_tail.iter().cloned());

        let estimate_messages_before: usize = self.estimated_context_tokens();
        // Last cancellation check before anything durable or live changes: a token
        // cancelled while the summary streamed must not still rewrite the transcript.
        if run_token.as_ref().is_some_and(|t| t.is_cancelled()) {
            return Ok(CompactionOutcome::Cancelled);
        }
        // Persist the compaction entry so the durable mirror's MODEL VIEW shadows the
        // range — before runtime history is replaced. With a store attached, provider
        // requests read the store's projection, so committing only one of the two views
        // would leave them disagreeing (the F07 append-failure shape).
        if let Some(store) = self.store() {
            let mut st = store.lock().map_err(|_| store_lock_failure())?;
            if let Err(e) = st.append(EntryKind::Compaction {
                summary: summary.clone(),
                tokens_before: estimate_messages_before as u64,
                retained_tail: kept_tail.to_vec(),
                details: Some(serde_json::json!({ "pruned_tool_results": pruned_count })),
                // M145/F22: the summarizer's real bill, on the entry that incurred it. The
                // response has been in scope here all along and the usage was discarded, so
                // a genuine provider call was free in every number the user could see.
                //
                // One entry, not two. M144 appended a SEPARATE accounting entry after this
                // one and was reverted: that row became the file's last entry, and
                // `pop_last_message` pops the last entry to discard a failed turn, so a
                // rollback separated a summary from its cost (F23). pi puts the usage here
                // for the same reason (`session-manager.ts:68-80`).
                //
                // The model comes from the **response**, not the `ModelRef` the request
                // was built from — which is `self.model_id` (`:3030-3035`) and therefore
                // the session's *current* model. The summarizer is an isolated stream and
                // the whole point of installing one is that it can answer from elsewhere,
                // so the request's model is only what was *asked for*. The main-turn path
                // already reads the model off the message (`:3464`) for the same reason:
                // a response that lands after a `/model` switch belongs to the model that
                // generated it.
                //
                // Epsilon prices from the catalog when it tallies, so a bill filed under
                // the wrong model prices that call at the wrong rates on every read,
                // forever, with nothing on screen to contradict it — the same
                // attributes-somewhere class rounds 138 and 139 just surfaced.
                usage: Some(answer.usage),
                model_id: Some(answer.model_id.clone()),
            }) {
                // F07: nothing has diverged yet — memory is untouched and the caller reports the
                // failure — but the store just refused a durable write, so a later request must
                // not be built from it on the assumption that writes work.
                self.durable_write_failed(format!("compaction append: {e}"));
                return Ok(CompactionOutcome::Failed(format!(
                    "compaction entry could not be appended: {e}"
                )));
            }
        }

        match self.history.lock() {
            Ok(mut h) => *h = new_history,
            Err(_) => {
                // Poisoned history: the durable entry was already appended. Drop it again
                // so the two views cannot disagree — with a store attached the provider
                // request reads the store's projection, and it must describe the same
                // transcript runtime history holds. F07: when that undo itself fails the
                // views DO disagree (the store keeps a compaction runtime history never
                // applied), so the failure is reported rather than discarded, exactly as
                // for an append that fails outright.
                if let Some(store) = self.store()
                    && let Ok(mut st) = store.lock()
                    && let Err(e) = st.pop_last()
                {
                    self.durable_write_failed(format!("compaction rollback pop: {e}"));
                }
                return Err(RunError::Internal("history poisoned".into()));
            }
        }
        // The anchor described the transcript that was just replaced; the next
        // request re-measures from scratch.
        self.clear_usage_anchor();
        // CORE-02: the history a mid-run `prepare_next_turn` replacement was built on
        // no longer exists. Bumping the epoch invalidates it, so the next request rebuilds
        // from the current source — the store's shadowed view when one is attached — and
        // an overflow retry cannot re-send the request the compaction just fixed.
        self.bump_history_epoch();

        // **The summarizer's own call is spend, in the numbers a reader can see.**
        //
        // The bill reached the durable `Compaction` entry and the `CompactionFinished`
        // event — and neither in-memory total. The session totals are folded at exactly
        // two points, both inside the run loop (`:2686-2688` and `:2352-2354`), and a
        // compaction happens *outside* that loop: it is the summarizer's own aux call,
        // which the provider stream never sums. So the call that resends the whole
        // context — the most expensive one in a long session — was free in
        // `session_usage()`, in the desktop's cost panel, and in `get_session_stats`.
        //
        // The TUI hid the hole: it re-derives its figures from `usage_accounts()` on
        // hydration (`state.rs:1701-1708`), so the terminal was right and the window was
        // wrong — a defect survives when one front end happens to recover.
        //
        // Folded here rather than inside the append because the two in-memory totals are
        // what the run leaves behind, and the poisoned-history path below pops the durable
        // entry: a bill counted there would outlive the compaction it paid for.
        if let Ok(mut session) = self.session_usage.lock() {
            *session = session.combine(answer.usage);
        }
        if let Ok(mut by_model) = self.session_usage_by_model.lock() {
            let row = by_model.entry(answer.model_id.clone()).or_default();
            *row = row.combine(answer.usage);
        }
        produced.push(Message::User(epsilon_contracts::message::UserMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: now_ms(),
            content: vec![UserContent::Text {
                text: COMPACTION_MARKER.into(),
            }],
        }));
        // The same usage that went onto the durable entry above, handed to the live
        // UI as well. Without it the cost is only visible after `/resume`, so the
        // number rises when a user reopens a session they were just working in.
        //
        // `answer.model_id`, not `model.id`: the event is the only live surface that
        // prices this bill (the terminal folds it into the same per-model rows the
        // request path uses — `epsilon-tui/src/state.rs:1702`), and a wrong model here
        // mis-prices it in the running session, not just on replay.
        self.emit(AgentEvent::CompactionFinished {
            usage: Some(answer.usage),
            model_id: Some(answer.model_id.clone()),
            summary: summary.clone(),
            tokens_before: estimate_messages_before as u64,
        });
        Ok(CompactionOutcome::Committed)
    }

    /// Synthesize the in-band abort message when cancellation wins mid-stream.
    ///
    /// The folded-so-far text/thinking is preserved (it really streamed);
    /// tool-call fragments are dropped — they never executed, and keeping them
    /// would fabricate dangling calls. The caller treats this like any failed
    /// turn: `TurnEndReason::Aborted`, no tool batch, run ends.
    /// Terminal message for a run that dies mid-stream — abort, transport
    /// failure, or a stream that ends without a terminal frame (M62).
    ///
    /// Rule (mirrors `HttpEventStream::failure_message` on the wire side):
    /// whatever the user already watched arrive stays, the turn is flagged with
    /// the real stop reason, and tool calls whose arguments may be half-written
    /// are dropped. Throwing away visible output is worse than an ugly
    /// transcript — and a resumed session rebuilding context from the store
    /// needs the partial reasoning, not a hole where it used to be.
    fn failure_message(
        &self,
        partial: Option<&AssistantMessage>,
        stop: StopReason,
        detail: &str,
    ) -> AssistantMessage {
        use epsilon_contracts::content::ContentBlock;
        let model_id = self
            .model_id
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| "faux-1".into());
        let (provider, content, usage) = match partial {
            Some(p) => (
                p.provider.clone(),
                p.content
                    .iter()
                    .filter(|b| match b {
                        ContentBlock::ToolCall { .. } => false,
                        ContentBlock::Text { text } => !text.is_empty(),
                        ContentBlock::Thinking { text, .. } => !text.is_empty(),
                        _ => true,
                    })
                    .cloned()
                    .collect(),
                p.usage,
            ),
            None => ("unknown".into(), Vec::new(), Usage::default()),
        };
        AssistantMessage {
            id: epsilon_contracts::new_id("msg"),
            timestamp_ms: now_ms(),
            provider,
            model_id,
            content,
            stop_reason: stop,
            error_message: Some(detail.to_string()),
            usage,
        }
    }

    /// [`Agent::failure_message`] for a user abort.
    fn aborted_message(&self, partial: Option<&AssistantMessage>) -> AssistantMessage {
        self.failure_message(partial, StopReason::Aborted, "aborted by user")
    }

    /// One provider turn, built from the run's current context.
    ///
    /// `turn_context` is a `prepare_next_turn` replacement when a hook installed one (it
    /// wins over every other source — pi replaces `currentContext` wholesale);
    /// `model_override` / `thinking_override` are that snapshot's per-run model and
    /// thinking level. All three are `None` for a run without turn hooks, which is the
    /// pre-CORE-02 behaviour exactly.
    async fn stream_one_turn(
        self: &Arc<Self>,
        usage_total: &mut Usage,
        turn_context: Option<&[Message]>,
        model_override: Option<&str>,
        thinking_override: Option<epsilon_contracts::ThinkingLevel>,
    ) -> Result<AssistantMessage, RunError> {
        // Provider-visible view: a hook replacement first, then the store's shadowed view
        // when attached, else runtime history. One shared read with the hook seam
        // (`try_context_view`), so a hook can never be handed a view this request would
        // refuse — and the two can never disagree about what the provider-bound context is.
        let from_replacement = turn_context.is_some();
        let history: Vec<Message> = match turn_context {
            Some(ctx) => ctx.to_vec(),
            None => self.try_context_view()?,
        };
        // How much of the history this request covers: the anchor recorded from
        // its usage describes exactly these messages, so the next pressure check
        // only has to estimate what is appended after them.
        //
        // A replacement is NOT runtime history: `covered` indexes the front of the
        // transcript (`crate::compact::anchored_tokens` slices `history[covered..]` of
        // runtime history), so a replacement's length would mis-index the anchor and
        // trigger needless paid compactions. Requests built from a replacement therefore
        // carry no anchor — dropped below, where the usage arrives.
        let covered = history.len();

        // ---- three-tier system prompt (ADR-008) ------------------------------------
        // The stable tier's fingerprint travels with this call: it is the bytes a provider's
        // prompt cache is expected to keep, so a change in it explains a cache miss
        // (`crate::cache_stats`) rather than leaving one unexplained.
        let tool_lines: Vec<prompt::ToolLine> = self
            .tools
            .descriptors()
            .iter()
            .map(|d| prompt::ToolLine {
                name: d.name.clone(),
                snippet: d.description.lines().next().unwrap_or("").to_string(),
            })
            .collect();
        // A `prepare_next_turn` model override wins for the rest of this run; otherwise the
        // session's model id (same lifecycle rule pi applies to `config.model`).
        let model_id = match model_override {
            Some(id) => id.to_string(),
            None => self
                .model_id
                .lock()
                .map(|s| s.clone())
                .unwrap_or_else(|_| "faux-1".into()),
        };
        // The identity overlays are discovered once per frontend, so a delegated child has to
        // be handed the same ones: a child that ran under the built-in contract while its
        // parent ran under the user's `SYSTEM.md` would be a different agent than the one the
        // task was delegated from.
        let overlay = self.system_overlay();
        let stable = prompt::Stable {
            tools: &tool_lines,
            custom_identity: overlay.custom.as_deref(),
            append: overlay.append.as_deref(),
        };
        let system_prompt = prompt::build(
            &stable,
            &prompt::ContextTier {
                agent_brief: self
                    .agent_brief
                    .lock()
                    .map(|s| s.clone())
                    .unwrap_or_default(),
                instructions: {
                    let mut instructions = prompt::load_project_instructions(&self.workspace_root);
                    // Subdirectory hints (M73) ride in the same tier, after the root files:
                    // they are just as project-sourced, and the closest directory is the one
                    // that governs the file being edited.
                    instructions.extend(prompt::load_subdirectory_hints(
                        &self.workspace_root,
                        &self.touched_dirs(),
                    ));
                    instructions
                },
                skills_catalog: Some(crate::skills::catalog(&self.skill_list())),
                // The objective, when there is one — and only its words: the round counter and
                // the revision are what `get_goal` is for, and a number that moves every round
                // would rewrite the cached prefix on every round.
                goal: self
                    .goal()
                    .map(|view| format!("This session's goal: {}", view.goal.objective)),
            },
            &prompt::Volatile {
                cwd: &self.workspace_root,
                model_id: &model_id,
                today: today_string(),
            },
        );
        // The stable tier is what the provider is expected to keep cached; its fingerprint is
        // recorded with the call so an unexplained miss can be told from an explained one.
        // The same `Stable` value that built the prompt, so the two cannot drift apart.
        let stable_hash = prompt::stable_fingerprint(&stable);

        // CORE-02 request boundary (pi `agent-loop.ts:288-295`): transform the run's context
        // first, then convert to provider-visible messages. Both are pure reads — the store
        // is never written and runtime history is untouched (ADR-007) — and both are
        // contained: a hook that panics falls back to the seam's no-op default (identity for
        // the transform, the harness-visibility filter for the conversion), exactly as if no
        // host had registered one.
        let hooks = self.turn_hooks();
        let transformed = contain_hook(hooks.transform_context(&history), history.clone()).await;
        let messages = contain_hook(
            hooks.convert_to_llm(&transformed),
            transformed
                .iter()
                .filter(|m| m.is_llm_visible())
                .cloned()
                .collect::<Vec<Message>>(),
        )
        .await;
        let ctx = epsilon_llm::LlmContext {
            thinking_level: thinking_override.unwrap_or_else(|| self.thinking_level()),
            system_prompt,
            messages,
            tools: self.tools.descriptors(),
        };

        let model = epsilon_llm::ModelRef::new(model_id);
        let stream_fn = self
            .stream_fn
            .read()
            .map(|g| Arc::clone(&*g))
            .unwrap_or_else(|e| Arc::clone(&*e.into_inner()));
        // Abort reaches the provider stream, not just the tools (pi passes the
        // AbortSignal to `streamFunction`): cancellation wins the select below,
        // the provider stream is dropped, and the run ends in-band as Aborted.
        let token = self.current_run_token();
        // Both awaits below honor cancellation: stream creation (provider
        // connect/TTFB hangs) and event polling. Whichever select wins first
        // decides; `biased` prefers an already-cancelled token over a ready event.
        let mut stream = tokio::select! {
            biased;
            _ = async {
                match token {
                    Some(ref t) => t.cancelled().await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                // A terminal message reaches the transcript the same way a completed
                // turn does: runtime history, durable mirror, and a MessageEnd event.
                // Returning early used to leave the visible abort only in `produced`,
                // so a resumed conversation lost the interrupted answer (pi persists
                // assistant message-end events; `agent-session.ts:639-656`).
                let message = self.aborted_message(None);
                self.push_message(&Message::Assistant(message.clone()))?;
                self.emit(AgentEvent::MessageEnd {
                    message: Box::new(Message::Assistant(message.clone())),
                });
                return Ok(message);
            }
            s = stream_fn.stream(&model, ctx) => s,
        };
        let mut folder = StreamFolder::default();

        loop {
            tokio::select! {
                biased;
                _ = async {
                    match token {
                        Some(ref t) => t.cancelled().await,
                        None => std::future::pending::<()>().await,
                    }
                } => {
                    // Same lifecycle as the connect-time abort above: the folded-so-far
                    // text is real output and must survive in the transcript.
                    let message = self.aborted_message(folder.partial());
                    self.push_message(&Message::Assistant(message.clone()))?;
                    self.emit(AgentEvent::MessageEnd {
                        message: Box::new(Message::Assistant(message.clone())),
                    });
                    return Ok(message);
                }
                ev = stream.next() => {
                    match ev {
                        Some(StreamEvent::Done { message }) => {
                            *usage_total = usage_total.combine(message.usage);
                            // Bill the response to the model that produced it, read from
                            // the message rather than from `self` — a response that lands
                            // after a `/model` switch belongs to the model that generated
                            // it, which is what makes a multi-model session price correctly.
                            if let Ok(mut by_model) = self.session_usage_by_model.lock() {
                                let row = by_model
                                    .entry(message.model_id.clone())
                                    .or_default();
                                *row = row.combine(message.usage);
                            }
                            // Cache bookkeeping (port plan item 4): what this call cost the
                            // prompt cache, and if it missed, why.
                            if let Ok(mut cache) = self.cache.lock() {
                                cache.observe(crate::cache_stats::CacheRequest {
                                    model: model.id.clone(),
                                    prefix_hash: stable_hash,
                                    usage: message.usage,
                                    at_ms: now_ms(),
                                });
                            }
                            // Anchor the next pressure check to what the provider
                            // actually billed for this request (pi usage anchor).
                            // Cache traffic counts: those tokens were in the prompt.
                            // A request built from a replacement drops the anchor instead
                            // (see `covered` above): its count indexes a transcript that is
                            // not the one `anchored_tokens` re-reads, so keeping it would
                            // mis-size the next pressure decision.
                            if from_replacement {
                                self.clear_usage_anchor();
                            } else {
                                self.set_usage_anchor(
                                    (message.usage.input
                                        + message.usage.cache_read
                                        + message.usage.cache_write)
                                        as usize,
                                    covered,
                                );
                            }
                            self.push_message(&Message::Assistant(message.clone()))?;
                            self.emit(AgentEvent::MessageEnd {
                                message: Box::new(Message::Assistant(message.clone())),
                            });
                            return Ok(message);
                        }
                        Some(other) => {
                            // Fold for live partials; Done handled above.
                            //
                            // Broadcast discipline: argument-fragment deltas are pure transport
                            // noise (hundreds per call on big payloads) — they'd overflow slow
                            // subscribers' rings for zero UI value, so they fold WITHOUT being
                            // broadcast. Visible content still streams live.
                            if matches!(other, StreamEvent::ToolCallArgumentsDelta { .. }) {
                                let _ = folder.apply(other);
                            } else if folder.apply(other).is_some()
                                && let Some(partial) = folder.partial()
                            {
                                self.emit(AgentEvent::MessageUpdate {
                                    message: Box::new(Message::Assistant(partial.clone())),
                                });
                            }
                        }
                        // Stream ended with no Done and no abort. If anything was
                        // decoded, that partial IS the turn's output: keep it and
                        // flag the failure (M62). With nothing decoded there is
                        // nothing to salvage, so this stays an internal error.
                        None => {
                            let partial = folder.partial().cloned();
                            let has_content = partial.as_ref().is_some_and(|p| {
                                !p.text().is_empty()
                                    || p.content.iter().any(|b| {
                                        matches!(
                                            b,
                                            epsilon_contracts::content::ContentBlock::Thinking { .. }
                                        )
                                    })
                            });
                            if !has_content {
                                return Err(RunError::Internal(
                                    "provider stream ended without Done".into(),
                                ));
                            }
                            let message = self.failure_message(
                                partial.as_ref(),
                                StopReason::Error,
                                "provider stream ended without a terminal frame — partial \\
                                 output kept",
                            );
                            self.push_message(&Message::Assistant(message.clone()))?;
                            self.emit(AgentEvent::MessageEnd {
                                message: Box::new(Message::Assistant(message.clone())),
                            });
                            return Ok(message);
                        }
                    }
                }
            }
        }
    }
}
