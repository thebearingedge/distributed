//! The greetings producer worker: builds the Kafka publisher inside
//! [`Worker::run`], publishes every interval, and flushes on shutdown.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use greetings::{GreetingError, KafkaGreetingPublisher, SendGreetingInteractor};
use kafka::config::{KafkaConfig, KafkaProducerConfig};
use kafka::metrics::ProducerMetrics;
use messaging::MessagingError;
use runtime::run_error::RunError;
use worker::{Worker, WorkerContext};

/// Publishes a greeting every `interval` until shutdown, then flushes.
#[derive(Debug)]
pub struct GreetingsProducerWorker {
    kafka: KafkaConfig,
    producer: KafkaProducerConfig,
    interval: Duration,
    message: String,
}

impl GreetingsProducerWorker {
    /// Creates a worker that publishes `message` every `interval`.
    #[must_use]
    pub fn new(
        kafka: KafkaConfig,
        producer: KafkaProducerConfig,
        interval: Duration,
        message: String,
    ) -> Self {
        Self {
            kafka,
            producer,
            interval,
            message,
        }
    }
}

#[async_trait]
impl Worker for GreetingsProducerWorker {
    fn name(&self) -> &'static str {
        "greetings_producer"
    }

    async fn run(&self, context: WorkerContext) -> Result<(), RunError> {
        let publisher = Arc::new(KafkaGreetingPublisher::connect(
            &self.kafka,
            &self.producer,
            &ProducerMetrics::new(context.meter_provider()),
        )?);
        let sender = SendGreetingInteractor::new(publisher.clone());

        let mut idle = tokio::time::interval(context.heartbeat().interval());
        let mut sends = tokio::time::interval(self.interval);

        loop {
            tokio::select! {
                biased;
                () = context.shutdown().triggered() => break,
                _ = idle.tick() => context.heartbeat().beat(),
                _ = sends.tick() => {
                    context
                        .heartbeat()
                        .beat_while(sender.send(&self.message))
                        .await
                        .map_err(classify)?;
                }
            }
        }

        publisher.flush().await.map_err(RunError::retryable)?;
        Ok(())
    }
}

/// Classifies a greeting failure: a misconfigured message and a rejected
/// delivery are permanent, an unavailable broker is retryable.
fn classify(error: GreetingError) -> RunError {
    match error {
        GreetingError::BlankMessage | GreetingError::Messaging(MessagingError::Rejected(_)) => {
            RunError::permanent(error)
        }
        GreetingError::Messaging(MessagingError::Unavailable(_)) => RunError::retryable(error),
    }
}

#[cfg(test)]
mod tests {
    use runtime::exit_status::ExitStatus;

    use super::classify;
    use greetings::GreetingError;
    use messaging::MessagingError;

    #[test]
    fn blank_message_is_permanent() {
        let error = classify(GreetingError::BlankMessage);
        assert_eq!(error.exit_status(), ExitStatus::Permanent);
    }

    #[test]
    fn a_rejected_message_is_permanent() {
        let error = classify(GreetingError::Messaging(MessagingError::Rejected(
            "boom".into(),
        )));
        assert_eq!(error.exit_status(), ExitStatus::Permanent);
    }

    #[test]
    fn an_unavailable_broker_is_retryable() {
        let error = classify(GreetingError::Messaging(MessagingError::Unavailable(
            "boom".into(),
        )));
        assert_eq!(error.exit_status(), ExitStatus::Retryable);
    }
}
