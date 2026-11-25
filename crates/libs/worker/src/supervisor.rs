use std::io;
use std::pin::pin;

use runtime::exit_status::ExitStatus;
use runtime::run_error::RunError;

use crate::worker::{Worker, WorkerContext};

/// Supervises the worker and the probe-server future until both have stopped.
///
/// Both futures are pinned and raced: whichever finishes first triggers the
/// context shutdown so the other drains, and the remaining future is then
/// awaited to completion.
///
/// The worker's status is derived from its result and from whether the
/// shutdown was already triggered before supervision stopped the futures:
///
/// - a clean stop while shutdown was triggered succeeds;
/// - a clean stop with no shutdown requested is retryable;
/// - a worker error maps through [`RunError::exit_status`].
///
/// The server's status is success when it stopped cleanly after the shutdown
/// was requested, and retryable when it failed or when it stopped on its own
/// before any shutdown was requested. The worker's status is returned, unless
/// the worker finished cleanly, in which case the server's status is
/// returned.
///
/// # Panics
///
/// Panics in either future propagate to the caller.
pub async fn supervise<W, S>(worker: &W, context: WorkerContext, server: S) -> ExitStatus
where
    W: Worker + ?Sized,
    S: Future<Output = io::Result<()>> + Send,
{
    let mut worker_run = pin!(worker.run(context.clone()));
    let mut server = pin!(server);

    let (worker_status, server_status) = tokio::select! {
        result = &mut worker_run => {
            let was_triggered = context.shutdown().is_triggered();
            context.shutdown().trigger();
            let worker_status = classify_worker(worker.name(), was_triggered, result);
            let server_status = classify_server(false, was_triggered, server.as_mut().await);
            (worker_status, server_status)
        }
        result = &mut server => {
            let was_triggered = context.shutdown().is_triggered();
            context.shutdown().trigger();
            let server_status = classify_server(true, was_triggered, result);
            let worker_status = classify_worker(worker.name(), was_triggered, worker_run.await);
            (worker_status, server_status)
        }
    };

    match worker_status {
        ExitStatus::Success => server_status,
        worker_status => worker_status,
    }
}

fn classify_worker(
    name: &'static str,
    was_triggered: bool,
    result: Result<(), RunError>,
) -> ExitStatus {
    match result {
        Ok(()) if was_triggered => ExitStatus::Success,
        Ok(()) => {
            tracing::error!(
                worker = name,
                "worker stopped before shutdown was requested"
            );
            ExitStatus::Retryable
        }
        Err(error) => {
            tracing::error!(worker = name, error = %error, "worker failed");
            error.exit_status()
        }
    }
}

fn classify_server(
    finished_first: bool,
    was_triggered: bool,
    result: io::Result<()>,
) -> ExitStatus {
    match result {
        Ok(()) if finished_first && !was_triggered => {
            tracing::error!("probe server stopped before shutdown was requested");
            ExitStatus::Retryable
        }
        Ok(()) => ExitStatus::Success,
        Err(error) => {
            tracing::error!(error = %error, "probe server failed");
            ExitStatus::Retryable
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::io;
    use std::time::Duration;

    use async_trait::async_trait;
    use tokio::time::timeout;

    use runtime::exit_status::ExitStatus;
    use runtime::heartbeat::Heartbeat;
    use runtime::run_error::RunError;
    use runtime::shutdown::Shutdown;

    use crate::worker::{Worker, WorkerContext};

    use super::supervise;

    fn context(shutdown: &Shutdown) -> WorkerContext {
        WorkerContext::new(shutdown.clone(), Heartbeat::new(Duration::from_secs(30)))
    }

    fn shutdown_aware_server(shutdown: &Shutdown) -> impl Future<Output = io::Result<()>> + Send {
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

    #[tokio::test(start_paused = true)]
    async fn external_shutdown_with_clean_stop_is_success() {
        let shutdown = Shutdown::new();
        shutdown.trigger();

        let status = timeout(
            Duration::from_secs(1),
            supervise(
                &StopsOnShutdown,
                context(&shutdown),
                shutdown_aware_server(&shutdown),
            ),
        )
        .await
        .expect("supervision completes after external shutdown");

        assert_eq!(status, ExitStatus::Success);
    }

    #[tokio::test(start_paused = true)]
    async fn worker_returning_early_is_retryable() {
        let shutdown = Shutdown::new();

        let status = timeout(
            Duration::from_secs(1),
            supervise(
                &ReturnsImmediately(Preset::Clean),
                context(&shutdown),
                shutdown_aware_server(&shutdown),
            ),
        )
        .await
        .expect("supervision completes after the worker returns early");

        assert_eq!(status, ExitStatus::Retryable);
    }

    #[tokio::test(start_paused = true)]
    async fn worker_permanent_error_is_permanent() {
        let shutdown = Shutdown::new();

        let status = timeout(
            Duration::from_secs(1),
            supervise(
                &ReturnsImmediately(Preset::PermanentFailure),
                context(&shutdown),
                async { Ok(()) },
            ),
        )
        .await
        .expect("supervision completes after the worker fails");

        assert_eq!(status, ExitStatus::Permanent);
    }

    #[tokio::test(start_paused = true)]
    async fn worker_retryable_error_is_retryable() {
        let shutdown = Shutdown::new();

        let status = timeout(
            Duration::from_secs(1),
            supervise(
                &ReturnsImmediately(Preset::RetryableFailure),
                context(&shutdown),
                async { Ok(()) },
            ),
        )
        .await
        .expect("supervision completes after the worker fails");

        assert_eq!(status, ExitStatus::Retryable);
    }

    #[tokio::test(start_paused = true)]
    async fn server_error_is_retryable_and_stops_the_worker() {
        let shutdown = Shutdown::new();

        let status = timeout(
            Duration::from_secs(1),
            supervise(&StopsOnShutdown, context(&shutdown), async {
                Err(io::Error::other("boom"))
            }),
        )
        .await
        .expect("supervision completes after the server fails");

        assert_eq!(status, ExitStatus::Retryable);
    }

    #[tokio::test(start_paused = true)]
    async fn worker_error_wins_over_server_error() {
        let shutdown = Shutdown::new();

        let status = timeout(
            Duration::from_secs(1),
            supervise(
                &ReturnsImmediately(Preset::PermanentFailure),
                context(&shutdown),
                async { Err(io::Error::other("boom")) },
            ),
        )
        .await
        .expect("supervision completes");

        assert_eq!(status, ExitStatus::Permanent);
    }
}
