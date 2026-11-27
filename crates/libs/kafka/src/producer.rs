//! Typed JSON producer with idempotent delivery.

use std::{fmt, marker::PhantomData, time::Duration};

use rdkafka::{
    producer::{FutureProducer, FutureRecord, Producer, future_producer::Delivery},
    util::Timeout,
};
use serde::Serialize;
use tokio::time::Instant;
use tracing::Instrument;

use crate::config::{KafkaConfig, KafkaProducerConfig};
use crate::headers;
use crate::kafka_error::{KafkaClientError, SendError};
use crate::metrics::{ProducerMetrics, SendInstruments};

/// Partition and offset a message was written to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ack {
    /// The partition the message was written to.
    pub partition: i32,
    /// The offset the message got in its partition.
    pub offset: i64,
}

/// A typed JSON producer for one topic.
///
/// Every [`send`](KafkaProducer::send) waits for the broker's delivery
/// acknowledgement, bounded by `KAFKA_PRODUCER_DELIVERY_TIMEOUT_SECONDS`.
/// Values are serialized as JSON, and W3C trace context from the current
/// span is injected into the message headers.
pub struct KafkaProducer<T> {
    inner: FutureProducer,
    topic: String,
    flush_timeout: Duration,
    metrics: SendInstruments,
    _marker: PhantomData<fn(&T)>,
}

impl<T> fmt::Debug for KafkaProducer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KafkaProducer")
            .field("topic", &self.topic)
            .finish_non_exhaustive()
    }
}

impl<T: Serialize> KafkaProducer<T> {
    /// Creates the producer client for `topic`. Sends are recorded on
    /// `metrics`.
    ///
    /// # Errors
    ///
    /// [`KafkaClientError::Config`] when librdkafka rejects the
    /// configuration.
    pub fn new(
        kafka: &KafkaConfig,
        producer: &KafkaProducerConfig,
        topic: impl Into<String>,
        metrics: &ProducerMetrics,
    ) -> Result<Self, KafkaClientError> {
        let topic = topic.into();
        let mut config = kafka.client_config();
        producer.apply(&mut config);
        let inner = config
            .create::<FutureProducer>()
            .map_err(KafkaClientError::Config)?;
        Ok(Self {
            inner,
            metrics: metrics.instruments(&topic),
            topic,
            flush_timeout: producer.flush_timeout(),
            _marker: PhantomData,
        })
    }

    /// Serializes `value` as JSON and produces it to the topic, waiting for
    /// the broker's delivery acknowledgement, bounded by
    /// `KAFKA_PRODUCER_DELIVERY_TIMEOUT_SECONDS`.
    ///
    /// Every attempt, including one that fails to encode, is recorded on
    /// `messaging.client.sent.messages` and
    /// `messaging.client.operation.duration`.
    ///
    /// # Errors
    ///
    /// [`SendError::Encode`] when the value cannot be serialized, or
    /// [`SendError::Delivery`] when the broker rejects or fails to deliver
    /// the message.
    pub async fn send(&self, key: Option<&[u8]>, value: &T) -> Result<Ack, SendError> {
        let started = Instant::now();
        let outcome = self.deliver(key, value).await;
        self.metrics.record(started.elapsed(), outcome.as_ref());
        outcome
    }

    async fn deliver(&self, key: Option<&[u8]>, value: &T) -> Result<Ack, SendError> {
        let payload = serde_json::to_vec(value)?;

        let span = tracing::info_span!(
            "kafka.send",
            otel.kind = "producer",
            messaging.system.name = "kafka",
            messaging.operation.type = "send",
            messaging.destination.name = %self.topic,
        );
        let trace_headers = span.in_scope(headers::current_trace_headers);

        let mut record = FutureRecord::<[u8], [u8]>::to(&self.topic)
            .payload(&payload)
            .headers(trace_headers);
        if let Some(key) = key {
            record = record.key(key);
        }

        self.inner
            .send(record, Timeout::Never)
            .instrument(span)
            .await
            .map(
                |Delivery {
                     partition, offset, ..
                 }| Ack { partition, offset },
            )
            .map_err(|(error, _message)| SendError::Delivery(error))
    }

    /// Waits for all in-flight deliveries to complete, bounded by
    /// `KAFKA_PRODUCER_FLUSH_TIMEOUT_SECONDS`.
    ///
    /// # Errors
    ///
    /// [`SendError::Flush`] when the flush fails or times out, or
    /// [`SendError::FlushTask`] when the blocking flush task panics or is
    /// cancelled.
    pub async fn flush(&self) -> Result<(), SendError> {
        let inner = self.inner.clone();
        let timeout = self.flush_timeout;
        tokio::task::spawn_blocking(move || inner.flush(Timeout::After(timeout)))
            .await
            .map_err(SendError::FlushTask)?
            .map_err(SendError::Flush)
    }
}
