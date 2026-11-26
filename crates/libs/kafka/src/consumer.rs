//! At-least-once JSON consumption with heartbeat and shutdown integration.

use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use rdkafka::Message;
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::error::{KafkaError, KafkaResult, RDKafkaErrorCode};
use rdkafka::message::BorrowedMessage;
use serde::de::DeserializeOwned;
use tracing::Instrument;

use runtime::heartbeat::Heartbeat;
use runtime::run_error::RunError;
use runtime::shutdown::Shutdown;

use crate::config::{KafkaConfig, KafkaConsumerConfig};
use crate::headers;
use crate::kafka_error::{ConsumeError, DecodeError, KafkaClientError, is_fatal};

/// Partition and offset a delivered message came from.
#[derive(Clone, Debug)]
pub struct DeliveryMeta {
    /// The partition the message was read from.
    pub partition: i32,
    /// The offset of the message within the partition.
    pub offset: i64,
    /// The message timestamp in milliseconds since the epoch, when the
    /// broker carries one.
    pub timestamp_millis: Option<i64>,
    /// The message key, when the producer set one.
    pub key: Option<Vec<u8>>,
}

/// A decoded message and its metadata.
#[derive(Debug)]
pub struct Delivery<T> {
    /// The message payload decoded into `T`.
    pub value: T,
    /// Where the message came from.
    pub meta: DeliveryMeta,
}

/// Processes decoded messages; returning an error stops consumption without
/// storing the message's offset, so it is redelivered.
#[async_trait]
pub trait MessageHandler<T>: Send + Sync {
    /// Handles one decoded message.
    ///
    /// # Errors
    ///
    /// A [`RunError`] classifying why the message could not be handled.
    async fn handle(&self, delivery: Delivery<T>) -> Result<(), RunError>;
}

/// A typed JSON consumer over one topic, delivering one message at a time.
///
/// [`run`](Self::run) gives at-least-once delivery: a message's offset is
/// stored only after its handler succeeds, and the final commit happens when
/// the consumer closes. Payloads that are empty or not valid JSON for `T` are
/// logged, acked, and skipped. A handler failure stops consumption without
/// storing the failing message's offset, so it is redelivered. A fatal
/// librdkafka error stops consumption; recreate the client to continue.
pub struct KafkaConsumer<T> {
    inner: Arc<StreamConsumer>,
    topic: String,
    _marker: PhantomData<fn() -> T>,
}

impl<T> std::fmt::Debug for KafkaConsumer<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KafkaConsumer")
            .field("topic", &self.topic)
            .finish_non_exhaustive()
    }
}

impl<T: DeserializeOwned + Send> KafkaConsumer<T> {
    /// Creates the stream consumer and subscribes it to `topic`.
    ///
    /// # Errors
    ///
    /// [`KafkaClientError::Config`] when librdkafka rejects the client
    /// configuration, and [`KafkaClientError::Subscribe`] when the
    /// subscription fails.
    ///
    /// # Panics
    ///
    /// Panics when called outside a Tokio runtime, because the stream
    /// consumer spawns its wake-up loop there.
    pub fn new(
        kafka: &KafkaConfig,
        consumer: &KafkaConsumerConfig,
        topic: impl Into<String>,
    ) -> Result<Self, KafkaClientError> {
        let topic = topic.into();
        let mut config = kafka.client_config();
        consumer.apply(&mut config);
        let created: StreamConsumer = config.create().map_err(KafkaClientError::Config)?;
        created
            .subscribe(&[topic.as_str()])
            .map_err(|source| KafkaClientError::Subscribe {
                topic: topic.clone(),
                source,
            })?;
        Ok(Self {
            inner: Arc::new(created),
            topic,
            _marker: PhantomData,
        })
    }

    /// Consumes until `shutdown` is triggered, a handler fails, or a fatal
    /// librdkafka error occurs, then commits and leaves the group.
    ///
    /// Messages are handled one at a time, giving at-least-once delivery:
    /// each message's offset is stored only after its handler succeeds, and
    /// the final commit runs when the consumer closes. Empty or undecodable
    /// payloads are logged, acked, and skipped. A handler error stops
    /// consumption without storing the failing message's offset, so the
    /// message is redelivered on the next run. A fatal client error stops
    /// consumption; its [`ConsumeError`] classifies as a retryable
    /// [`RunError`], the handler error as the handler's own classification.
    /// The heartbeat is beaten on every loop iteration, on idle ticks, and
    /// while the handler runs.
    ///
    /// # Errors
    ///
    /// [`ConsumeError::Handler`] when the handler fails,
    /// [`ConsumeError::Fatal`] on a fatal librdkafka error,
    /// [`ConsumeError::Commit`] when the final offset commit fails, and
    /// [`ConsumeError::CloseTask`] when the blocking close task panics or is
    /// cancelled.
    pub async fn run<H: MessageHandler<T> + ?Sized>(
        self,
        handler: &H,
        shutdown: &Shutdown,
        heartbeat: &Heartbeat,
    ) -> Result<(), ConsumeError> {
        let mut idle = tokio::time::interval(heartbeat.interval());
        let outcome: Result<(), ConsumeError> = loop {
            tokio::select! {
                biased;
                () = shutdown.triggered() => break Ok(()),
                _ = idle.tick() => heartbeat.beat(),
                received = self.inner.recv() => {
                    heartbeat.beat();
                    match received {
                        Err(error) if is_fatal(&error) => break Err(ConsumeError::Fatal(error)),
                        Err(error) => {
                            tracing::warn!(%error, "kafka consumer error");
                        }
                        Ok(message) => {
                            if let Err(error) = self.process(message, handler, heartbeat).await {
                                break Err(error);
                            }
                        }
                    }
                }
            }
        };
        outcome.and(close(self.inner).await)
    }

    /// Decodes one received message into owned data, hands it to `handler`,
    /// and stores its offset when the handler succeeds. Undecodable payloads
    /// are logged, acked, and skipped.
    async fn process<H: MessageHandler<T> + ?Sized>(
        &self,
        message: BorrowedMessage<'_>,
        handler: &H,
        heartbeat: &Heartbeat,
    ) -> Result<(), ConsumeError> {
        let partition = message.partition();
        let offset = message.offset();
        let meta = DeliveryMeta {
            partition,
            offset,
            timestamp_millis: message.timestamp().to_millis(),
            key: message.key().map(Vec::from),
        };
        let span = tracing::info_span!(
            "kafka.process",
            otel.kind = "consumer",
            messaging.system = "kafka",
            messaging.operation.type = "process",
            messaging.destination.name = %self.topic,
            messaging.destination.partition.id = partition,
            messaging.kafka.offset = offset,
        );
        headers::link_parent(&span, message.headers());
        let decoded = decode::<T>(message.payload());
        drop(message);
        match decoded {
            Ok(value) => {
                let delivery = Delivery {
                    value,
                    meta: meta.clone(),
                };
                if let Err(source) = heartbeat
                    .beat_while(handler.handle(delivery))
                    .instrument(span)
                    .await
                {
                    return Err(ConsumeError::Handler {
                        partition,
                        offset,
                        source,
                    });
                }
            }
            Err(DecodeError::EmptyPayload) => {
                tracing::debug!(partition, offset, "skipping empty payload");
            }
            Err(error @ DecodeError::Json(_)) => {
                tracing::warn!(partition, offset, %error, "skipping undecodable message");
            }
        }
        self.store_or_warn(&meta);
        Ok(())
    }

    /// Stores the next offset to resume from, so the message at `meta.offset`
    /// is never redelivered; librdkafka adds one itself. A failure is logged
    /// at warn, which leaves the message to be redelivered.
    fn store_or_warn(&self, meta: &DeliveryMeta) {
        if let Err(error) = self
            .inner
            .store_offset(&self.topic, meta.partition, meta.offset)
        {
            tracing::warn!(
                partition = meta.partition,
                offset = meta.offset,
                %error,
                "failed to store the consumed offset"
            );
        }
    }
}

/// Decodes a message payload into `T`; `None` is an empty payload.
fn decode<T: DeserializeOwned>(payload: Option<&[u8]>) -> Result<T, DecodeError> {
    match payload {
        None => Err(DecodeError::EmptyPayload),
        Some(bytes) => serde_json::from_slice(bytes).map_err(DecodeError::Json),
    }
}

/// Commits the stored offsets and drops the consumer, which leaves the
/// group, on a blocking thread.
///
/// # Errors
///
/// [`ConsumeError::Commit`] when the final commit fails, and
/// [`ConsumeError::CloseTask`] when the blocking task panics or is cancelled.
async fn close(consumer: Arc<StreamConsumer>) -> Result<(), ConsumeError> {
    match tokio::task::spawn_blocking(move || {
        let result = commit_on_shutdown(&consumer);
        drop(consumer);
        result
    })
    .await
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(ConsumeError::Commit(error)),
        Err(error) => Err(ConsumeError::CloseTask(error)),
    }
}

/// Commits the consumer's stored offsets synchronously, tolerating the
/// nothing-to-commit and assignment-lost outcomes.
fn commit_on_shutdown(consumer: &StreamConsumer) -> KafkaResult<()> {
    match consumer.commit_consumer_state(CommitMode::Sync) {
        Ok(()) | Err(KafkaError::ConsumerCommit(RDKafkaErrorCode::NoOffset)) => Ok(()),
        Err(KafkaError::ConsumerCommit(RDKafkaErrorCode::AssignmentLost)) => {
            tracing::warn!("assignment lost before final commit; offsets will be redelivered");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use async_trait::async_trait;
    use clap::Parser;
    use serde::Deserialize;
    use serde_json::Value;

    use runtime::heartbeat::Heartbeat;
    use runtime::shutdown::Shutdown;

    use super::*;

    #[derive(Deserialize, Debug, PartialEq)]
    struct Payload {
        seq: u32,
    }

    #[test]
    fn decode_rejects_empty_payloads() {
        assert!(matches!(
            decode::<Payload>(None),
            Err(DecodeError::EmptyPayload)
        ));
    }

    #[test]
    fn decode_rejects_invalid_json() {
        let error = decode::<Payload>(Some(b"not json")).expect_err("invalid json is rejected");
        assert!(matches!(error, DecodeError::Json(_)));
    }

    #[test]
    fn decode_parses_json() {
        assert_eq!(
            decode::<Payload>(Some(br#"{"seq": 7}"#)).expect("valid json decodes"),
            Payload { seq: 7 }
        );
    }

    struct NopHandler;

    #[async_trait]
    impl MessageHandler<Value> for NopHandler {
        async fn handle(&self, _delivery: Delivery<Value>) -> Result<(), RunError> {
            Ok(())
        }
    }

    #[test]
    fn run_future_is_send() {
        fn assert_send<F: Send>(_: F) {}

        let consumer_run = |kafka: &KafkaConfig, group: &str, topic: &str| {
            let consumer =
                KafkaConsumerConfig::try_parse_from(["test", "--kafka-consumer-group-id", group])
                    .expect("consumer config parses");
            let consumer: KafkaConsumer<Value> =
                KafkaConsumer::new(kafka, &consumer, topic).expect("consumer builds");
            let shutdown = Shutdown::new();
            let heartbeat = Heartbeat::new(Duration::from_secs(1));
            assert_send(consumer.run(&NopHandler, &shutdown, &heartbeat));
        };
        if false {
            let kafka = KafkaConfig::try_parse_from(["test", "--kafka-bootstrap-servers", "h:1"])
                .expect("kafka config parses");
            consumer_run(&kafka, "group", "topic");
        }
    }
}
