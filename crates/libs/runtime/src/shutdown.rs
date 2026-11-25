use std::io;

#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;

/// Process-wide shutdown handle: cloned into every task that must stop on
/// shutdown; triggered by an OS signal (see
/// [`Shutdown::listen_for_signals`]) or programmatically.
///
/// All clones share one cancellation state, so any clone can trigger every
/// other.
#[derive(Clone, Debug, Default)]
pub struct Shutdown(CancellationToken);

impl Shutdown {
    /// A shutdown handle that is not triggered.
    #[must_use]
    pub fn new() -> Self {
        Self(CancellationToken::new())
    }

    /// Cancels the token, releasing every waiter. Idempotent.
    pub fn trigger(&self) {
        self.0.cancel();
    }

    /// Whether [`Shutdown::trigger`] has been called on any clone.
    #[must_use]
    pub fn is_triggered(&self) -> bool {
        self.0.is_cancelled()
    }

    /// Resolves once triggered, immediately if it already was.
    pub async fn triggered(&self) {
        self.0.cancelled().await;
    }

    /// Installs OS-signal handlers that trigger the shutdown on `SIGTERM`
    /// (unix) or `Ctrl-C`, whichever arrives first.
    ///
    /// # Errors
    ///
    /// Returns the OS error when a signal handler cannot be registered.
    ///
    /// # Panics
    ///
    /// Panics when called outside a Tokio runtime.
    pub fn listen_for_signals(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            let mut sigterm = signal(SignalKind::terminate())?;
            let mut sigint = signal(SignalKind::interrupt())?;
            let shutdown = self.clone();
            tokio::spawn(async move {
                tokio::select! {
                    _ = sigterm.recv() => {
                        tracing::info!(signal = "SIGTERM", "shutdown signal received");
                        shutdown.trigger();
                    }
                    _ = sigint.recv() => {
                        tracing::info!(signal = "SIGINT", "shutdown signal received");
                        shutdown.trigger();
                    }
                }
            });
        }

        #[cfg(not(unix))]
        {
            let shutdown = self.clone();
            tokio::spawn(async move {
                match tokio::signal::ctrl_c().await {
                    Ok(()) => {
                        tracing::info!(signal = "SIGINT", "shutdown signal received");
                        shutdown.trigger();
                    }
                    Err(error) => {
                        tracing::error!(%error, "failed to await the shutdown signal");
                    }
                }
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::timeout;

    use super::Shutdown;

    #[test]
    fn new_is_not_triggered() {
        assert!(!Shutdown::new().is_triggered());
    }

    #[test]
    fn trigger_flips_is_triggered() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        assert!(shutdown.is_triggered());
    }

    #[test]
    fn clones_share_the_trigger() {
        let shutdown = Shutdown::new();
        shutdown.clone().trigger();
        assert!(shutdown.is_triggered());
    }

    #[tokio::test(start_paused = true)]
    async fn triggered_resolves_after_trigger() {
        let shutdown = Shutdown::new();
        let waiter = shutdown.clone();
        let handle = tokio::spawn(async move { waiter.triggered().await });

        shutdown.trigger();

        assert!(timeout(Duration::from_secs(1), handle).await.is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn triggered_resolves_immediately_when_already_triggered() {
        let shutdown = Shutdown::new();
        shutdown.trigger();

        assert!(
            timeout(Duration::from_secs(1), shutdown.triggered())
                .await
                .is_ok()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn triggered_pends_until_trigger() {
        let shutdown = Shutdown::new();

        assert!(
            timeout(Duration::from_secs(60), shutdown.triggered())
                .await
                .is_err()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn listen_for_signals_installs_without_triggering() {
        let shutdown = Shutdown::new();

        shutdown.listen_for_signals().expect("signal registration");

        assert!(!shutdown.is_triggered());
    }
}
