use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use opentelemetry::metrics::MeterProvider;

use runtime::run_error::RunError;
use runtime::shutdown::Shutdown;

/// The shutdown handle and meter provider handed to a [`Job::run`].
///
/// Clones share the underlying [`Shutdown`] state and meter provider.
#[derive(Clone)]
pub struct JobContext {
    shutdown: Shutdown,
    meter_provider: Arc<dyn MeterProvider + Send + Sync>,
}

impl JobContext {
    /// Creates a context from the process shutdown handle and the meter
    /// provider for instrumentation.
    #[must_use]
    pub fn new(shutdown: Shutdown, meter_provider: Arc<dyn MeterProvider + Send + Sync>) -> Self {
        Self {
            shutdown,
            meter_provider,
        }
    }

    /// The process shutdown, triggered by an OS signal or the shutdown
    /// deadline machinery.
    #[must_use]
    pub fn shutdown(&self) -> &Shutdown {
        &self.shutdown
    }

    /// The meter provider to pass to instrumentation such as
    /// `kafka::metrics`.
    #[must_use]
    pub fn meter_provider(&self) -> &(dyn MeterProvider + Send + Sync) {
        self.meter_provider.as_ref()
    }
}

impl fmt::Debug for JobContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobContext")
            .field("shutdown", &self.shutdown)
            .finish_non_exhaustive()
    }
}

/// A run-to-completion unit of work hosted by a Kubernetes Job process.
///
/// [`run`](Job::run) should observe `context.shutdown()` and return promptly
/// once it is triggered; an error returned after shutdown is reported as
/// interrupted. Jobs should be idempotent, because a retried Job runs again
/// from the start.
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
    async fn run(&self, context: JobContext) -> Result<(), RunError>;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use opentelemetry::metrics::NoopMeterProvider;

    use super::JobContext;
    use runtime::shutdown::Shutdown;

    #[test]
    fn context_exposes_its_parts() {
        let shutdown = Shutdown::new();
        let context = JobContext::new(shutdown.clone(), Arc::new(NoopMeterProvider::new()));

        context.shutdown().trigger();
        assert!(shutdown.is_triggered());

        let counter = context
            .meter_provider()
            .meter("test")
            .u64_counter("test.count")
            .build();
        counter.add(1, &[]);

        assert!(format!("{context:?}").starts_with("JobContext {"));
    }
}
