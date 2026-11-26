use std::future::Future;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use tokio::time::Instant;

/// Divisor giving the recommended beat cadence as a fraction of the stale
/// window.
const HEARTBEAT_BEATS_PER_STALE_WINDOW: u32 = 3;

/// Liveness signal for a long-running loop: the loop calls
/// [`crate::heartbeat::Heartbeat::beat`] at least every
/// [`crate::heartbeat::Heartbeat::interval`], and a liveness probe fails once
/// no beat has landed within `stale_after`. Clones share one state.
#[derive(Clone, Debug)]
pub struct Heartbeat(Arc<HeartbeatState>);

#[derive(Debug)]
struct HeartbeatState {
    origin: Instant,
    last_beat_millis: AtomicU64,
    stale_after: Duration,
}

/// A liveness check failure: no beat has landed within the stale window.
#[derive(Debug, thiserror::Error)]
#[error("no heartbeat for {elapsed:?}; stale after {stale_after:?}")]
pub struct StaleHeartbeat {
    /// Time elapsed since the last beat.
    pub elapsed: Duration,
    /// The stale window the heartbeat was created with.
    pub stale_after: Duration,
}

impl Heartbeat {
    /// Creates a heartbeat that goes stale after `stale_after` without a
    /// beat.
    ///
    /// Creation counts as the first beat.
    ///
    /// # Panics
    ///
    /// Panics when `stale_after` is zero.
    #[must_use]
    pub fn new(stale_after: Duration) -> Self {
        assert!(
            !stale_after.is_zero(),
            "heartbeat stale_after must be non-zero"
        );
        Self(Arc::new(HeartbeatState {
            origin: Instant::now(),
            last_beat_millis: AtomicU64::new(0),
            stale_after,
        }))
    }

    /// Records a beat at the current instant.
    pub fn beat(&self) {
        self.0
            .last_beat_millis
            .store(self.millis_since_origin(Instant::now()), Ordering::Relaxed);
    }

    /// The recommended beat cadence: a third of `stale_after`.
    #[must_use]
    pub fn interval(&self) -> Duration {
        self.0.stale_after / HEARTBEAT_BEATS_PER_STALE_WINDOW
    }

    /// The stale window this heartbeat was created with.
    #[must_use]
    pub fn stale_after(&self) -> Duration {
        self.0.stale_after
    }

    /// Checks the heartbeat, failing once it has gone stale.
    ///
    /// # Errors
    ///
    /// Returns [`StaleHeartbeat`] when more than `stale_after` has passed
    /// since the last beat.
    pub fn check(&self) -> Result<(), StaleHeartbeat> {
        let elapsed = Duration::from_millis(
            self.millis_since_origin(Instant::now())
                .saturating_sub(self.0.last_beat_millis.load(Ordering::Relaxed)),
        );
        if elapsed > self.0.stale_after {
            return Err(StaleHeartbeat {
                elapsed,
                stale_after: self.0.stale_after,
            });
        }
        Ok(())
    }

    fn millis_since_origin(&self, now: Instant) -> u64 {
        u64::try_from(now.saturating_duration_since(self.0.origin).as_millis()).unwrap_or(u64::MAX)
    }

    /// Keeps the heartbeat fresh while awaiting a long future: beats every
    /// [`interval`](Self::interval) while `future` is pending, and once more
    /// when it completes, then returns its output. Use it for awaits that can
    /// outlast [`stale_after`](Self::stale_after), such as handlers or message
    /// deliveries.
    pub async fn beat_while<F: Future>(&self, future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut ticker = tokio::time::interval(self.interval());
        loop {
            tokio::select! {
                output = &mut future => {
                    self.beat();
                    return output;
                }
                _ = ticker.tick() => self.beat(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::{advance, sleep};

    use super::*;

    #[test]
    fn fresh_at_creation() {
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        assert!(heartbeat.check().is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn stale_after_window_without_beats() {
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        advance(Duration::from_secs(31)).await;
        let err = heartbeat
            .check()
            .expect_err("stale after 31s without beats");
        assert!(err.elapsed >= Duration::from_secs(31));
        assert_eq!(err.stale_after, Duration::from_secs(30));
    }

    #[tokio::test(start_paused = true)]
    async fn fresh_at_the_window_edge() {
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        advance(Duration::from_secs(30)).await;
        assert!(heartbeat.check().is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn beat_refreshes() {
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        advance(Duration::from_secs(60)).await;
        heartbeat.beat();
        advance(Duration::from_secs(15)).await;
        assert!(heartbeat.check().is_ok());
    }

    #[test]
    fn interval_is_a_third_of_stale_after() {
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        assert_eq!(heartbeat.interval(), Duration::from_secs(10));
    }

    #[tokio::test(start_paused = true)]
    async fn clones_share_state() {
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        let clone = heartbeat.clone();
        advance(Duration::from_secs(60)).await;
        clone.beat();
        advance(Duration::from_secs(15)).await;
        assert!(heartbeat.check().is_ok());
    }

    #[test]
    #[should_panic(expected = "heartbeat stale_after must be non-zero")]
    fn zero_stale_after_panics() {
        let _ = Heartbeat::new(Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn beat_while_keeps_the_heartbeat_fresh_during_a_long_future() {
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        let task_heartbeat = heartbeat.clone();
        let task = tokio::spawn(async move {
            task_heartbeat
                .beat_while(sleep(Duration::from_secs(120)))
                .await;
        });
        sleep(Duration::from_secs(90)).await;
        assert!(heartbeat.check().is_ok());
        task.await.expect("beat_while task completes");
        assert!(heartbeat.check().is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn beat_while_returns_the_future_output() {
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        assert_eq!(heartbeat.beat_while(async { 7 }).await, 7);
    }
}
