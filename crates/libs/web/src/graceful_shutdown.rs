use std::time::Duration;

use runtime::shutdown::Shutdown;

/// Resolves once the server should stop accepting connections: `shutdown`
/// triggered (readiness already reports shutting down), then `drain_delay`
/// elapsed with the server still handling requests.
pub async fn graceful_shutdown(shutdown: Shutdown, drain_delay: Duration) {
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

    use runtime::shutdown::Shutdown;
    use tokio::time::timeout;

    use super::*;

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
    async fn waits_out_the_drain_delay() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        let delay = Duration::from_secs(5);

        let resolved = timeout(Duration::from_secs(4), graceful_shutdown(shutdown, delay)).await;

        assert!(resolved.is_err());
    }
}
