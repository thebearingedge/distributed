use async_trait::async_trait;

use runtime::run_error::RunError;
use runtime::shutdown::Shutdown;

/// A run-to-completion unit of work hosted by a Kubernetes Job process.
///
/// [`run`](Job::run) should observe `shutdown` and return promptly once it is
/// triggered; an error returned after shutdown is reported as interrupted.
/// Jobs should be idempotent, because a retried Job runs again from the start.
#[async_trait]
pub trait Job: Send + Sync {
    /// Short static name used in logs.
    fn name(&self) -> &'static str;

    /// Runs the job until it finishes or shutdown is triggered.
    ///
    /// # Errors
    ///
    /// Returns a [`runtime::run_error::RunError`] classifying the failure as
    /// retryable or permanent.
    async fn run(&self, shutdown: Shutdown) -> Result<(), RunError>;
}
