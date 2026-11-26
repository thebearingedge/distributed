//! The greetings consumer worker: builds the Kafka subscriber inside
//! [`Worker::run`] and consumes until shutdown.

use async_trait::async_trait;
use greetings::{KafkaGreetingSubscriber, ReceiveGreetingInteractor};
use kafka::config::{KafkaConfig, KafkaConsumerConfig};
use runtime::run_error::RunError;
use worker::{Worker, WorkerContext};

/// Consumes greetings until shutdown.
#[derive(Debug)]
pub(crate) struct GreetingsConsumerWorker {
    kafka: KafkaConfig,
    consumer: KafkaConsumerConfig,
}

impl GreetingsConsumerWorker {
    /// Creates the worker from the Kafka and consumer configuration.
    #[must_use]
    pub(crate) fn new(kafka: KafkaConfig, consumer: KafkaConsumerConfig) -> Self {
        Self { kafka, consumer }
    }
}

#[async_trait]
impl Worker for GreetingsConsumerWorker {
    fn name(&self) -> &'static str {
        "greetings_consumer"
    }

    async fn run(&self, context: WorkerContext) -> Result<(), RunError> {
        KafkaGreetingSubscriber::connect(&self.kafka, &self.consumer)?
            .run(
                ReceiveGreetingInteractor,
                context.shutdown(),
                context.heartbeat(),
            )
            .await
            .map_err(RunError::from)
    }
}
