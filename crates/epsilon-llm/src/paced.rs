//! A provider that streams at a controllable *rate*, not just a controllable start.
//!
//! # Why this exists
//!
//! The reported Windows TUI defect is: *"it freezes and shows no input or output the
//! moment you start typing, even though tokens are clearly being consumed
//! underneath."* Every test and probe this repository has run against the TUI has
//! missed it, and the reason is mechanical rather than accidental.
//!
//! `FauxProvider` emits an entire turn's events with no delay at all — see
//! [`FauxEventStream::next`], which is a bare `self.pending.pop_front()`. A turn is
//! therefore over in microseconds, and every harness that has ever driven the TUI has
//! photographed a run that was already finished. The one state the report describes —
//! a turn **in flight**, streaming over seconds while the user types — has never been
//! executed by anything.
//!
//! [`SlowProvider`](crate::testing::SlowProvider) does not close the gap either, and
//! the distinction matters. It sleeps *before* `stream` opens, so it can delay the
//! **start** of a turn. It cannot delay the **duration** of one, because a turn's
//! length is decided by how many events the stream yields and how fast the consumer
//! pulls them.
//!
//! So: this wrapper paces the events themselves. A turn that would take 4 ms takes
//! `delay_ms` per event, which for a 40-delta answer is a turn measured in seconds —
//! the shape of a real provider, and the shape the bug needs.
//!
//! # What it is for
//!
//! Not a general-purpose test utility. It exists so that **a TUI test can type while a
//! turn is genuinely in flight**, which is the assertion that would have caught the
//! reported freeze and the one no existing test makes. See
//! `crates/epsilon-cli/tests/terminal.rs`.
//!
//! # Why the delay is per-event rather than a total
//!
//! A total-duration delay would have to know the event count in advance, and the count
//! is not known until the inner stream has been drained — at which point the delay
//! would have to be applied after the fact and would not slow anything down. Per-event
//! is the only formulation that actually produces a long turn, and it has the useful
//! side effect that a longer answer streams for longer, exactly as a real one does.

use async_trait::async_trait;

use epsilon_contracts::stream::StreamEvent;

use crate::stream_fn::{BoxEventStream, EventStream, LlmContext, ModelRef, StreamFn};

/// Paces an inner provider's events at `delay_ms` per event.
///
/// The first event is emitted without a delay so a caller waiting on
/// `StreamEvent::Start` is not made to wait for a turn that has not begun — matching
/// how a real provider opens a stream promptly and then trickles.
pub struct PacedProvider<S> {
    inner: S,
    delay_ms: u64,
}

impl<S> PacedProvider<S> {
    /// Wrap `inner`, sleeping `delay_ms` before every event after the first.
    pub fn new(inner: S, delay_ms: u64) -> Self {
        Self { inner, delay_ms }
    }

    /// The per-event delay this provider was built with.
    pub fn delay_ms(&self) -> u64 {
        self.delay_ms
    }
}

#[async_trait]
impl<S: StreamFn + Send + Sync> StreamFn for PacedProvider<S> {
    async fn stream(&self, model: &ModelRef, ctx: LlmContext) -> BoxEventStream {
        Box::new(PacedStream {
            inner: self.inner.stream(model, ctx).await,
            delay: std::time::Duration::from_millis(self.delay_ms),
            emitted: 0,
        })
    }
}

struct PacedStream {
    inner: BoxEventStream,
    delay: std::time::Duration,
    emitted: usize,
}

#[async_trait]
impl EventStream for PacedStream {
    async fn next(&mut self) -> Option<StreamEvent> {
        // Pace *between* events, not before the first one: a turn that opens
        // promptly and then trickles is what a real provider does, and it means
        // `Start` still arrives immediately.
        if self.emitted > 0 && !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        let event = self.inner.next().await;
        if event.is_some() {
            self.emitted += 1;
        }
        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::faux::{FauxProvider, FauxStep};
    use epsilon_contracts::message::ThinkingLevel;
    use std::time::Instant;

    fn ctx() -> LlmContext {
        LlmContext {
            system_prompt: String::new(),
            messages: vec![],
            tools: vec![],
            thinking_level: ThinkingLevel::Off,
        }
    }

    /// A long answer, so the stream has many events to pace.
    fn long_script() -> FauxProvider {
        FauxProvider::new(vec![FauxStep::say("word ".repeat(400))])
    }

    #[tokio::test]
    async fn a_paced_turn_takes_materially_longer_than_an_unpaced_one() {
        let drain = |mut s: BoxEventStream| async move {
            let mut n = 0;
            while s.next().await.is_some() {
                n += 1;
            }
            n
        };

        let fast = drain(long_script().stream(&ModelRef::new("faux-1"), ctx()).await).await;
        let t0 = Instant::now();
        drain(long_script().stream(&ModelRef::new("faux-1"), ctx()).await).await;
        let unpaced = t0.elapsed();

        let paced = PacedProvider::new(long_script(), 5);
        let t1 = Instant::now();
        let slow = drain(paced.stream(&ModelRef::new("faux-1"), ctx()).await).await;
        let paced_elapsed = t1.elapsed();

        // Same event count: pacing must not add or drop events.
        assert_eq!(fast, slow, "pacing must not change the event sequence");
        assert!(
            paced_elapsed > unpaced * 10,
            "a 5ms pace over {} events should dominate an unpaced drain \
             ({}µs vs {unpaced:?})",
            slow,
            paced_elapsed.as_micros()
        );
    }

    #[tokio::test]
    async fn the_first_event_is_not_delayed() {
        // A caller blocked on `Start` should not wait out one interval first; that
        // is what makes a paced provider usable as a drop-in for the faux one.
        let paced = PacedProvider::new(long_script(), 500);
        let mut s = paced.stream(&ModelRef::new("faux-1"), ctx()).await;
        let t0 = Instant::now();
        let first = s.next().await;
        assert!(
            matches!(first, Some(StreamEvent::Start { .. })),
            "the first event must be Start"
        );
        assert!(
            t0.elapsed() < std::time::Duration::from_millis(400),
            "Start waited {:?} before arriving; it must not pay the first interval",
            t0.elapsed()
        );
    }

    #[tokio::test]
    async fn a_zero_pace_is_a_passthrough() {
        let paced = PacedProvider::new(long_script(), 0);
        assert_eq!(paced.delay_ms(), 0);
        let mut s = paced.stream(&ModelRef::new("faux-1"), ctx()).await;
        let mut n = 0;
        while s.next().await.is_some() {
            n += 1;
        }
        assert!(n > 100, "expected the full script, got {n} events");
    }
}
