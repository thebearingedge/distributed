//! Command-line configuration for the greetings consumer binary.

use kafka::config::{KafkaConfig, KafkaConsumerConfig};
use worker::WorkerConfig;

/// Command-line configuration for the greetings consumer binary.
#[derive(Debug, clap::Parser)]
pub(crate) struct Args {
    /// Worker runtime configuration.
    #[command(flatten)]
    pub(crate) worker: WorkerConfig,
    /// Kafka connection configuration.
    #[command(flatten)]
    pub(crate) kafka: KafkaConfig,
    /// Kafka consumer configuration.
    #[command(flatten)]
    pub(crate) consumer: KafkaConsumerConfig,
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use clap::{CommandFactory, Parser};
    use kafka::config::AutoOffsetReset;

    use super::Args;

    #[test]
    fn parses_with_all_configs_flattened() {
        let args = Args::try_parse_from([
            "test",
            "--kafka-bootstrap-servers",
            "127.0.0.1:9092",
            "--kafka-consumer-group-id",
            "greetings-consumer",
        ])
        .unwrap();
        assert_eq!(args.kafka.kafka_bootstrap_servers, "127.0.0.1:9092");
        assert_eq!(args.consumer.kafka_consumer_group_id, "greetings-consumer");
        assert_eq!(
            args.consumer.kafka_consumer_auto_offset_reset,
            AutoOffsetReset::Earliest
        );
    }

    #[test]
    fn group_id_is_required_and_env_backed() {
        let command = Args::command();
        let arg = command
            .get_arguments()
            .find(|arg| arg.get_long() == Some("kafka-consumer-group-id"))
            .expect("kafka_consumer_group_id argument is defined");
        assert!(arg.is_required_set());
        assert_eq!(arg.get_default_values(), &[] as &[std::ffi::OsString]);
        assert_eq!(arg.get_env(), Some(OsStr::new("KAFKA_CONSUMER_GROUP_ID")));
    }
}
