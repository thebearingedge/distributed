use async_trait::async_trait;
use kafka::config::{KafkaConfig, KafkaConsumerConfig};
use kafka::consumer::{Delivery, KafkaConsumer, MessageHandler};
use kafka::kafka_error::{ConsumeError, KafkaClientError};
use runtime::heartbeat::Heartbeat;
use runtime::run_error::RunError;
use runtime::shutdown::Shutdown;

use super::GREETINGS_TOPIC;
use super::kafka_greeting_message::KafkaGreetingMessage;
use crate::models::greeting::GreetingModel;
use crate::use_cases::greeting_error::GreetingError;
use crate::use_cases::receive_greeting_interactor::ReceiveGreetingInteractor;

/// Kafka implementation of the greetings subscriber: consumes
/// [`GREETINGS_TOPIC`] and hands each greeting to the
/// [`ReceiveGreetingInteractor`].
#[derive(Debug)]
pub struct KafkaGreetingSubscriber {
    consumer: KafkaConsumer<KafkaGreetingMessage>,
}

impl KafkaGreetingSubscriber {
    /// Creates the stream consumer and subscribes it to [`GREETINGS_TOPIC`].
    ///
    /// # Errors
    ///
    /// Returns a [`KafkaClientError`] when librdkafka rejects the client
    /// configuration or the subscription fails.
    ///
    /// # Panics
    ///
    /// Panics when called outside a Tokio runtime, because the stream
    /// consumer spawns its wake-up loop there.
    pub fn connect(
        kafka: &KafkaConfig,
        consumer: &KafkaConsumerConfig,
    ) -> Result<Self, KafkaClientError> {
        Ok(Self {
            consumer: KafkaConsumer::new(kafka, consumer, GREETINGS_TOPIC)?,
        })
    }

    /// Consumes greetings and hands each one to `receiver`, until `shutdown`
    /// is triggered, a handler fails, or a fatal librdkafka error occurs.
    ///
    /// Blank greetings are logged, acked, and skipped. A messaging failure
    /// stops consumption without storing the failing message's offset, so it
    /// is redelivered.
    ///
    /// # Errors
    ///
    /// Returns a [`ConsumeError`] classifying why consumption stopped.
    pub async fn run(
        self,
        receiver: ReceiveGreetingInteractor,
        shutdown: &Shutdown,
        heartbeat: &Heartbeat,
    ) -> Result<(), ConsumeError> {
        self.consumer
            .run(&GreetingMessageHandler { receiver }, shutdown, heartbeat)
            .await
    }
}

/// Hands decoded greetings over to the [`ReceiveGreetingInteractor`].
struct GreetingMessageHandler {
    receiver: ReceiveGreetingInteractor,
}

#[async_trait]
impl MessageHandler<KafkaGreetingMessage> for GreetingMessageHandler {
    async fn handle(&self, delivery: Delivery<KafkaGreetingMessage>) -> Result<(), RunError> {
        let greeting = GreetingModel::from(delivery.value);

        match self.receiver.receive(&greeting) {
            Ok(()) => Ok(()),
            Err(GreetingError::BlankMessage) => {
                tracing::warn!(
                    partition = delivery.meta.partition,
                    offset = delivery.meta.offset,
                    "skipping blank greeting"
                );
                Ok(())
            }
            Err(error @ GreetingError::Messaging(_)) => Err(RunError::retryable(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use kafka::consumer::DeliveryMeta;

    use crate::models::greeting::GreetingModel;
    use crate::use_cases::receive_greeting_interactor::ReceiveGreetingInteractor;

    use super::*;

    fn delivery(greeting: &GreetingModel) -> Delivery<KafkaGreetingMessage> {
        Delivery {
            value: KafkaGreetingMessage::from(greeting),
            meta: DeliveryMeta {
                partition: 0,
                offset: 0,
                timestamp_millis: None,
                key: None,
            },
        }
    }

    #[tokio::test]
    async fn handler_accepts_valid_greetings() {
        let handler = GreetingMessageHandler {
            receiver: ReceiveGreetingInteractor,
        };

        let result = handler.handle(delivery(&GreetingModel::new("hello"))).await;

        assert!(result.is_ok(), "result: {result:?}");
    }

    #[tokio::test]
    async fn handler_skips_blank_greetings() {
        let handler = GreetingMessageHandler {
            receiver: ReceiveGreetingInteractor,
        };

        let result = handler.handle(delivery(&GreetingModel::new("   "))).await;

        assert!(result.is_ok(), "result: {result:?}");
    }
}
