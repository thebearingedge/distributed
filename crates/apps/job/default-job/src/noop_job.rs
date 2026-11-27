use async_trait::async_trait;

use jobs::{Job, JobContext};
use runtime::run_error::RunError;

/// A placeholder job that does nothing and succeeds.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopJob;

#[async_trait]
impl Job for NoopJob {
    fn name(&self) -> &'static str {
        "noop"
    }

    async fn run(&self, _context: JobContext) -> Result<(), RunError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use opentelemetry::metrics::NoopMeterProvider;

    use super::NoopJob;
    use jobs::{Job, JobContext};
    use runtime::shutdown::Shutdown;

    #[tokio::test]
    async fn succeeds() {
        let context = JobContext::new(Shutdown::new(), Arc::new(NoopMeterProvider::new()));

        let result = NoopJob.run(context).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn succeeds_even_when_shutdown_is_triggered() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        let context = JobContext::new(shutdown, Arc::new(NoopMeterProvider::new()));

        let result = NoopJob.run(context).await;

        assert!(result.is_ok());
    }

    #[test]
    fn is_named_noop() {
        assert_eq!(NoopJob.name(), "noop");
    }
}
