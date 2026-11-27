//! Composable runtime for run-to-completion Kubernetes Job processes: the
//! [`crate::Job`] contract and the [`crate::JobContext`] it runs with, the
//! [`crate::runner::run_within()`] function that maps outcomes to exit
//! statuses, and the [`crate::launch::launch()`] composition root.

/// The job contract.
pub mod job;

/// The job process composition root.
pub mod launch;

/// Runs a job and maps its outcome to an exit status, either unbounded or
/// bounded by the shutdown deadline ([`crate::runner::run_within()`]).
pub mod runner;

pub use job::{Job, JobContext};
pub use launch::launch;
pub use runner::run_within;
