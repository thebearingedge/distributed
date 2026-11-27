use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use runtime::run_error::RunError;
use runtime::supervisor::{Supervised, Supervisor};

use crate::worker::{Worker, WorkerContext};

/// Supervisor task name for the worker's probe server.
pub const WORKER_PROBE_SERVER_TASK: &str = "probe_server";

/// Supervises the worker (task named [`crate::Worker::name`]) and the
/// probe-server future (task [`crate::supervisor::WORKER_PROBE_SERVER_TASK`])
/// on one [`runtime::supervisor::Supervisor`] sharing the context's shutdown.
/// Either task stopping first (error, panic, or a clean return before
/// shutdown) triggers the shutdown and becomes the culprit. Tasks still
/// running `deadline` after the shutdown trigger are aborted.
///
/// # Panics
///
/// Panics when polled outside a Tokio runtime (see
/// [`runtime::supervisor::Supervisor::spawn`]).
pub async fn supervise<W, S>(
    worker: Arc<W>,
    context: WorkerContext,
    server: S,
    deadline: Duration,
) -> Supervised
where
    W: Worker + ?Sized + 'static,
    S: Future<Output = Result<(), RunError>> + Send + 'static,
{
    let mut supervisor = Supervisor::new(context.shutdown().clone());
    let name = worker.name();
    supervisor.spawn(name, async move { worker.run(context).await });
    supervisor.spawn(WORKER_PROBE_SERVER_TASK, server);
    supervisor.run(deadline).await
}

#[cfg(test)]
mod tests {
    use std::future::{Future, pending};
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use opentelemetry::metrics::NoopMeterProvider;
    use tokio::time::{Instant, timeout};

    use runtime::exit_status::ExitStatus;
    use runtime::heartbeat::Heartbeat;
    use runtime::lifecycle::RunOutcome;
    use runtime::run_error::RunError;
    use runtime::shutdown::Shutdown;
    use runtime::supervisor::Supervised;

    use crate::worker::{Worker, WorkerContext};

    use super::{WORKER_PROBE_SERVER_TASK, supervise};

    fn context(shutdown: &Shutdown) -> WorkerContext {
        WorkerContext::new(
            shutdown.clone(),
            Heartbeat::new(Duration::from_secs(30)),
            Arc::new(NoopMeterProvider::new()),
        )
    }

    fn shutdown_aware_server(
        shutdown: &Shutdown,
    ) -> impl Future<Output = Result<(), RunError>> + Send + 'static {
        let shutdown = shutdown.clone();
        async move {
            shutdown.triggered().await;
            Ok(())
        }
    }

    struct StopsOnShutdown;

    #[async_trait]
    impl Worker for StopsOnShutdown {
        fn name(&self) -> &'static str {
            "stops_on_shutdown"
        }

        async fn run(&self, context: WorkerContext) -> Result<(), RunError> {
            context.shutdown().triggered().await;
            Ok(())
        }
    }

    #[derive(Clone, Copy)]
    enum Preset {
        Clean,
        RetryableFailure,
        PermanentFailure,
    }

    struct ReturnsImmediately(Preset);

    #[async_trait]
    impl Worker for ReturnsImmediately {
        fn name(&self) -> &'static str {
            "returns_immediately"
        }

        async fn run(&self, _context: WorkerContext) -> Result<(), RunError> {
            match self.0 {
                Preset::Clean => Ok(()),
                Preset::RetryableFailure => Err(RunError::retryable("transient worker failure")),
                Preset::PermanentFailure => Err(RunError::permanent("bad worker configuration")),
            }
        }
    }

    struct Panics;

    #[async_trait]
    impl Worker for Panics {
        fn name(&self) -> &'static str {
            "panics"
        }

        async fn run(&self, _context: WorkerContext) -> Result<(), RunError> {
            panic!("worker exploded")
        }
    }

    struct IgnoresShutdown;

    #[async_trait]
    impl Worker for IgnoresShutdown {
        fn name(&self) -> &'static str {
            "ignores_shutdown"
        }

        async fn run(&self, _context: WorkerContext) -> Result<(), RunError> {
            pending().await
        }
    }

    #[tokio::test(start_paused = true)]
    async fn external_shutdown_with_clean_stop_is_success() {
        let shutdown = Shutdown::new();
        shutdown.trigger();

        let supervised = timeout(
            Duration::from_secs(60),
            supervise(
                Arc::new(StopsOnShutdown),
                context(&shutdown),
                shutdown_aware_server(&shutdown),
                Duration::from_secs(5),
            ),
        )
        .await
        .expect("supervision completes after external shutdown");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::completed(ExitStatus::Success),
                culprit: None,
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn worker_returning_early_is_retryable() {
        let shutdown = Shutdown::new();

        let supervised = timeout(
            Duration::from_secs(60),
            supervise(
                Arc::new(ReturnsImmediately(Preset::Clean)),
                context(&shutdown),
                shutdown_aware_server(&shutdown),
                Duration::from_secs(5),
            ),
        )
        .await
        .expect("supervision completes after the worker returns early");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::completed(ExitStatus::Retryable),
                culprit: Some("returns_immediately"),
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn worker_permanent_error_is_permanent() {
        let shutdown = Shutdown::new();

        let supervised = timeout(
            Duration::from_secs(60),
            supervise(
                Arc::new(ReturnsImmediately(Preset::PermanentFailure)),
                context(&shutdown),
                shutdown_aware_server(&shutdown),
                Duration::from_secs(5),
            ),
        )
        .await
        .expect("supervision completes after the worker fails");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::completed(ExitStatus::Permanent),
                culprit: Some("returns_immediately"),
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn worker_retryable_error_is_retryable() {
        let shutdown = Shutdown::new();

        let supervised = timeout(
            Duration::from_secs(60),
            supervise(
                Arc::new(ReturnsImmediately(Preset::RetryableFailure)),
                context(&shutdown),
                shutdown_aware_server(&shutdown),
                Duration::from_secs(5),
            ),
        )
        .await
        .expect("supervision completes after the worker fails");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::completed(ExitStatus::Retryable),
                culprit: Some("returns_immediately"),
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn server_error_is_retryable_and_stops_the_worker() {
        let shutdown = Shutdown::new();

        let supervised = timeout(
            Duration::from_secs(60),
            supervise(
                Arc::new(StopsOnShutdown),
                context(&shutdown),
                async { Err(RunError::retryable("boom")) },
                Duration::from_secs(5),
            ),
        )
        .await
        .expect("supervision completes after the server fails");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::completed(ExitStatus::Retryable),
                culprit: Some(WORKER_PROBE_SERVER_TASK),
            }
        );
        assert!(shutdown.is_triggered());
    }

    #[tokio::test(start_paused = true)]
    async fn worker_panic_is_retryable() {
        let shutdown = Shutdown::new();

        let supervised = timeout(
            Duration::from_secs(60),
            supervise(
                Arc::new(Panics),
                context(&shutdown),
                shutdown_aware_server(&shutdown),
                Duration::from_secs(5),
            ),
        )
        .await
        .expect("supervision completes after the worker panics");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::completed(ExitStatus::Retryable),
                culprit: Some("panics"),
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn worker_ignoring_shutdown_exceeds_the_deadline() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        let started = Instant::now();

        let supervised = timeout(
            Duration::from_secs(60),
            supervise(
                Arc::new(IgnoresShutdown),
                context(&shutdown),
                shutdown_aware_server(&shutdown),
                Duration::from_secs(5),
            ),
        )
        .await
        .expect("supervision completes at the deadline");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::abandoned(ExitStatus::Interrupted),
                culprit: None,
            }
        );
        assert_eq!(started.elapsed(), Duration::from_secs(5));
    }
}
