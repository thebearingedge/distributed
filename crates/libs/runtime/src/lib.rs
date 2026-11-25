//! Process lifecycle primitives shared by every deployable shape (API,
//! worker, job): the cancellation-token shutdown handle and OS-signal
//! installation, the Kubernetes-oriented exit-status taxonomy, classified
//! run errors, and the worker liveness heartbeat.

/// Process exit statuses aligned with Kubernetes `podFailurePolicy`
/// (`crate::exit_status::ExitStatus`).
pub mod exit_status;

/// Liveness heartbeat for long-running loops (`crate::heartbeat::Heartbeat`).
pub mod heartbeat;

/// Retryable/permanent run-failure classification
/// (`crate::run_error::RunError`).
pub mod run_error;

/// Cancellation-token shutdown handle and OS-signal installation
/// (`crate::shutdown::Shutdown`).
pub mod shutdown;
