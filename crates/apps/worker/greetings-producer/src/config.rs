//! Command-line configuration for the greetings producer binary.

use std::time::Duration;

use kafka::config::{KafkaConfig, KafkaProducerConfig};
use worker::WorkerConfig;

/// Default for [`Args::greeting_interval_seconds`].
pub(crate) const GREETING_INTERVAL_DEFAULT_SECONDS: u64 = 3;

/// Command-line configuration for the greetings producer binary.
#[derive(Debug, clap::Parser)]
pub(crate) struct Args {
    /// Worker runtime configuration.
    #[command(flatten)]
    pub(crate) worker: WorkerConfig,
    /// Kafka connection configuration.
    #[command(flatten)]
    pub(crate) kafka: KafkaConfig,
    /// Kafka producer configuration.
    #[command(flatten)]
    pub(crate) producer: KafkaProducerConfig,
    /// Seconds between published greetings.
    #[arg(
        long,
        env = "GREETING_INTERVAL_SECONDS",
        default_value_t = GREETING_INTERVAL_DEFAULT_SECONDS,
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    pub(crate) greeting_interval_seconds: u64,
    /// Message each published greeting carries.
    #[arg(long, env = "GREETING_MESSAGE", default_value = "Hello, Kafka!")]
    pub(crate) greeting_message: String,
}

impl Args {
    /// Time between published greetings.
    #[must_use]
    pub(crate) fn greeting_interval(&self) -> Duration {
        Duration::from_secs(self.greeting_interval_seconds)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use clap::Parser;

    use super::{Args, GREETING_INTERVAL_DEFAULT_SECONDS};

    #[test]
    fn parses_with_all_configs_flattened() {
        let args = Args::try_parse_from(["test", "--kafka-bootstrap-servers", "127.0.0.1:9092"])
            .expect("the flattened configs have no clap id collision");

        assert_eq!(args.kafka.kafka_bootstrap_servers, "127.0.0.1:9092");
        assert_eq!(
            args.greeting_interval_seconds,
            GREETING_INTERVAL_DEFAULT_SECONDS
        );
        assert_eq!(args.greeting_message, "Hello, Kafka!");
        assert_eq!(args.greeting_interval(), Duration::from_secs(3));
    }

    #[test]
    fn zero_interval_is_rejected() {
        let args = Args::try_parse_from([
            "test",
            "--kafka-bootstrap-servers",
            "127.0.0.1:9092",
            "--greeting-interval-seconds",
            "0",
        ]);

        assert!(args.is_err());
    }
}
