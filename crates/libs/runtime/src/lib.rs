//! Process lifecycle primitives shared by every deployable shape (API,
//! worker, job): the shared lifecycle configuration, the cancellation-token
//! shutdown handle and OS-signal installation, shutdown-deadline enforcement,
//! named task supervision, the bounded final flush and exit, the
//! Kubernetes-oriented exit-status taxonomy, classified run errors, and the
//! worker liveness heartbeat.

/// Process lifecycle configuration: drain, shutdown deadline, and final
/// flush budget (`crate::config::LifecycleConfig`).
pub mod config;

/// Shutdown-deadline enforcement for futures
/// (`crate::deadline::within_deadline`).
pub mod deadline;

/// Process exit statuses aligned with Kubernetes `podFailurePolicy`
/// (`crate::exit_status::ExitStatus`).
pub mod exit_status;

/// Liveness heartbeat for long-running loops (`crate::heartbeat::Heartbeat`).
pub mod heartbeat;

/// Run outcomes, the bounded final flush, and process exit
/// (`crate::lifecycle::finish`).
pub mod lifecycle;

/// Retryable/permanent run-failure classification
/// (`crate::run_error::RunError`).
pub mod run_error;

/// Cancellation-token shutdown handle and OS-signal installation
/// (`crate::shutdown::Shutdown`).
pub mod shutdown;

/// Named critical-task supervision (`crate::supervisor::Supervisor`).
pub mod supervisor;
