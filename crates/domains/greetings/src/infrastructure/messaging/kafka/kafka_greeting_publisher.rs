use async_trait::async_trait;
use kafka::config::{KafkaConfig, KafkaProducerConfig};
use kafka::kafka_error::{KafkaClientError, SendError};
use kafka::metrics::ProducerMetrics;
use kafka::producer::KafkaProducer;
use messaging::MessagingError;

use super::GREETINGS_TOPIC;
use super::kafka_greeting_message::KafkaGreetingMessage;
use crate::models::greeting::GreetingModel;
use crate::use_cases::greeting_publisher::GreetingPublisher as Interface;

/// Kafka implementation of the
/// [`GreetingPublisher`](crate::use_cases::greeting_publisher::GreetingPublisher)
/// port.
#[derive(Debug)]
pub struct KafkaGreetingPublisher {
    producer: KafkaProducer<KafkaGreetingMessage>,
}

impl KafkaGreetingPublisher {
    /// Creates the producer client for [`GREETINGS_TOPIC`]. Sends are
    /// recorded on `metrics`.
    ///
    /// # Errors
    ///
    /// Returns a [`KafkaClientError`] when librdkafka rejects the client
    /// configuration.
    pub fn connect(
        kafka: &KafkaConfig,
        producer: &KafkaProducerConfig,
        metrics: &ProducerMetrics,
    ) -> Result<Self, KafkaClientError> {
        Ok(Self {
            producer: KafkaProducer::new(kafka, producer, GREETINGS_TOPIC, metrics)?,
        })
    }

    /// Waits for all in-flight deliveries to complete, bounded by
    /// `KAFKA_PRODUCER_FLUSH_TIMEOUT_SECONDS`.
    ///
    /// # Errors
    ///
    /// Returns a [`SendError`] when the flush fails or times out.
    pub async fn flush(&self) -> Result<(), SendError> {
        self.producer.flush().await
    }
}

#[async_trait]
impl Interface for KafkaGreetingPublisher {
    async fn publish(&self, greeting: &GreetingModel) -> Result<(), MessagingError> {
        self.producer
            .send(
                Some(greeting.id.as_bytes()),
                &KafkaGreetingMessage::from(greeting),
            )
            .await
            .map(|_ack| ())
            .map_err(MessagingError::from)
    }
}
