use async_trait::async_trait;

use runtime::heartbeat::Heartbeat;
use runtime::run_error::RunError;
use runtime::shutdown::Shutdown;

/// The shutdown handle and heartbeat handed to a [`Worker::run`].
///
/// Clones share the underlying [`Shutdown`] and [`Heartbeat`] state.
#[derive(Clone, Debug)]
pub struct WorkerContext {
    shutdown: Shutdown,
    heartbeat: Heartbeat,
}

impl WorkerContext {
    /// Creates a context from the process shutdown handle and the worker
    /// heartbeat.
    #[must_use]
    pub fn new(shutdown: Shutdown, heartbeat: Heartbeat) -> Self {
        Self {
            shutdown,
            heartbeat,
        }
    }

    /// The process shutdown, triggered by an OS signal or supervision.
    #[must_use]
    pub fn shutdown(&self) -> &Shutdown {
        &self.shutdown
    }

    /// The liveness heartbeat the worker must keep fresh.
    #[must_use]
    pub fn heartbeat(&self) -> &Heartbeat {
        &self.heartbeat
    }
}

/// A long-running unit of work hosted by a worker process. [`Worker::run`]
/// must keep calling `context.heartbeat().beat()` at least every
/// `context.heartbeat().interval()`, and must return once
/// `context.shutdown()` is triggered. Returning before shutdown is treated as
/// a failure of the process.
#[async_trait]
pub trait Worker: Send + Sync {
    /// Short static name used in logs.
    fn name(&self) -> &'static str;

    /// Runs the worker until it fails or shutdown is triggered.
    ///
    /// # Errors
    ///
    /// Returns a [`runtime::run_error::RunError`] classifying why the worker
    /// could not continue.
    async fn run(&self, context: WorkerContext) -> Result<(), RunError>;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::advance;

    use runtime::heartbeat::Heartbeat;
    use runtime::shutdown::Shutdown;

    use super::WorkerContext;

    #[tokio::test(start_paused = true)]
    async fn context_exposes_its_parts() {
        let shutdown = Shutdown::new();
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        let context = WorkerContext::new(shutdown.clone(), heartbeat.clone());

        context.shutdown().trigger();
        assert!(shutdown.is_triggered());

        advance(Duration::from_secs(31)).await;
        context.heartbeat().beat();
        assert!(heartbeat.check().is_ok());
    }
}
