use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use opentelemetry::{
    KeyValue,
    metrics::{Counter, Histogram, MeterProvider},
};

use super::consumer_lag::{LagRegistry, LagSnapshot};
use super::{
    ERROR_TYPE, MESSAGE_UNIT, MESSAGING_CONSUMER_GROUP_NAME, MESSAGING_OPERATION_NAME,
    destination_attributes, meter, partition_id,
};
use crate::kafka_error::DecodeError;
use runtime::run_error::RunError;
use telemetry::metrics::duration_histogram;

const MESSAGING_CLIENT_CONSUMED_MESSAGES: &str = "messaging.client.consumed.messages";
const MESSAGING_PROCESS_DURATION: &str = "messaging.process.duration";
const MESSAGING_KAFKA_CONSUMER_SKIPPED_MESSAGES: &str = "messaging.kafka.consumer.skipped.messages";
const MESSAGING_KAFKA_CONSUMER_LAG: &str = "messaging.kafka.consumer.lag";
const MESSAGING_OPERATION_POLL: &str = "poll";
const MESSAGING_OPERATION_PROCESS: &str = "process";
const ERROR_TYPE_EMPTY_PAYLOAD: &str = "empty_payload";
const ERROR_TYPE_INVALID_JSON: &str = "invalid_json";
const ERROR_TYPE_RETRYABLE: &str = "retryable";
const ERROR_TYPE_PERMANENT: &str = "permanent";

/// The consumer instruments `messaging.client.consumed.messages`,
/// `messaging.process.duration`,
/// `messaging.kafka.consumer.skipped.messages`, and the
/// `messaging.kafka.consumer.lag` gauge, created under the `kafka`
/// instrumentation scope.
///
/// Build one per meter provider and pass it to every
/// [`crate::consumer::KafkaConsumer::new`]; clones share the instruments.
/// Each call to [`ConsumerMetrics::new`] registers a lag callback for the
/// lifetime of the meter provider.
#[derive(Clone)]
pub struct ConsumerMetrics {
    consumed: Counter<u64>,
    process_duration: Histogram<f64>,
    skipped: Counter<u64>,
    lag: Arc<LagRegistry>,
}

impl fmt::Debug for ConsumerMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConsumerMetrics").finish_non_exhaustive()
    }
}

impl ConsumerMetrics {
    /// Creates the consumer instruments and the lag gauge on
    /// `meter_provider`.
    #[must_use]
    pub fn new<P>(meter_provider: &P) -> Self
    where
        P: MeterProvider + ?Sized,
    {
        let meter = meter(meter_provider);
        let lag = Arc::new(LagRegistry::default());
        let observed = Arc::clone(&lag);
        let _ = meter
            .i64_observable_gauge(MESSAGING_KAFKA_CONSUMER_LAG)
            .with_unit(MESSAGE_UNIT)
            .with_description(
                "Number of messages between a consumer's position and the partition's high watermark.",
            )
            .with_callback(move |observer| observed.observe(observer))
            .build();
        Self {
            consumed: meter
                .u64_counter(MESSAGING_CLIENT_CONSUMED_MESSAGES)
                .with_unit(MESSAGE_UNIT)
                .with_description("Number of messages that were delivered to the application.")
                .build(),
            process_duration: duration_histogram(
                &meter,
                MESSAGING_PROCESS_DURATION,
                "Duration of processing operations.",
            ),
            skipped: meter
                .u64_counter(MESSAGING_KAFKA_CONSUMER_SKIPPED_MESSAGES)
                .with_unit(MESSAGE_UNIT)
                .with_description(
                    "Number of received messages skipped because their payload was empty or not decodable.",
                )
                .build(),
            lag,
        }
    }

    /// The instruments bound to the attributes of a consumer on `topic` in
    /// `group`, with a fresh lag snapshot registered on the lag gauge.
    pub(crate) fn instruments(&self, topic: &str, group: &str) -> ConsumerInstruments {
        let mut attributes = destination_attributes(topic);
        attributes.push(KeyValue::new(
            MESSAGING_CONSUMER_GROUP_NAME,
            group.to_owned(),
        ));
        let lag = Arc::new(LagSnapshot::new(topic, attributes.clone()));
        self.lag.register(&lag);
        ConsumerInstruments {
            metrics: self.clone(),
            attributes,
            lag,
        }
    }
}

/// The consumer instruments bound to one consumer's topic and group.
pub(crate) struct ConsumerInstruments {
    metrics: ConsumerMetrics,
    attributes: Vec<KeyValue>,
    lag: Arc<LagSnapshot>,
}

impl ConsumerInstruments {
    /// The consumer's lag snapshot, fed by librdkafka statistics.
    pub(crate) fn lag(&self) -> Arc<LagSnapshot> {
        Arc::clone(&self.lag)
    }

    /// Records one message received from `partition`.
    pub(crate) fn record_consumed(&self, partition: i32) {
        self.metrics.consumed.add(
            1,
            &self.attributes([
                KeyValue::new(MESSAGING_OPERATION_NAME, MESSAGING_OPERATION_POLL),
                partition_id(partition),
            ]),
        );
    }

    /// Records one handler run on a message from `partition` that took
    /// `elapsed`, classified by the handler's `error`, if any.
    pub(crate) fn record_processed(
        &self,
        partition: i32,
        elapsed: Duration,
        error: Option<&RunError>,
    ) {
        let attributes = self.attributes(
            [
                KeyValue::new(MESSAGING_OPERATION_NAME, MESSAGING_OPERATION_PROCESS),
                partition_id(partition),
            ]
            .into_iter()
            .chain(error.map(|error| KeyValue::new(ERROR_TYPE, run_error_type(error)))),
        );
        self.metrics
            .process_duration
            .record(elapsed.as_secs_f64(), &attributes);
    }

    /// Records one message from `partition` skipped because of `error`.
    pub(crate) fn record_skipped(&self, partition: i32, error: &DecodeError) {
        self.metrics.skipped.add(
            1,
            &self.attributes([
                partition_id(partition),
                KeyValue::new(ERROR_TYPE, decode_error_type(error)),
            ]),
        );
    }

    /// The consumer's shared attributes followed by `extra`.
    fn attributes(&self, extra: impl IntoIterator<Item = KeyValue>) -> Vec<KeyValue> {
        self.attributes.iter().cloned().chain(extra).collect()
    }
}

/// The bounded `error.type` of a failed handler run.
fn run_error_type(error: &RunError) -> &'static str {
    match error {
        RunError::Retryable(_) => ERROR_TYPE_RETRYABLE,
        RunError::Permanent(_) => ERROR_TYPE_PERMANENT,
    }
}

/// The bounded `error.type` of a skipped message.
fn decode_error_type(error: &DecodeError) -> &'static str {
    match error {
        DecodeError::EmptyPayload => ERROR_TYPE_EMPTY_PAYLOAD,
        DecodeError::Json(_) => ERROR_TYPE_INVALID_JSON,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use opentelemetry::Value;

    use super::{
        ConsumerMetrics, MESSAGING_CLIENT_CONSUMED_MESSAGES, MESSAGING_KAFKA_CONSUMER_LAG,
        MESSAGING_KAFKA_CONSUMER_SKIPPED_MESSAGES, MESSAGING_PROCESS_DURATION,
    };
    use crate::kafka_error::DecodeError;
    use runtime::run_error::RunError;
    use telemetry::test_support::MetricsHarness;

    const LAG_REPORT: &str = r#"{"topics":{"orders":{"partitions":{
        "0":{"partition":0,"desired":true,"consumer_lag":5,"consumer_lag_stored":3},
        "1":{"partition":1,"desired":true,"consumer_lag":2,"consumer_lag_stored":0}
    }}}}"#;

    fn lag_report(topic: &str) -> Vec<u8> {
        LAG_REPORT.replace("orders", topic).into_bytes()
    }

    #[test]
    fn consumed_messages_record_poll_and_partition() {
        let harness = MetricsHarness::default();
        ConsumerMetrics::new(harness.provider())
            .instruments("orders", "g")
            .record_consumed(0);

        let consumed = harness.u64_sum(MESSAGING_CLIENT_CONSUMED_MESSAGES);
        assert_eq!(consumed.len(), 1);
        let point = &consumed[0];
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
            point.attribute("messaging.consumer.group.name"),
            Some(&Value::from("g"))
        );
        assert_eq!(
            point.attribute("messaging.operation.name"),
            Some(&Value::from("poll"))
        );
        assert_eq!(
            point.attribute("messaging.destination.partition.id"),
            Some(&Value::from("0"))
        );
    }

    #[test]
    fn processed_messages_record_no_error_type() {
        let harness = MetricsHarness::default();
        ConsumerMetrics::new(harness.provider())
            .instruments("orders", "g")
            .record_processed(0, Duration::from_millis(5), None);

        let durations = harness.f64_histogram(MESSAGING_PROCESS_DURATION);
        assert_eq!(durations.len(), 1);
        let point = &durations[0];
        assert_eq!(point.count, 1);
        assert_eq!(
            point.attribute("messaging.operation.name"),
            Some(&Value::from("process"))
        );
        assert_eq!(
            point.attribute("messaging.destination.partition.id"),
            Some(&Value::from("0"))
        );
        assert_eq!(point.attribute("error.type"), None);
    }

    #[test]
    fn handler_failures_record_their_classification() {
        let harness = MetricsHarness::default();
        let instruments = ConsumerMetrics::new(harness.provider()).instruments("orders", "g");
        instruments.record_processed(0, Duration::from_millis(5), Some(&RunError::retryable("x")));
        instruments.record_processed(0, Duration::from_millis(5), Some(&RunError::permanent("x")));

        let durations = harness.f64_histogram(MESSAGING_PROCESS_DURATION);
        assert_eq!(durations.len(), 2);
        for classification in ["retryable", "permanent"] {
            let point = durations
                .iter()
                .find(|point| point.attribute("error.type") == Some(&Value::from(classification)))
                .unwrap_or_else(|| panic!("a point classified {classification}"));
            assert_eq!(point.count, 1);
        }
    }

    #[test]
    fn skipped_messages_record_the_reason() {
        let harness = MetricsHarness::default();
        let instruments = ConsumerMetrics::new(harness.provider()).instruments("orders", "g");
        instruments.record_skipped(0, &DecodeError::EmptyPayload);
        instruments.record_skipped(
            0,
            &DecodeError::Json(serde_json::from_str::<serde_json::Value>("{").unwrap_err()),
        );

        let skipped = harness.u64_sum(MESSAGING_KAFKA_CONSUMER_SKIPPED_MESSAGES);
        assert_eq!(skipped.len(), 2);
        for reason in ["empty_payload", "invalid_json"] {
            let point = skipped
                .iter()
                .find(|point| point.attribute("error.type") == Some(&Value::from(reason)))
                .unwrap_or_else(|| panic!("a point skipped for {reason}"));
            assert_eq!(point.value, 1);
            assert_eq!(
                point.attribute("messaging.destination.partition.id"),
                Some(&Value::from("0"))
            );
            assert_eq!(
                point.attribute("messaging.consumer.group.name"),
                Some(&Value::from("g"))
            );
        }
    }

    #[test]
    fn lag_is_observed_per_partition() {
        let harness = MetricsHarness::default();
        let instruments = ConsumerMetrics::new(harness.provider()).instruments("orders", "g");
        instruments.lag().update(&lag_report("orders"));

        let lag = harness.i64_gauge(MESSAGING_KAFKA_CONSUMER_LAG);
        assert_eq!(lag.len(), 2);
        for (partition, expected) in [("0", 3), ("1", 0)] {
            let point = lag
                .iter()
                .find(|point| {
                    point.attribute("messaging.destination.partition.id")
                        == Some(&Value::from(partition))
                })
                .unwrap_or_else(|| panic!("a lag point for partition {partition}"));
            assert_eq!(point.value, expected);
            assert_eq!(
                point.attribute("messaging.destination.name"),
                Some(&Value::from("orders"))
            );
            assert_eq!(
                point.attribute("messaging.consumer.group.name"),
                Some(&Value::from("g"))
            );
            assert_eq!(
                point.attribute("messaging.system.name"),
                Some(&Value::from("kafka"))
            );
        }
    }

    #[test]
    fn dropped_consumers_stop_reporting_lag() {
        let harness = MetricsHarness::default();
        let instruments = ConsumerMetrics::new(harness.provider()).instruments("orders", "g");
        instruments.lag().update(&lag_report("orders"));
        assert_eq!(harness.i64_gauge(MESSAGING_KAFKA_CONSUMER_LAG).len(), 2);

        drop(instruments);
        assert!(harness.i64_gauge(MESSAGING_KAFKA_CONSUMER_LAG).is_empty());
    }

    #[test]
    fn one_handle_reports_every_live_consumer() {
        let harness = MetricsHarness::default();
        let metrics = ConsumerMetrics::new(harness.provider());
        let orders = metrics.instruments("orders", "g");
        let payments = metrics.instruments("payments", "g");
        orders.lag().update(&lag_report("orders"));
        payments.lag().update(&lag_report("payments"));

        let lag = harness.i64_gauge(MESSAGING_KAFKA_CONSUMER_LAG);
        assert_eq!(lag.len(), 4);
        for topic in ["orders", "payments"] {
            assert_eq!(
                lag.iter()
                    .filter(|point| point.attribute("messaging.destination.name")
                        == Some(&Value::from(topic)))
                    .count(),
                2
            );
        }
    }
}
