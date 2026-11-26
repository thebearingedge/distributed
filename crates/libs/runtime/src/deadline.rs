use std::future::Future;
use std::time::Duration;

use crate::shutdown::Shutdown;

/// The shutdown deadline passed before a future completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("shutdown deadline of {deadline:?} exceeded")]
pub struct DeadlineExceeded {
    /// The deadline, measured from the shutdown trigger.
    pub deadline: Duration,
}

/// Runs `future` to completion, unless `shutdown` is triggered and `deadline`
/// then passes first.
///
/// The deadline clock starts only once `shutdown` is triggered (immediately if
/// it already was); before that, `future` may run for any length of time.
/// When the deadline wins, `future` is dropped.
///
/// # Errors
///
/// Returns [`DeadlineExceeded`] when `deadline` passes after the shutdown
/// trigger before `future` completes.
pub async fn within_deadline<F: Future>(
    shutdown: &Shutdown,
    deadline: Duration,
    future: F,
) -> Result<F::Output, DeadlineExceeded> {
    let expired = async {
        shutdown.triggered().await;
        tokio::time::sleep(deadline).await;
    };
    tokio::select! {
        biased;
        output = future => Ok(output),
        () = expired => Err(DeadlineExceeded { deadline }),
    }
}

#[cfg(test)]
mod tests {
    use std::future::pending;
    use std::time::Duration;

    use tokio::time::{Instant, sleep};

    use super::{DeadlineExceeded, within_deadline};
    use crate::shutdown::Shutdown;

    #[tokio::test(start_paused = true)]
    async fn completes_before_shutdown() {
        let shutdown = Shutdown::new();

        let result = within_deadline(&shutdown, Duration::from_secs(5), async { 7 }).await;

        assert_eq!(result, Ok(7));
    }

    #[tokio::test(start_paused = true)]
    async fn long_future_is_not_bounded_until_shutdown() {
        let shutdown = Shutdown::new();

        let result = within_deadline(&shutdown, Duration::from_secs(5), async {
            sleep(Duration::from_secs(3600)).await;
            1
        })
        .await;

        assert_eq!(result, Ok(1));
    }

    #[tokio::test(start_paused = true)]
    async fn completes_within_the_deadline_after_trigger() {
        let shutdown = Shutdown::new();
        shutdown.trigger();

        let result = within_deadline(&shutdown, Duration::from_secs(5), async {
            sleep(Duration::from_secs(4)).await;
        })
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn exceeds_the_deadline_after_trigger() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        let started = Instant::now();

        let result = within_deadline(&shutdown, Duration::from_secs(5), pending::<()>()).await;

        assert_eq!(
            result,
            Err(DeadlineExceeded {
                deadline: Duration::from_secs(5),
            })
        );
        assert_eq!(started.elapsed(), Duration::from_secs(5));
    }

    #[tokio::test(start_paused = true)]
    async fn deadline_starts_at_the_trigger() {
        let shutdown = Shutdown::new();
        let trigger = shutdown.clone();
        tokio::spawn(async move {
            sleep(Duration::from_secs(10)).await;
            trigger.trigger();
        });
        let started = Instant::now();

        let result = within_deadline(&shutdown, Duration::from_secs(5), pending::<()>()).await;

        assert!(result.is_err());
        assert_eq!(started.elapsed(), Duration::from_secs(15));
    }

    #[tokio::test(start_paused = true)]
    async fn zero_deadline_expires_immediately_once_triggered() {
        let shutdown = Shutdown::new();
        shutdown.trigger();

        let result = within_deadline(&shutdown, Duration::ZERO, pending::<()>()).await;

        assert!(result.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn display_names_the_deadline() {
        let error = DeadlineExceeded {
            deadline: Duration::from_secs(5),
        };

        assert_eq!(error.to_string(), "shutdown deadline of 5s exceeded");
    }
}
