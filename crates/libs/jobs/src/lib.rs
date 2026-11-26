//! Composable runtime for run-to-completion Kubernetes Job processes: the
//! [`crate::Job`] contract, the [`crate::runner::run()`] and
//! [`crate::runner::run_within()`] functions that map outcomes to exit
//! statuses, the [`crate::launch::launch()`] composition root, and the
//! placeholder [`crate::NoopJob`].

/// The job contract.
pub mod job;

/// The job process composition root.
pub mod launch;

/// The placeholder no-op job.
pub mod noop;

/// Runs a job and maps its outcome to an exit status, either unbounded
/// ([`crate::runner::run()`]) or bounded by the shutdown deadline
/// ([`crate::runner::run_within()`]).
pub mod runner;

pub use job::Job;
pub use launch::launch;
pub use noop::NoopJob;
pub use runner::{run, run_within};
