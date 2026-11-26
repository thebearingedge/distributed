use async_trait::async_trait;

use crate::job::Job;
use runtime::run_error::RunError;
use runtime::shutdown::Shutdown;

/// A placeholder job that does nothing and succeeds.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopJob;

#[async_trait]
impl Job for NoopJob {
    fn name(&self) -> &'static str {
        "noop"
    }

    async fn run(&self, _shutdown: Shutdown) -> Result<(), RunError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::NoopJob;
    use crate::job::Job;
    use runtime::shutdown::Shutdown;

    #[tokio::test]
    async fn succeeds() {
        let result = NoopJob.run(Shutdown::new()).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn succeeds_even_when_shutdown_is_triggered() {
        let shutdown = Shutdown::new();
        shutdown.trigger();

        let result = NoopJob.run(shutdown).await;

        assert!(result.is_ok());
    }

    #[test]
    fn is_named_noop() {
        assert_eq!(NoopJob.name(), "noop");
    }
}
