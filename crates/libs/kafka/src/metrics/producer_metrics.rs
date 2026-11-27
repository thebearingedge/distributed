use std::fmt;
use std::time::Duration;

use opentelemetry::{
    KeyValue,
    metrics::{Counter, Histogram, MeterProvider},
};

use super::{
    ERROR_TYPE, ERROR_TYPE_OTHER, MESSAGE_UNIT, MESSAGING_OPERATION_NAME, MESSAGING_OPERATION_TYPE,
    destination_attributes, meter, partition_id,
};
use crate::kafka_error::SendError;
use crate::producer::Ack;
use telemetry::metrics::duration_histogram;

const MESSAGING_CLIENT_SENT_MESSAGES: &str = "messaging.client.sent.messages";
const MESSAGING_CLIENT_OPERATION_DURATION: &str = "messaging.client.operation.duration";
const MESSAGING_OPERATION_SEND: &str = "send";
const ERROR_TYPE_ENCODE: &str = "encode";

/// The producer instruments `messaging.client.sent.messages` and
/// `messaging.client.operation.duration`, created under the `kafka`
/// instrumentation scope.
///
/// Build one per meter provider and pass it to every
/// [`crate::producer::KafkaProducer::new`]; clones share the instruments.
#[derive(Clone)]
pub struct ProducerMetrics {
    sent: Counter<u64>,
    duration: Histogram<f64>,
}

impl fmt::Debug for ProducerMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProducerMetrics").finish_non_exhaustive()
    }
}

impl ProducerMetrics {
    /// Creates the producer instruments on `meter_provider`.
    #[must_use]
    pub fn new<P>(meter_provider: &P) -> Self
    where
        P: MeterProvider + ?Sized,
    {
        let meter = meter(meter_provider);
        Self {
            sent: meter
                .u64_counter(MESSAGING_CLIENT_SENT_MESSAGES)
                .with_unit(MESSAGE_UNIT)
                .with_description("Number of messages a producer attempted to send to the broker.")
                .build(),
            duration: duration_histogram(
                &meter,
                MESSAGING_CLIENT_OPERATION_DURATION,
                "Duration of messaging operations initiated by a producer.",
            ),
        }
    }

    /// The instruments bound to the attributes of a producer on `topic`.
    pub(crate) fn instruments(&self, topic: &str) -> SendInstruments {
        let mut attributes = destination_attributes(topic);
        attributes.push(KeyValue::new(
            MESSAGING_OPERATION_NAME,
            MESSAGING_OPERATION_SEND,
        ));
        attributes.push(KeyValue::new(
            MESSAGING_OPERATION_TYPE,
            MESSAGING_OPERATION_SEND,
        ));
        SendInstruments {
            metrics: self.clone(),
            attributes,
        }
    }
}

/// The producer instruments bound to one topic's attributes.
pub(crate) struct SendInstruments {
    metrics: ProducerMetrics,
    attributes: Vec<KeyValue>,
}

impl SendInstruments {
    /// Records one send attempt that took `elapsed`.
    pub(crate) fn record(&self, elapsed: Duration, outcome: Result<&Ack, &SendError>) {
        let mut attributes = self.attributes.clone();
        attributes.push(match outcome {
            Ok(ack) => partition_id(ack.partition),
            Err(error) => send_error_type(error),
        });
        self.metrics.sent.add(1, &attributes);
        self.metrics
            .duration
            .record(elapsed.as_secs_f64(), &attributes);
    }
}

/// The bounded `error.type` attribute of a failed send.
fn send_error_type(error: &SendError) -> KeyValue {
    match error {
        SendError::Encode(_) => KeyValue::new(ERROR_TYPE, ERROR_TYPE_ENCODE),
        SendError::Delivery(source) => source.rdkafka_error_code().map_or_else(
            || KeyValue::new(ERROR_TYPE, ERROR_TYPE_OTHER),
            |code| KeyValue::new(ERROR_TYPE, format!("{code:?}")),
        ),
        SendError::Flush(_) | SendError::FlushTask(_) => {
            KeyValue::new(ERROR_TYPE, ERROR_TYPE_OTHER)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use opentelemetry::Value;
    use rdkafka::error::{KafkaError, RDKafkaErrorCode};

    use super::{
        MESSAGING_CLIENT_OPERATION_DURATION, MESSAGING_CLIENT_SENT_MESSAGES, ProducerMetrics,
    };
    use crate::kafka_error::SendError;
    use crate::producer::Ack;
    use telemetry::metrics::DURATION_HISTOGRAM_BOUNDARIES;
    use telemetry::test_support::MetricsHarness;

    fn record_failure(error: &SendError) -> MetricsHarness {
        let harness = MetricsHarness::default();
        ProducerMetrics::new(harness.provider())
            .instruments("orders")
            .record(Duration::from_millis(5), Err(error));
        harness
    }

    #[test]
    fn successful_sends_record_the_partition() {
        let harness = MetricsHarness::default();
        ProducerMetrics::new(harness.provider())
            .instruments("orders")
            .record(
                Duration::from_millis(5),
                Ok(&Ack {
                    partition: 2,
                    offset: 0,
                }),
            );

        let sent = harness.u64_sum(MESSAGING_CLIENT_SENT_MESSAGES);
        assert_eq!(sent.len(), 1);
        let point = &sent[0];
        assert_eq!(point.value, 1);
        assert_eq!(
            point.attribute("messaging.system.name"),
            Some(&Value::from("kafka"))
        );
        assert_eq!(
            point.attribute("messaging.destination.name"),
            Some(&Value::from("orders"))
        );
        assert_eq!(
            point.attribute("messaging.operation.name"),
            Some(&Value::from("send"))
        );
        assert_eq!(
            point.attribute("messaging.operation.type"),
            Some(&Value::from("send"))
        );
        assert_eq!(
            point.attribute("messaging.destination.partition.id"),
            Some(&Value::from("2"))
        );
        assert_eq!(point.attribute("error.type"), None);

        let durations = harness.f64_histogram(MESSAGING_CLIENT_OPERATION_DURATION);
        assert_eq!(durations.len(), 1);
        assert_eq!(durations[0].count, 1);
        assert_eq!(durations[0].bounds, DURATION_HISTOGRAM_BOUNDARIES.to_vec());
    }

    #[test]
    fn encode_failures_record_encode() {
        let error = SendError::Encode(serde_json::from_str::<serde_json::Value>("{").unwrap_err());
        let harness = record_failure(&error);

        let sent = harness.u64_sum(MESSAGING_CLIENT_SENT_MESSAGES);
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].attribute("error.type"),
            Some(&Value::from("encode"))
        );
        assert_eq!(
            sent[0].attribute("messaging.destination.partition.id"),
            None
        );
        let durations = harness.f64_histogram(MESSAGING_CLIENT_OPERATION_DURATION);
        assert_eq!(durations.len(), 1);
        assert_eq!(
            durations[0].attribute("error.type"),
            Some(&Value::from("encode"))
        );
        assert_eq!(
            durations[0].attribute("messaging.destination.partition.id"),
            None
        );
    }

    #[test]
    fn delivery_failures_record_the_error_code() {
        let error = SendError::Delivery(KafkaError::MessageProduction(
            RDKafkaErrorCode::MessageTimedOut,
        ));
        let harness = record_failure(&error);

        let sent = harness.u64_sum(MESSAGING_CLIENT_SENT_MESSAGES);
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].attribute("error.type"),
            Some(&Value::from("MessageTimedOut"))
        );
    }

    #[test]
    fn delivery_failures_without_a_code_record_other() {
        assert_eq!(KafkaError::Canceled.rdkafka_error_code(), None);
        let harness = record_failure(&SendError::Delivery(KafkaError::Canceled));

        let sent = harness.u64_sum(MESSAGING_CLIENT_SENT_MESSAGES);
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].attribute("error.type"),
            Some(&Value::from("_OTHER"))
        );
    }

    #[test]
    fn clones_share_instruments() {
        let harness = MetricsHarness::default();
        let metrics = ProducerMetrics::new(harness.provider());
        let ack = Ack {
            partition: 0,
            offset: 0,
        };
        metrics
            .instruments("a")
            .record(Duration::from_millis(1), Ok(&ack));
        metrics
            .clone()
            .instruments("a")
            .record(Duration::from_millis(1), Ok(&ack));

        let sent = harness.u64_sum(MESSAGING_CLIENT_SENT_MESSAGES);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].value, 2);
    }
}
