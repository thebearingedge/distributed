//! Error types for the kafka lib and their classification into
//! [`messaging::MessagingError`] and [`runtime::run_error::RunError`].
//!
//! `KafkaClientError` classifies as a permanent [`runtime::run_error::RunError`]
//! because a restart cannot fix it; `ConsumeError` classifies as retryable,
//! except that a handler failure keeps its own classification.

use rdkafka::error::{KafkaError, RDKafkaErrorCode};
use thiserror::Error;

use messaging::MessagingError;
use runtime::run_error::RunError;

/// Building or subscribing a Kafka client failed.
#[derive(Debug, Error)]
pub enum KafkaClientError {
    /// librdkafka rejected the client configuration.
    #[error("invalid kafka client config: {0}")]
    Config(#[source] KafkaError),
    /// Subscribing a consumer to a topic failed.
    #[error("subscribing to topic `{topic}` failed: {source}")]
    Subscribe {
        /// The topic the consumer failed to subscribe to.
        topic: String,
        /// The underlying librdkafka error.
        #[source]
        source: KafkaError,
    },
}

/// Producing a message failed.
#[derive(Debug, Error)]
pub enum SendError {
    /// The payload could not be encoded as JSON.
    #[error("failed to encode payload: {0}")]
    Encode(#[from] serde_json::Error),
    /// The broker refused or could not deliver the message.
    #[error("kafka delivery failed: {0}")]
    Delivery(#[source] KafkaError),
    /// The final flush on shutdown timed out or failed.
    #[error("kafka producer flush failed: {0}")]
    Flush(#[source] KafkaError),
    /// The blocking flush task panicked or was cancelled.
    #[error("kafka producer flush task failed: {0}")]
    FlushTask(#[source] tokio::task::JoinError),
}

/// Consumption stopped with a failure.
#[derive(Debug, Error)]
pub enum ConsumeError {
    /// The message handler failed; the message is redelivered.
    #[error("handler failed at partition {partition}, offset {offset}: {source}")]
    Handler {
        /// The partition the failing message was read from.
        partition: i32,
        /// The offset of the failing message.
        offset: i64,
        /// Why the handler failed.
        #[source]
        source: RunError,
    },
    /// A fatal librdkafka error; the client must be recreated.
    #[error("fatal kafka consumer error: {0}")]
    Fatal(#[source] KafkaError),
    /// The final offset commit on shutdown failed.
    #[error("final offset commit failed: {0}")]
    Commit(#[source] KafkaError),
    /// The blocking close task panicked or was cancelled.
    #[error("kafka consumer close task failed: {0}")]
    CloseTask(#[source] tokio::task::JoinError),
}

/// An admin request failed.
#[derive(Debug, Error)]
pub enum KafkaAdminError {
    /// librdkafka rejected the admin client configuration.
    #[error("invalid kafka admin config: {0}")]
    Config(#[source] KafkaError),
    /// The admin request to the broker failed.
    #[error("kafka admin request failed: {0}")]
    Request(#[source] KafkaError),
    /// The per-topic operation failed with a broker error code.
    #[error("topic `{topic}` operation failed: {code}")]
    Topic {
        /// The topic the operation targeted.
        topic: String,
        /// The broker error code reported for the operation.
        code: RDKafkaErrorCode,
    },
}

/// A message payload could not be decoded.
#[derive(Debug, Error)]
pub(crate) enum DecodeError {
    /// The message carried no payload.
    #[error("empty payload")]
    EmptyPayload,
    /// The payload was not valid JSON.
    #[error("invalid json payload: {0}")]
    Json(#[source] serde_json::Error),
}

/// True when librdkafka has raised a fatal error and the client must be
/// recreated.
pub(crate) fn is_fatal(error: &KafkaError) -> bool {
    match error {
        KafkaError::MessageConsumptionFatal(_) => true,
        KafkaError::Transaction(error) => error.is_fatal(),
        other => other.rdkafka_error_code() == Some(RDKafkaErrorCode::Fatal),
    }
}

impl From<SendError> for MessagingError {
    fn from(error: SendError) -> Self {
        let rejected = match &error {
            SendError::Encode(_) => true,
            SendError::Delivery(source) => is_rejection(source),
            SendError::Flush(_) | SendError::FlushTask(_) => false,
        };
        if rejected {
            Self::Rejected(Box::new(error))
        } else {
            Self::Unavailable(Box::new(error))
        }
    }
}

impl From<ConsumeError> for RunError {
    fn from(error: ConsumeError) -> Self {
        match error {
            ConsumeError::Handler { source, .. } => source,
            error => Self::retryable(error),
        }
    }
}

impl From<KafkaClientError> for RunError {
    fn from(error: KafkaClientError) -> Self {
        Self::permanent(error)
    }
}

/// True when the broker refused the message itself, so retrying the same
/// message cannot succeed.
fn is_rejection(error: &KafkaError) -> bool {
    matches!(
        error.rdkafka_error_code(),
        Some(
            RDKafkaErrorCode::MessageSizeTooLarge
                | RDKafkaErrorCode::InvalidMessageSize
                | RDKafkaErrorCode::InvalidMessage
                | RDKafkaErrorCode::InvalidRecord
                | RDKafkaErrorCode::UnknownTopic
                | RDKafkaErrorCode::UnknownTopicOrPartition
                | RDKafkaErrorCode::TopicAuthorizationFailed
                | RDKafkaErrorCode::InvalidArgument
        )
    )
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use runtime::exit_status::ExitStatus;

    use super::*;

    #[test]
    fn message_consumption_fatal_is_fatal() {
        let error = KafkaError::MessageConsumptionFatal(RDKafkaErrorCode::Fatal);
        assert!(is_fatal(&error));
    }

    #[test]
    fn message_production_fatal_is_fatal() {
        let error = KafkaError::MessageProduction(RDKafkaErrorCode::Fatal);
        assert!(is_fatal(&error));
    }

    #[test]
    fn message_consumption_transport_failure_is_not_fatal() {
        let error = KafkaError::MessageConsumption(RDKafkaErrorCode::BrokerTransportFailure);
        assert!(!is_fatal(&error));
    }

    fn encode_error() -> SendError {
        SendError::Encode(serde_json::from_str::<u8>("x").unwrap_err())
    }

    #[test]
    fn encode_classifies_as_rejected() {
        assert!(matches!(
            MessagingError::from(encode_error()),
            MessagingError::Rejected(_)
        ));
    }

    #[test]
    fn oversized_delivery_classifies_as_rejected() {
        let error = SendError::Delivery(KafkaError::MessageProduction(
            RDKafkaErrorCode::MessageSizeTooLarge,
        ));
        assert!(matches!(
            MessagingError::from(error),
            MessagingError::Rejected(_)
        ));
    }

    #[test]
    fn queue_full_delivery_classifies_as_unavailable() {
        let error = SendError::Delivery(KafkaError::MessageProduction(RDKafkaErrorCode::QueueFull));
        assert!(matches!(
            MessagingError::from(error),
            MessagingError::Unavailable(_)
        ));
    }

    #[test]
    fn fatal_delivery_classifies_as_unavailable() {
        let error = SendError::Delivery(KafkaError::MessageProduction(RDKafkaErrorCode::Fatal));
        assert!(matches!(
            MessagingError::from(error),
            MessagingError::Unavailable(_)
        ));
    }

    #[test]
    fn handler_error_keeps_its_classification() {
        let error = ConsumeError::Handler {
            partition: 0,
            offset: 7,
            source: RunError::permanent("bad input"),
        };
        assert_eq!(RunError::from(error).exit_status(), ExitStatus::Permanent);
    }

    #[test]
    fn fatal_consume_error_is_retryable() {
        let error =
            ConsumeError::Fatal(KafkaError::MessageConsumptionFatal(RDKafkaErrorCode::Fatal));
        assert_eq!(RunError::from(error).exit_status(), ExitStatus::Retryable);
    }

    #[test]
    fn client_error_is_permanent() {
        let error = KafkaClientError::Config(KafkaError::MessageProduction(
            RDKafkaErrorCode::InvalidArgument,
        ));
        assert_eq!(RunError::from(error).exit_status(), ExitStatus::Permanent);
    }

    #[test]
    fn subscribe_display_includes_the_topic_and_source() {
        let error = KafkaClientError::Subscribe {
            topic: "greetings".into(),
            source: KafkaError::MessageConsumptionFatal(RDKafkaErrorCode::Fatal),
        };
        let display = error.to_string();
        assert!(display.contains("subscribing to topic `greetings` failed"));
        assert!(display.contains("(Fatal) Message consumption error"));
        assert!(error.source().is_some());
    }

    #[test]
    fn handler_display_includes_partition_offset_and_source() {
        let error = ConsumeError::Handler {
            partition: 3,
            offset: 42,
            source: RunError::permanent("boom"),
        };
        let display = error.to_string();
        assert!(display.contains("partition 3"));
        assert!(display.contains("offset 42"));
        assert!(display.contains("boom"));
        assert!(error.source().is_some());
    }
}
