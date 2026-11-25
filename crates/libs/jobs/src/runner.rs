//! Runs a job and maps its outcome to an exit status.

use crate::job::Job;
use runtime::exit_status::ExitStatus;
use runtime::shutdown::Shutdown;

/// Runs `job` until it finishes or `shutdown` is triggered, and maps the
/// outcome to an [`ExitStatus`]:
///
/// - `Ok(())` maps to [`ExitStatus::Success`], even when shutdown was already
///   triggered.
/// - An error returned while shutdown is triggered logs a warning and maps to
///   [`ExitStatus::Interrupted`], regardless of the error's classification.
/// - Any other error logs the failure and maps to the error's own
///   [`runtime::run_error::RunError::exit_status`].
///
/// The job receives a clone of `shutdown` and is expected to observe it; the
/// runner does not race or abort the job. Panics inside the job propagate to
/// the caller.
pub async fn run<J: Job + ?Sized>(job: &J, shutdown: Shutdown) -> ExitStatus {
    match job.run(shutdown.clone()).await {
        Ok(()) => ExitStatus::Success,
        Err(error) if shutdown.is_triggered() => {
            tracing::warn!(job = job.name(), %error, "job interrupted by shutdown");
            ExitStatus::Interrupted
        }
        Err(error) => {
            tracing::error!(job = job.name(), %error, "job failed");
            error.exit_status()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use async_trait::async_trait;
    use tokio::time::timeout;

    use super::{Job, run};
    use runtime::exit_status::ExitStatus;
    use runtime::run_error::RunError;
    use runtime::shutdown::Shutdown;

    struct Succeeds;

    #[async_trait]
    impl Job for Succeeds {
        fn name(&self) -> &'static str {
            "succeeds"
        }

        async fn run(&self, _shutdown: Shutdown) -> Result<(), RunError> {
            Ok(())
        }
    }

    struct Fails<F>(F)
    where
        F: Fn() -> RunError;

    #[async_trait]
    impl<F> Job for Fails<F>
    where
        F: Fn() -> RunError + Send + Sync,
    {
        fn name(&self) -> &'static str {
            "fails"
        }

        async fn run(&self, _shutdown: Shutdown) -> Result<(), RunError> {
            Err((self.0)())
        }
    }

    struct FailsOnShutdown;

    #[async_trait]
    impl Job for FailsOnShutdown {
        fn name(&self) -> &'static str {
            "fails_on_shutdown"
        }

        async fn run(&self, shutdown: Shutdown) -> Result<(), RunError> {
            shutdown.triggered().await;
            Err(RunError::retryable("interrupted"))
        }
    }

    struct SucceedsOnShutdown;

    #[async_trait]
    impl Job for SucceedsOnShutdown {
        fn name(&self) -> &'static str {
            "succeeds_on_shutdown"
        }

        async fn run(&self, shutdown: Shutdown) -> Result<(), RunError> {
            shutdown.triggered().await;
            Ok(())
        }
    }

    #[tokio::test(start_paused = true)]
    async fn success_maps_to_success() {
        let status = run(&Succeeds, Shutdown::new()).await;

        assert_eq!(status, ExitStatus::Success);
    }

    #[tokio::test(start_paused = true)]
    async fn retryable_maps_to_retryable() {
        let status = run(
            &Fails(|| RunError::retryable("connection reset")),
            Shutdown::new(),
        )
        .await;

        assert_eq!(status, ExitStatus::Retryable);
    }

    #[tokio::test(start_paused = true)]
    async fn permanent_maps_to_permanent() {
        let status = run(
            &Fails(|| RunError::permanent("bad config")),
            Shutdown::new(),
        )
        .await;

        assert_eq!(status, ExitStatus::Permanent);
    }

    #[tokio::test(start_paused = true)]
    async fn error_after_shutdown_maps_to_interrupted() {
        let shutdown = Shutdown::new();
        shutdown.trigger();

        let status = run(&FailsOnShutdown, shutdown).await;

        assert_eq!(status, ExitStatus::Interrupted);
    }

    #[tokio::test(start_paused = true)]
    async fn permanent_error_after_shutdown_still_maps_to_interrupted() {
        let shutdown = Shutdown::new();
        shutdown.trigger();

        let status = run(&Fails(|| RunError::permanent("bad config")), shutdown).await;

        assert_eq!(status, ExitStatus::Interrupted);
    }

    #[tokio::test(start_paused = true)]
    async fn ok_after_shutdown_maps_to_success() {
        let shutdown = Shutdown::new();
        shutdown.trigger();

        let status = run(&SucceedsOnShutdown, shutdown).await;

        assert_eq!(status, ExitStatus::Success);
    }

    #[tokio::test(start_paused = true)]
    async fn run_forwards_the_shutdown_handle() {
        let shutdown = Shutdown::new();
        let handle = tokio::spawn(run(&FailsOnShutdown, shutdown.clone()));

        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(!handle.is_finished());

        shutdown.trigger();

        let status = timeout(Duration::from_secs(1), handle)
            .await
            .expect("the run finishes once shutdown is triggered")
            .expect("the spawned run does not panic");
        assert_eq!(status, ExitStatus::Interrupted);
    }
}
