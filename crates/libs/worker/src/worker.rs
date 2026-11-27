use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use opentelemetry::metrics::MeterProvider;

use runtime::heartbeat::Heartbeat;
use runtime::run_error::RunError;
use runtime::shutdown::Shutdown;

/// The shutdown handle, heartbeat, and meter provider handed to a
/// [`Worker::run`].
///
/// Clones share the underlying [`Shutdown`] and [`Heartbeat`] state and meter
/// provider.
#[derive(Clone)]
pub struct WorkerContext {
    shutdown: Shutdown,
    heartbeat: Heartbeat,
    meter_provider: Arc<dyn MeterProvider + Send + Sync>,
}

impl WorkerContext {
    /// Creates a context from the process shutdown handle, the worker
    /// heartbeat, and the meter provider for the worker's instrumentation.
    #[must_use]
    pub fn new(
        shutdown: Shutdown,
        heartbeat: Heartbeat,
        meter_provider: Arc<dyn MeterProvider + Send + Sync>,
    ) -> Self {
        Self {
            shutdown,
            heartbeat,
            meter_provider,
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

    /// The meter provider to pass to instrumentation such as
    /// `kafka::metrics`.
    #[must_use]
    pub fn meter_provider(&self) -> &(dyn MeterProvider + Send + Sync) {
        self.meter_provider.as_ref()
    }
}

impl fmt::Debug for WorkerContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkerContext")
            .field("shutdown", &self.shutdown)
            .field("heartbeat", &self.heartbeat)
            .finish_non_exhaustive()
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
    use std::sync::Arc;
    use std::time::Duration;

    use opentelemetry::metrics::NoopMeterProvider;
    use tokio::time::advance;

    use runtime::heartbeat::Heartbeat;
    use runtime::shutdown::Shutdown;

    use super::WorkerContext;

    #[tokio::test(start_paused = true)]
    async fn context_exposes_its_parts() {
        let shutdown = Shutdown::new();
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        let context = WorkerContext::new(
            shutdown.clone(),
            heartbeat.clone(),
            Arc::new(NoopMeterProvider::new()),
        );

        context.shutdown().trigger();
        assert!(shutdown.is_triggered());

        advance(Duration::from_secs(31)).await;
        context.heartbeat().beat();
        assert!(heartbeat.check().is_ok());

        let counter = context
            .meter_provider()
            .meter("test")
            .u64_counter("test.count")
            .build();
        counter.add(1, &[]);
    }

    #[test]
    fn context_debug_omits_the_provider() {
        let context = WorkerContext::new(
            Shutdown::new(),
            Heartbeat::new(Duration::from_secs(30)),
            Arc::new(NoopMeterProvider::new()),
        );

        let debug = format!("{context:?}");

        assert!(debug.starts_with("WorkerContext {"), "{debug}");
        assert!(debug.contains(".."), "{debug}");
    }
}
