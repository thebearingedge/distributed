use std::time::Duration;

use axum::Router;
use tokio::net::TcpListener;

use runtime::run_error::RunError;
use runtime::shutdown::Shutdown;

/// Supervisor task name for an API's HTTP server.
pub(crate) const HTTP_SERVER_TASK: &str = "http_server";

/// Supervisor task name for a worker's probe server.
pub(crate) const PROBE_SERVER_TASK: &str = "probe_server";

/// Serves `router` on `listener` until `shutdown` is triggered and
/// `drain_delay` has elapsed (readiness reports shutting down meanwhile), then
/// stops accepting connections and waits for in-flight requests. Bound the
/// total wait with [`runtime::supervisor::Supervisor`].
///
/// # Errors
///
/// Returns a retryable [`runtime::run_error::RunError`] when the server fails
/// with an I/O error.
pub(crate) async fn serve(
    listener: TcpListener,
    router: Router,
    shutdown: Shutdown,
    drain_delay: Duration,
) -> Result<(), RunError> {
    axum::serve(listener, router)
        .with_graceful_shutdown(graceful_shutdown(shutdown, drain_delay))
        .await
        .map_err(RunError::retryable)
}

/// Resolves once the server should stop accepting connections: `shutdown`
/// triggered (readiness already reports shutting down), then `drain_delay`
/// elapsed with the server still handling requests.
async fn graceful_shutdown(shutdown: Shutdown, drain_delay: Duration) {
    shutdown.triggered().await;
    tracing::info!(
        delay_secs = drain_delay.as_secs(),
        "shutdown signal received, draining"
    );
    tokio::time::sleep(drain_delay).await;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::Router;
    use tokio::net::TcpListener;
    use tokio::time::timeout;

    use runtime::shutdown::Shutdown;

    use super::{graceful_shutdown, serve};

    async fn bind_listener() -> TcpListener {
        TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener binds on an ephemeral port")
    }

    #[tokio::test(start_paused = true)]
    async fn stops_after_trigger_and_drain() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        let listener = bind_listener().await;

        let stopped = timeout(
            Duration::from_secs(6),
            serve(listener, Router::new(), shutdown, Duration::from_secs(5)),
        )
        .await;

        assert!(matches!(stopped, Ok(Ok(()))));
    }

    #[tokio::test(start_paused = true)]
    async fn waits_out_the_drain_delay() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        let listener = bind_listener().await;

        let stopped = timeout(
            Duration::from_secs(4),
            serve(listener, Router::new(), shutdown, Duration::from_secs(5)),
        )
        .await;

        assert!(stopped.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn resolves_after_trigger_and_drain() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        let delay = Duration::from_secs(5);

        let resolved = timeout(Duration::from_secs(6), graceful_shutdown(shutdown, delay)).await;

        assert!(resolved.is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn pends_until_triggered() {
        let shutdown = Shutdown::new();

        let resolved = timeout(
            Duration::from_secs(60),
            graceful_shutdown(shutdown, Duration::ZERO),
        )
        .await;

        assert!(resolved.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn graceful_shutdown_waits_out_the_drain_delay() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        let delay = Duration::from_secs(5);

        let resolved = timeout(Duration::from_secs(4), graceful_shutdown(shutdown, delay)).await;

        assert!(resolved.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn serves_until_triggered() {
        let shutdown = Shutdown::new();
        let listener = bind_listener().await;

        let stopped = timeout(
            Duration::from_secs(60),
            serve(listener, Router::new(), shutdown, Duration::ZERO),
        )
        .await;

        assert!(stopped.is_err());
    }
}
