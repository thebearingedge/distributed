//! Composable runtime for long-running Kubernetes worker processes: the
//! [`crate::Worker`] contract, its [`crate::WorkerConfig`], the
//! [`crate::probes::probes_router()`] HTTP probe server (heartbeat-backed
//! liveness, shutdown-aware readiness), the [`crate::supervisor::supervise()`]
//! worker/server supervision on a [`runtime::supervisor::Supervisor`] with a
//! shutdown deadline, and the [`crate::launch::launch()`] composition root.

/// Worker process configuration.
pub mod config;

/// The worker process composition root.
pub mod launch;

/// The worker's liveness/readiness probe router.
pub mod probes;

/// Joint supervision of the worker and its probe server on a
/// [`runtime::supervisor::Supervisor`], with a shutdown deadline.
pub mod supervisor;

/// The worker contract and its run context.
pub mod worker;

pub use config::WorkerConfig;
pub use launch::launch;
pub use probes::probes_router;
pub use supervisor::supervise;
pub use worker::{Worker, WorkerContext};
