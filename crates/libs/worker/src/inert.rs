use async_trait::async_trait;

use runtime::run_error::RunError;

use crate::worker::{Worker, WorkerContext};

/// A placeholder worker that does no work: it heartbeats on schedule and
/// returns once shutdown is triggered.
#[derive(Clone, Copy, Debug, Default)]
pub struct InertWorker;

#[async_trait]
impl Worker for InertWorker {
    fn name(&self) -> &'static str {
        "inert"
    }

    async fn run(&self, context: WorkerContext) -> Result<(), RunError> {
        let mut ticker = tokio::time::interval(context.heartbeat().interval());
        loop {
            tokio::select! {
                () = context.shutdown().triggered() => return Ok(()),
                _ = ticker.tick() => context.heartbeat().beat(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::timeout;

    use runtime::heartbeat::Heartbeat;
    use runtime::shutdown::Shutdown;

    use crate::worker::{Worker, WorkerContext};

    use super::InertWorker;

    #[tokio::test(start_paused = true)]
    async fn returns_ok_once_shutdown_is_triggered() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        let context = WorkerContext::new(shutdown, heartbeat);

        let result = timeout(Duration::from_secs(1), async {
            InertWorker.run(context).await
        })
        .await;

        assert!(result.is_ok(), "the worker returns within 1s");
        assert!(result.expect("completed in time").is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn keeps_the_heartbeat_fresh() {
        let shutdown = Shutdown::new();
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        let context = WorkerContext::new(shutdown.clone(), heartbeat.clone());

        let handle = tokio::spawn(async move { InertWorker.run(context).await });

        tokio::time::sleep(Duration::from_secs(120)).await;
        assert!(heartbeat.check().is_ok(), "the worker keeps beating");

        shutdown.trigger();
        let joined = timeout(Duration::from_secs(1), handle)
            .await
            .expect("the worker stops once shutdown is triggered");
        assert!(joined.is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn pends_while_not_shut_down() {
        let shutdown = Shutdown::new();
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        let context = WorkerContext::new(shutdown, heartbeat);

        let result = timeout(Duration::from_secs(300), async {
            InertWorker.run(context).await
        })
        .await;

        assert!(result.is_err(), "the worker pends until shutdown");
    }
}
