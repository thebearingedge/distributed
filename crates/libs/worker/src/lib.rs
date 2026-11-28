//! Composable runtime for long-running Kubernetes worker processes: the
//! [`crate::Worker`] contract, its [`crate::WorkerConfig`], and the
//! [`crate::launch()`] composition root, which serves heartbeat-backed
//! liveness and shutdown-aware readiness probes and supervises the worker and
//! its probe server through [`web::launch_with()`].

/// Worker process configuration.
pub mod config;

/// The worker process composition root.
pub mod launch;

// The worker's heartbeat liveness check for its probe router.
mod probes;

/// The worker contract and its run context.
pub mod worker;

pub use config::WorkerConfig;
pub use launch::launch;
pub use worker::{Worker, WorkerContext};
