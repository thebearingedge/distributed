//! default-worker application library: a placeholder long-running worker
//! that heartbeats on schedule and stops once shutdown is triggered.

/// The placeholder inert worker.
mod inert_worker;

pub use inert_worker::InertWorker;
