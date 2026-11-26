use std::io;

#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::exit_status::ExitStatus;

/// An OS signal that requests shutdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShutdownSignal {
    /// `SIGTERM`.
    Terminate,
    /// `SIGINT` / Ctrl-C.
    Interrupt,
}

impl ShutdownSignal {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Terminate => "SIGTERM",
            Self::Interrupt => "SIGINT",
        }
    }
}

/// Capacity of the channel forwarding OS signals to the watcher.
const SHUTDOWN_SIGNAL_BUFFER: usize = 2;

/// Triggers `shutdown` on the first received signal, and calls `force_exit`
/// on the second. Returns once `signals` closes or after `force_exit`
/// returns. A programmatic [`Shutdown::trigger`] does not count as a signal.
async fn watch_signals<X>(
    shutdown: Shutdown,
    mut signals: mpsc::Receiver<ShutdownSignal>,
    force_exit: X,
) where
    X: FnOnce(ShutdownSignal),
{
    let Some(first) = signals.recv().await else {
        return;
    };
    tracing::info!(signal = first.as_str(), "shutdown signal received");
    shutdown.trigger();
    let Some(second) = signals.recv().await else {
        return;
    };
    tracing::warn!(
        signal = second.as_str(),
        "second shutdown signal received, exiting immediately"
    );
    force_exit(second);
}

/// Ends the process at once with
/// [`crate::exit_status::ExitStatus::Interrupted`].
fn exit_interrupted(_signal: ShutdownSignal) {
    std::process::exit(i32::from(ExitStatus::Interrupted.code()));
}

/// Process-wide shutdown handle: cloned into every task that must stop on
/// shutdown; triggered by an OS signal (see
/// [`Shutdown::listen_for_signals`]) or programmatically.
///
/// All clones share one cancellation state, so any clone can trigger every
/// other. The first OS signal triggers the shutdown; a second one ends the
/// process immediately, skipping any drain, deadline, or flush (see
/// [`Shutdown::listen_for_signals`]).
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

    /// Installs OS-signal handlers: the first `SIGTERM` (unix) or `Ctrl-C`
    /// triggers the shutdown; a second one ends the process immediately with
    /// exit code 143, skipping any drain, deadline, or flush.
    ///
    /// # Errors
    ///
    /// Returns the OS error when a signal handler cannot be registered.
    ///
    /// # Panics
    ///
    /// Panics when called outside a Tokio runtime.
    pub fn listen_for_signals(&self) -> io::Result<()> {
        let (signal_tx, signal_rx) = mpsc::channel(SHUTDOWN_SIGNAL_BUFFER);

        #[cfg(unix)]
        {
            let mut sigterm = signal(SignalKind::terminate())?;
            let mut sigint = signal(SignalKind::interrupt())?;
            tokio::spawn(async move {
                loop {
                    let received = tokio::select! {
                        Some(()) = sigterm.recv() => ShutdownSignal::Terminate,
                        Some(()) = sigint.recv() => ShutdownSignal::Interrupt,
                        else => break,
                    };
                    if signal_tx.send(received).await.is_err() {
                        break;
                    }
                }
            });
        }

        #[cfg(not(unix))]
        {
            tokio::spawn(async move {
                loop {
                    match tokio::signal::ctrl_c().await {
                        Ok(()) => {
                            if signal_tx.send(ShutdownSignal::Interrupt).await.is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            tracing::error!(%error, "failed to await the shutdown signal");
                            break;
                        }
                    }
                }
            });
        }

        tokio::spawn(watch_signals(self.clone(), signal_rx, exit_interrupted));

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::{mpsc, oneshot};
    use tokio::time::timeout;

    use super::{SHUTDOWN_SIGNAL_BUFFER, Shutdown, ShutdownSignal, watch_signals};

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

    async fn run_watcher(
        shutdown: &Shutdown,
        signals: mpsc::Receiver<ShutdownSignal>,
    ) -> Result<ShutdownSignal, oneshot::error::RecvError> {
        let (force_tx, force_rx) = oneshot::channel();
        timeout(
            Duration::from_secs(1),
            watch_signals(shutdown.clone(), signals, move |signal| {
                let _ = force_tx.send(signal);
            }),
        )
        .await
        .expect("watch_signals completes");
        force_rx.await
    }

    #[tokio::test(start_paused = true)]
    async fn first_signal_triggers_without_forcing() {
        let shutdown = Shutdown::new();
        let (sender, receiver) = mpsc::channel(SHUTDOWN_SIGNAL_BUFFER);

        sender
            .send(ShutdownSignal::Terminate)
            .await
            .expect("receiver is alive");
        drop(sender);

        let forced = run_watcher(&shutdown, receiver).await;

        assert!(shutdown.is_triggered());
        assert!(forced.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn second_signal_forces_exit() {
        let shutdown = Shutdown::new();
        let (sender, receiver) = mpsc::channel(SHUTDOWN_SIGNAL_BUFFER);

        sender
            .send(ShutdownSignal::Terminate)
            .await
            .expect("receiver is alive");
        sender
            .send(ShutdownSignal::Interrupt)
            .await
            .expect("receiver is alive");
        drop(sender);

        let forced = run_watcher(&shutdown, receiver).await;

        assert!(shutdown.is_triggered());
        assert_eq!(forced, Ok(ShutdownSignal::Interrupt));
    }

    #[tokio::test(start_paused = true)]
    async fn closed_channel_returns_without_triggering() {
        let shutdown = Shutdown::new();
        let (_, receiver) = mpsc::channel(SHUTDOWN_SIGNAL_BUFFER);

        let forced = run_watcher(&shutdown, receiver).await;

        assert!(!shutdown.is_triggered());
        assert!(forced.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn programmatic_trigger_does_not_count_as_a_signal() {
        let shutdown = Shutdown::new();
        let (sender, receiver) = mpsc::channel(SHUTDOWN_SIGNAL_BUFFER);

        shutdown.trigger();
        sender
            .send(ShutdownSignal::Terminate)
            .await
            .expect("receiver is alive");
        drop(sender);

        let forced = run_watcher(&shutdown, receiver).await;

        assert!(forced.is_err());
    }
}
