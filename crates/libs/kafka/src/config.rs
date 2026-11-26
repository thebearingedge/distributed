//! Client configuration from CLI args and env vars: bootstrap servers,
//! client id, consumer group settings, and producer timeouts.

use std::time::Duration;

use rdkafka::ClientConfig;

/// Default for
/// [`KafkaProducerConfig::kafka_producer_delivery_timeout_seconds`].
pub const KAFKA_PRODUCER_DELIVERY_TIMEOUT_DEFAULT_SECONDS: u64 = 30;
/// Default for [`KafkaProducerConfig::kafka_producer_flush_timeout_seconds`].
pub const KAFKA_PRODUCER_FLUSH_TIMEOUT_DEFAULT_SECONDS: u64 = 10;

const BOOTSTRAP_SERVERS: &str = "bootstrap.servers";
const BROKER_ADDRESS_FAMILY: &str = "broker.address.family";
const CLIENT_ID: &str = "client.id";
const GROUP_ID: &str = "group.id";
const ENABLE_AUTO_COMMIT: &str = "enable.auto.commit";
const ENABLE_AUTO_OFFSET_STORE: &str = "enable.auto.offset.store";
const AUTO_OFFSET_RESET: &str = "auto.offset.reset";
const ENABLE_IDEMPOTENCE: &str = "enable.idempotence";
const MESSAGE_TIMEOUT_MS: &str = "message.timeout.ms";

/// Kafka client configuration shared by producers, consumers and admin
/// helpers: bootstrap brokers and the client id. CLI args fall back to env
/// vars; bootstrap servers are required.
#[derive(Debug, clap::Parser)]
pub struct KafkaConfig {
    /// Comma-separated `host:port` bootstrap brokers.
    #[arg(long, env = "KAFKA_BOOTSTRAP_SERVERS")]
    pub kafka_bootstrap_servers: String,
    /// Client id reported to brokers; librdkafka's default when unset.
    #[arg(long, env = "KAFKA_CLIENT_ID")]
    pub kafka_client_id: Option<String>,
}

impl KafkaConfig {
    /// Builds the rdkafka client config with the bootstrap servers, the
    /// IPv4-only broker address family, and the client id when configured.
    #[must_use]
    pub(crate) fn client_config(&self) -> ClientConfig {
        let mut config = ClientConfig::new();
        config.set(BOOTSTRAP_SERVERS, self.kafka_bootstrap_servers.as_str());
        config.set(BROKER_ADDRESS_FAMILY, "v4");
        if let Some(client_id) = &self.kafka_client_id {
            config.set(CLIENT_ID, client_id.as_str());
        }
        config
    }
}

/// Where a consumer group with no committed offset starts reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum AutoOffsetReset {
    /// Start from the oldest available offset.
    Earliest,
    /// Start from the newest offset.
    Latest,
}

impl AutoOffsetReset {
    /// The librdkafka value for [`KafkaConsumerConfig`]'s
    /// `auto.offset.reset` setting.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Earliest => "earliest",
            Self::Latest => "latest",
        }
    }
}

/// Consumer configuration: group id and where a group with no committed
/// offset starts reading. CLI args fall back to env vars, which fall back
/// to declared defaults.
#[derive(Debug, clap::Parser)]
pub struct KafkaConsumerConfig {
    /// Consumer group id.
    #[arg(long, env = "KAFKA_CONSUMER_GROUP_ID")]
    pub kafka_consumer_group_id: String,
    /// Where a group with no committed offset starts reading.
    #[arg(
        long,
        env = "KAFKA_CONSUMER_AUTO_OFFSET_RESET",
        value_enum,
        default_value_t = AutoOffsetReset::Earliest
    )]
    pub kafka_consumer_auto_offset_reset: AutoOffsetReset,
}

impl KafkaConsumerConfig {
    /// Applies the consumer settings to an rdkafka client config: the
    /// group id, auto-commit enabled, manual offset storing, and the
    /// auto offset reset value.
    pub(crate) fn apply(&self, config: &mut ClientConfig) {
        config.set(GROUP_ID, self.kafka_consumer_group_id.as_str());
        config.set(ENABLE_AUTO_COMMIT, "true");
        config.set(ENABLE_AUTO_OFFSET_STORE, "false");
        config.set(
            AUTO_OFFSET_RESET,
            self.kafka_consumer_auto_offset_reset.as_str(),
        );
    }
}

/// Producer configuration: delivery and flush timeouts. CLI args fall back
/// to env vars, which fall back to declared defaults.
#[derive(Debug, clap::Parser)]
pub struct KafkaProducerConfig {
    /// Seconds librdkafka may spend delivering a message before failing it.
    #[arg(
        long,
        env = "KAFKA_PRODUCER_DELIVERY_TIMEOUT_SECONDS",
        default_value_t = KAFKA_PRODUCER_DELIVERY_TIMEOUT_DEFAULT_SECONDS,
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    pub kafka_producer_delivery_timeout_seconds: u64,
    /// Seconds a shutdown flush may wait for in-flight deliveries.
    #[arg(
        long,
        env = "KAFKA_PRODUCER_FLUSH_TIMEOUT_SECONDS",
        default_value_t = KAFKA_PRODUCER_FLUSH_TIMEOUT_DEFAULT_SECONDS,
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    pub kafka_producer_flush_timeout_seconds: u64,
}

impl KafkaProducerConfig {
    /// Time librdkafka may spend delivering a message before failing it.
    #[must_use]
    pub fn delivery_timeout(&self) -> Duration {
        Duration::from_secs(self.kafka_producer_delivery_timeout_seconds)
    }

    /// Time a shutdown flush may wait for in-flight deliveries.
    #[must_use]
    pub fn flush_timeout(&self) -> Duration {
        Duration::from_secs(self.kafka_producer_flush_timeout_seconds)
    }

    /// Applies the producer settings to an rdkafka client config:
    /// idempotent delivery and the delivery timeout in milliseconds.
    pub(crate) fn apply(&self, config: &mut ClientConfig) {
        config.set(ENABLE_IDEMPOTENCE, "true");
        config.set(
            MESSAGE_TIMEOUT_MS,
            self.delivery_timeout().as_millis().to_string(),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::time::Duration;

    use clap::{CommandFactory, Parser};
    use rdkafka::ClientConfig;

    use super::{
        AUTO_OFFSET_RESET, AutoOffsetReset, BOOTSTRAP_SERVERS, BROKER_ADDRESS_FAMILY, CLIENT_ID,
        ENABLE_AUTO_COMMIT, ENABLE_AUTO_OFFSET_STORE, ENABLE_IDEMPOTENCE, GROUP_ID,
        KAFKA_PRODUCER_DELIVERY_TIMEOUT_DEFAULT_SECONDS,
        KAFKA_PRODUCER_FLUSH_TIMEOUT_DEFAULT_SECONDS, KafkaConfig, KafkaConsumerConfig,
        KafkaProducerConfig, MESSAGE_TIMEOUT_MS,
    };

    #[test]
    fn parses_explicit_args() {
        let config = KafkaConfig::try_parse_from([
            "test",
            "--kafka-bootstrap-servers",
            "h:1",
            "--kafka-client-id",
            "c",
        ])
        .unwrap();
        assert_eq!(config.kafka_bootstrap_servers, "h:1");
        assert_eq!(config.kafka_client_id.as_deref(), Some("c"));
    }

    #[test]
    fn bootstrap_servers_is_required_and_env_backed() {
        let command = KafkaConfig::command();
        let arg = command
            .get_arguments()
            .find(|arg| arg.get_long() == Some("kafka-bootstrap-servers"))
            .expect("kafka_bootstrap_servers argument is defined");
        assert!(arg.is_required_set());
        assert_eq!(arg.get_default_values(), &[] as &[std::ffi::OsString]);
        assert_eq!(arg.get_env(), Some(OsStr::new("KAFKA_BOOTSTRAP_SERVERS")));
    }

    #[test]
    fn consumer_defaults_to_earliest() {
        let config =
            KafkaConsumerConfig::try_parse_from(["test", "--kafka-consumer-group-id", "g"])
                .unwrap();
        assert_eq!(config.kafka_consumer_group_id, "g");
        assert_eq!(
            config.kafka_consumer_auto_offset_reset,
            AutoOffsetReset::Earliest
        );
    }

    #[test]
    fn producer_defaults_and_duration_accessors() {
        let config = KafkaProducerConfig::try_parse_from(["test"]).unwrap();
        assert_eq!(
            config.kafka_producer_delivery_timeout_seconds,
            KAFKA_PRODUCER_DELIVERY_TIMEOUT_DEFAULT_SECONDS
        );
        assert_eq!(
            config.kafka_producer_flush_timeout_seconds,
            KAFKA_PRODUCER_FLUSH_TIMEOUT_DEFAULT_SECONDS
        );
        assert_eq!(config.delivery_timeout(), Duration::from_secs(30));
        assert_eq!(config.flush_timeout(), Duration::from_secs(10));
    }

    #[test]
    fn zero_producer_timeouts_are_rejected() {
        let delivery = KafkaProducerConfig::try_parse_from([
            "test",
            "--kafka-producer-delivery-timeout-seconds",
            "0",
        ]);
        let flush = KafkaProducerConfig::try_parse_from([
            "test",
            "--kafka-producer-flush-timeout-seconds",
            "0",
        ]);
        assert!(delivery.is_err());
        assert!(flush.is_err());
    }

    #[test]
    fn client_config_sets_fixed_settings() {
        let anonymous =
            KafkaConfig::try_parse_from(["test", "--kafka-bootstrap-servers", "h:1"]).unwrap();
        let config = anonymous.client_config();
        assert_eq!(config.get(BOOTSTRAP_SERVERS), Some("h:1"));
        assert_eq!(config.get(BROKER_ADDRESS_FAMILY), Some("v4"));
        assert_eq!(config.get(CLIENT_ID), None);

        let identified = KafkaConfig::try_parse_from([
            "test",
            "--kafka-bootstrap-servers",
            "h:1",
            "--kafka-client-id",
            "c",
        ])
        .unwrap();
        assert_eq!(identified.client_config().get(CLIENT_ID), Some("c"));
    }

    #[test]
    fn consumer_apply_sets_group_and_manual_offset_store() {
        let config =
            KafkaConsumerConfig::try_parse_from(["test", "--kafka-consumer-group-id", "g"])
                .unwrap();
        let mut client = ClientConfig::new();
        config.apply(&mut client);
        assert_eq!(client.get(GROUP_ID), Some("g"));
        assert_eq!(client.get(ENABLE_AUTO_COMMIT), Some("true"));
        assert_eq!(client.get(ENABLE_AUTO_OFFSET_STORE), Some("false"));
        assert_eq!(client.get(AUTO_OFFSET_RESET), Some("earliest"));

        let latest = KafkaConsumerConfig::try_parse_from([
            "test",
            "--kafka-consumer-group-id",
            "g",
            "--kafka-consumer-auto-offset-reset",
            "latest",
        ])
        .unwrap();
        let mut client = ClientConfig::new();
        latest.apply(&mut client);
        assert_eq!(client.get(AUTO_OFFSET_RESET), Some("latest"));
    }

    #[test]
    fn producer_apply_sets_idempotence_and_message_timeout() {
        let config = KafkaProducerConfig::try_parse_from([
            "test",
            "--kafka-producer-delivery-timeout-seconds",
            "2",
        ])
        .unwrap();
        let mut client = ClientConfig::new();
        config.apply(&mut client);
        assert_eq!(client.get(ENABLE_IDEMPOTENCE), Some("true"));
        assert_eq!(client.get(MESSAGE_TIMEOUT_MS), Some("2000"));
    }

    #[test]
    fn all_configs_flatten_together() {
        #[derive(Parser)]
        struct Args {
            #[command(flatten)]
            kafka: KafkaConfig,
            #[command(flatten)]
            consumer: KafkaConsumerConfig,
            #[command(flatten)]
            producer: KafkaProducerConfig,
        }
        let args = Args::try_parse_from([
            "test",
            "--kafka-bootstrap-servers",
            "h:1",
            "--kafka-consumer-group-id",
            "g",
        ])
        .unwrap();
        assert_eq!(args.kafka.kafka_bootstrap_servers, "h:1");
        assert_eq!(args.consumer.kafka_consumer_group_id, "g");
        assert_eq!(args.producer.kafka_producer_flush_timeout_seconds, 10);
    }
}
