//! OpenTelemetry messaging metrics for Kafka clients: the
//! [`crate::metrics::ProducerMetrics`] and [`crate::metrics::ConsumerMetrics`]
//! handles passed to [`crate::producer::KafkaProducer::new`] and
//! [`crate::consumer::KafkaConsumer::new`].

mod consumer_lag;
mod consumer_metrics;
mod producer_metrics;

pub(crate) use consumer_lag::LagSnapshot;
pub(crate) use consumer_metrics::ConsumerInstruments;
pub use consumer_metrics::ConsumerMetrics;
pub use producer_metrics::ProducerMetrics;
pub(crate) use producer_metrics::SendInstruments;

use opentelemetry::{
    InstrumentationScope, Key, KeyValue,
    metrics::{Meter, MeterProvider},
};

const MESSAGING_SYSTEM_NAME: Key = Key::from_static_str("messaging.system.name");
const MESSAGING_SYSTEM_NAME_KAFKA: &str = "kafka";
const MESSAGING_DESTINATION_NAME: Key = Key::from_static_str("messaging.destination.name");
const MESSAGING_DESTINATION_PARTITION_ID: Key =
    Key::from_static_str("messaging.destination.partition.id");
const MESSAGING_CONSUMER_GROUP_NAME: Key = Key::from_static_str("messaging.consumer.group.name");
const MESSAGING_OPERATION_NAME: Key = Key::from_static_str("messaging.operation.name");
const MESSAGING_OPERATION_TYPE: Key = Key::from_static_str("messaging.operation.type");
const ERROR_TYPE: Key = Key::from_static_str("error.type");
const ERROR_TYPE_OTHER: &str = "_OTHER";
const MESSAGE_UNIT: &str = "{message}";

/// The kafka instrumentation-scoped meter of `meter_provider`.
fn meter<P>(meter_provider: &P) -> Meter
where
    P: MeterProvider + ?Sized,
{
    meter_provider.meter_with_scope(
        InstrumentationScope::builder(env!("CARGO_PKG_NAME"))
            .with_version(env!("CARGO_PKG_VERSION"))
            .build(),
    )
}

/// The `messaging.system.name` and `messaging.destination.name`
/// attributes shared by every instrument of a client on `topic`.
fn destination_attributes(topic: &str) -> Vec<KeyValue> {
    vec![
        KeyValue::new(MESSAGING_SYSTEM_NAME, MESSAGING_SYSTEM_NAME_KAFKA),
        KeyValue::new(MESSAGING_DESTINATION_NAME, topic.to_owned()),
    ]
}

/// The `messaging.destination.partition.id` attribute, which semantic
/// conventions record as a string.
fn partition_id(partition: i32) -> KeyValue {
    KeyValue::new(MESSAGING_DESTINATION_PARTITION_ID, partition.to_string())
}
