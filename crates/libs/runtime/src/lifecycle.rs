use std::fmt::Display;
use std::process::ExitCode;
use std::time::Duration;

use crate::exit_status::ExitStatus;

/// How a supervised run ended: its exit status, and whether unfinished work
/// was abandoned because the shutdown deadline passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunOutcome {
    /// The exit status of the run.
    pub status: ExitStatus,
    /// Whether tasks were still running when the shutdown deadline passed.
    pub deadline_exceeded: bool,
}

impl RunOutcome {
    /// A run whose tasks all stopped before the deadline.
    #[must_use]
    pub const fn completed(status: ExitStatus) -> Self {
        Self {
            status,
            deadline_exceeded: false,
        }
    }

    /// A run whose remaining tasks were abandoned at the deadline.
    #[must_use]
    pub const fn abandoned(status: ExitStatus) -> Self {
        Self {
            status,
            deadline_exceeded: true,
        }
    }
}

/// How the final flush ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlushOutcome {
    /// The flush completed successfully.
    Flushed,
    /// The flush returned an error or its blocking task panicked.
    Failed,
    /// The flush did not finish within its timeout.
    TimedOut,
}

/// Runs the blocking `flush` on the blocking thread pool, waiting at most
/// `timeout`. Failures and timeouts are logged as warnings and never panic.
pub async fn flush_within<F, E>(timeout: Duration, flush: F) -> FlushOutcome
where
    F: FnOnce() -> Result<(), E> + Send + 'static,
    E: Display + Send + 'static,
{
    match tokio::time::timeout(timeout, tokio::task::spawn_blocking(flush)).await {
        Ok(Ok(Ok(()))) => FlushOutcome::Flushed,
        Ok(Ok(Err(error))) => {
            tracing::warn!(%error, "shutdown flush failed");
            FlushOutcome::Failed
        }
        Ok(Err(error)) => {
            tracing::warn!(%error, "shutdown flush task failed");
            FlushOutcome::Failed
        }
        Err(_) => {
            tracing::warn!(timeout_secs = timeout.as_secs(), "shutdown flush timed out");
            FlushOutcome::TimedOut
        }
    }
}

/// Whether the process must end with [`std::process::exit`] instead of
/// returning from `main`: work was abandoned at the deadline, or the flush
/// timed out. Returning would let the Tokio runtime wait on that work
/// indefinitely.
#[must_use]
pub const fn must_exit_immediately(outcome: RunOutcome, flush: FlushOutcome) -> bool {
    outcome.deadline_exceeded || matches!(flush, FlushOutcome::TimedOut)
}

/// Ends a process run: flushes telemetry with
/// [`crate::lifecycle::flush_within`], then returns the [`ExitCode`] for
/// `outcome.status`. A failed flush is logged and never changes the status.
/// When [`crate::lifecycle::must_exit_immediately`] holds, the process exits
/// with the status code from inside this function instead of returning.
pub async fn finish<F, E>(outcome: RunOutcome, flush_timeout: Duration, flush: F) -> ExitCode
where
    F: FnOnce() -> Result<(), E> + Send + 'static,
    E: Display + Send + 'static,
{
    let flushed = flush_within(flush_timeout, flush).await;
    if must_exit_immediately(outcome, flushed) {
        tracing::warn!(
            code = outcome.status.code(),
            "abandoning unfinished work and exiting"
        );
        std::process::exit(i32::from(outcome.status.code()));
    }
    outcome.status.into()
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;
    use std::time::Duration;

    use super::{FlushOutcome, RunOutcome, flush_within, must_exit_immediately};
    use crate::exit_status::ExitStatus;

    #[test]
    fn completed_is_not_deadline_exceeded() {
        assert!(!RunOutcome::completed(ExitStatus::Success).deadline_exceeded);
    }

    #[test]
    fn abandoned_is_deadline_exceeded() {
        assert!(RunOutcome::abandoned(ExitStatus::Interrupted).deadline_exceeded);
    }

    #[tokio::test(start_paused = true)]
    async fn flush_success_is_flushed() {
        let outcome = flush_within(Duration::from_secs(5), || Ok::<(), String>(())).await;

        assert_eq!(outcome, FlushOutcome::Flushed);
    }

    #[tokio::test(start_paused = true)]
    async fn flush_error_is_failed() {
        let outcome = flush_within(Duration::from_secs(5), || {
            Err::<(), String>("collector down".into())
        })
        .await;

        assert_eq!(outcome, FlushOutcome::Failed);
    }

    #[tokio::test(start_paused = true)]
    async fn flush_panic_is_failed() {
        let outcome = flush_within(Duration::from_secs(5), || -> Result<(), String> {
            panic!("flush exploded")
        })
        .await;

        assert_eq!(outcome, FlushOutcome::Failed);
    }

    #[tokio::test(start_paused = true)]
    async fn slow_flush_times_out() {
        let (release, blocker) = channel::<()>();
        let flush = tokio::spawn(flush_within(Duration::from_secs(5), move || {
            let _ = blocker.recv();
            Ok::<(), String>(())
        }));
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(6)).await;
        let outcome = flush.await.expect("flush_within does not panic");

        release
            .send(())
            .expect("the blocking flush is still waiting");
        assert_eq!(outcome, FlushOutcome::TimedOut);
    }

    #[test]
    fn must_exit_immediately_table() {
        let completed = RunOutcome::completed(ExitStatus::Success);
        let abandoned = RunOutcome::abandoned(ExitStatus::Interrupted);

        assert!(!must_exit_immediately(completed, FlushOutcome::Flushed));
        assert!(!must_exit_immediately(completed, FlushOutcome::Failed));
        assert!(must_exit_immediately(completed, FlushOutcome::TimedOut));
        assert!(must_exit_immediately(abandoned, FlushOutcome::Flushed));
    }
}
