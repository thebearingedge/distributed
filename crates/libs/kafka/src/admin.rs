//! Topic administration helpers for creating and deleting topics.

use std::time::Duration;

use rdkafka::admin::{AdminClient, AdminOptions, NewTopic, TopicReplication};
use rdkafka::client::DefaultClientContext;
use rdkafka::error::RDKafkaErrorCode;

use crate::config::KafkaConfig;
use crate::kafka_error::KafkaAdminError;

const ADMIN_OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

/// Creates a topic with the given partition count and replication factor.
/// Creating an already existing topic succeeds, so the operation is
/// idempotent.
///
/// # Errors
/// Returns [`KafkaAdminError::Config`] when librdkafka rejects the admin
/// client configuration, [`KafkaAdminError::Topic`] when the broker reports
/// a per-topic failure other than an existing topic, and
/// [`KafkaAdminError::Request`] when the admin request itself fails.
pub async fn create_topic(
    config: &KafkaConfig,
    topic: &str,
    partitions: i32,
    replication_factor: i32,
) -> Result<(), KafkaAdminError> {
    let admin: AdminClient<DefaultClientContext> = config
        .client_config()
        .create()
        .map_err(KafkaAdminError::Config)?;
    let options = AdminOptions::new().operation_timeout(Some(ADMIN_OPERATION_TIMEOUT));
    let new_topic = NewTopic::new(
        topic,
        partitions,
        TopicReplication::Fixed(replication_factor),
    );
    for result in admin
        .create_topics(&[new_topic], &options)
        .await
        .map_err(KafkaAdminError::Request)?
    {
        match result {
            Ok(_) | Err((_, RDKafkaErrorCode::TopicAlreadyExists)) => {}
            Err((_name, code)) => {
                return Err(KafkaAdminError::Topic {
                    topic: topic.into(),
                    code,
                });
            }
        }
    }
    Ok(())
}

/// Deletes the named topic. Deleting a missing topic succeeds, so the
/// operation is idempotent.
///
/// # Errors
/// Returns [`KafkaAdminError::Config`] when librdkafka rejects the admin
/// client configuration, [`KafkaAdminError::Topic`] when the broker reports
/// a per-topic failure other than a missing topic, and
/// [`KafkaAdminError::Request`] when the admin request itself fails.
pub async fn delete_topic(config: &KafkaConfig, topic: &str) -> Result<(), KafkaAdminError> {
    let admin: AdminClient<DefaultClientContext> = config
        .client_config()
        .create()
        .map_err(KafkaAdminError::Config)?;
    let options = AdminOptions::new().operation_timeout(Some(ADMIN_OPERATION_TIMEOUT));
    for result in admin
        .delete_topics(&[topic], &options)
        .await
        .map_err(KafkaAdminError::Request)?
    {
        match result {
            Ok(_) | Err((_, RDKafkaErrorCode::UnknownTopicOrPartition)) => {}
            Err((_name, code)) => {
                return Err(KafkaAdminError::Topic {
                    topic: topic.into(),
                    code,
                });
            }
        }
    }
    Ok(())
}
