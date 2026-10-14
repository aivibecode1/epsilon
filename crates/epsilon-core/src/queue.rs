//! The two message queues a run drains at its stop boundaries (pi parity).

use std::collections::VecDeque;

use epsilon_contracts::QueueMode;
use epsilon_contracts::agent_event::AgentEvent;
use epsilon_contracts::message::Message;

use crate::Agent;

#[derive(Debug)]
pub(crate) struct PendingQueue {
    inner: VecDeque<Message>,
    mode: QueueMode,
}

impl PendingQueue {
    pub(crate) fn new(mode: QueueMode) -> Self {
        Self {
            inner: VecDeque::new(),
            mode,
        }
    }

    fn push(&mut self, m: Message) {
        self.inner.push_back(m);
    }

    /// Restore drained messages to the front (abnormal-exit requeue, plan 002).
    fn restore(&mut self, msgs: Vec<Message>) {
        for m in msgs.into_iter().rev() {
            self.inner.push_front(m);
        }
    }

    fn drain(&mut self) -> Vec<Message> {
        match self.mode {
            QueueMode::All => self.inner.drain(..).collect(),
            QueueMode::OneAtATime => self.inner.pop_front().into_iter().collect(),
        }
    }

    fn len(&self) -> usize {
        self.inner.len()
    }

    /// Drain all queued messages regardless of mode (used by abort restore).
    fn drain_all(&mut self) -> Vec<Message> {
        self.inner.drain(..).collect()
    }

    fn set_mode(&mut self, mode: QueueMode) {
        self.mode = mode;
    }

    fn mode(&self) -> QueueMode {
        self.mode
    }
}

impl Agent {
    /// Queue a mid-run interjection; drained at the next drain point (after current turn).
    pub fn steer(&self, m: Message) {
        if let Ok(mut q) = self.steering.lock() {
            q.push(m);
        }
    }

    // M30: queue visibility — the TUI reads this to show a count badge in the
    // status bar so the user knows their interjections are queued, not lost.
    /// Number of steering messages waiting to be delivered after the current turn.
    pub fn steer_queue_len(&self) -> usize {
        self.steering.lock().map(|q| q.len()).unwrap_or(0)
    }

    /// How much of the steering queue releases at a drain point.
    ///
    /// pi's `setSteeringMode` (`rpc-mode.ts`). Readable as well as settable because a
    /// client that changes a setting needs to be able to see what is in effect: an
    /// unreadable setting is one the client cannot confirm and can only guess at.
    pub fn steering_mode(&self) -> QueueMode {
        self.steering
            .lock()
            .map(|q| q.mode())
            .unwrap_or(QueueMode::OneAtATime)
    }

    /// Set how much of the steering queue releases at a drain point.
    ///
    /// **Takes effect at the next drain, and does not disturb what is already queued.**
    /// The mode is a property of the *drain*, not of the queue's contents, so switching
    /// from `OneAtATime` to `All` with three messages waiting releases all three at the
    /// next drain point and switching the other way releases one. The alternative —
    /// draining on the spot — would deliver messages the user never asked to be released
    /// now, in the middle of a turn.
    pub fn set_steering_mode(&self, mode: QueueMode) {
        if let Ok(mut q) = self.steering.lock() {
            q.set_mode(mode);
        }
    }

    /// How much of the follow-up queue releases at a drain point (pi's
    /// `setFollowUpMode`).
    pub fn follow_up_mode(&self) -> QueueMode {
        self.followups
            .lock()
            .map(|q| q.mode())
            .unwrap_or(QueueMode::OneAtATime)
    }

    /// Set how much of the follow-up queue releases at a drain point.
    ///
    /// The same mid-queue semantics as [`Agent::set_steering_mode`], and the same reason:
    /// the mode belongs to the drain, so changing it must not itself deliver anything.
    pub fn set_follow_up_mode(&self, mode: QueueMode) {
        if let Ok(mut q) = self.followups.lock() {
            q.set_mode(mode);
        }
    }

    /// Queue work for after the agent would otherwise stop.
    pub fn follow_up(&self, m: Message) {
        if let Ok(mut q) = self.followups.lock() {
            q.push(m);
        }
    }

    pub(crate) fn drain_steering(&self) -> Vec<Message> {
        let mut out = self
            .steering
            .lock()
            .map(|mut q| q.drain())
            .unwrap_or_default();
        // M53: a background child's completion notice rides the same drain
        // point as user interjections, so a finishing subagent reaches the
        // model at the next turn — or at the next run's start when the session
        // is idle (run start prefills from this same queue).
        if let Ok(mut rx) = self.task_notices_rx.lock() {
            while let Ok(m) = rx.try_recv() {
                // The model is not the only reader: a notice the user cannot
                // see is a notice the user cannot trust, so it enters the
                // transcript with the same event a normal message uses.
                self.emit(AgentEvent::MessageEnd {
                    message: Box::new(m.clone()),
                });
                out.push(m);
            }
        }
        out
    }

    /// Drain ALL queued follow-up messages, ignoring queue mode.
    ///
    /// The follow-up counterpart to [`Agent::drain_steering_all`], and it exists because
    /// pi's `clear_queue` returns **both** queues (`rpc-types.ts:59`: `data: { steering,
    /// followUp }`). Without this, a client asking to clear the queue would silently leave
    /// every follow-up queued — the messages that run *after* the agent would otherwise stop
    /// — and the reply would claim a clear that did not happen. That is the same
    /// acknowledge-without-acting failure the TUI hit when it restored steering to the
    /// editor but had nowhere to put follow-ups.
    pub fn drain_followups_all(&self) -> Vec<Message> {
        self.followups
            .lock()
            .map(|mut q| q.drain_all())
            .unwrap_or_default()
    }

    /// Number of queued follow-up messages, for a queue-depth reply.
    pub fn followup_queue_len(&self) -> usize {
        self.followups.lock().map(|q| q.len()).unwrap_or(0)
    }

    /// M30: drain ALL queued steering messages on abort (ignoring queue mode)
    /// so the TUI can restore them to the editor.
    pub fn drain_steering_all(&self) -> Vec<Message> {
        self.steering
            .lock()
            .map(|mut q| q.drain_all())
            .unwrap_or_default()
    }

    /// Plan 002: on abnormal exits (abort/budget/error), put drained-but-uninjected
    /// messages back where they came from. `pending` is steering-sourced inside the
    /// inner loop (follow-ups only enter as fresh `pending` at the outer boundary,
    /// where an empty follow-up batch simply breaks), so requeueing to the steering
    /// queue preserves delivery order and loses nothing.
    pub(crate) fn requeue_pending(&self, pending: &mut Vec<Message>) {
        if pending.is_empty() {
            return;
        }
        if let Ok(mut q) = self.steering.lock() {
            for m in pending.drain(..).rev() {
                q.restore(vec![m]);
            }
        } else {
            pending.clear();
        }
    }

    pub(crate) fn drain_followups(&self) -> Vec<Message> {
        self.followups
            .lock()
            .map(|mut q| q.drain())
            .unwrap_or_default()
    }
}
