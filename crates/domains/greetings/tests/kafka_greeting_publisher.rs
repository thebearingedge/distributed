//! Integration tests for the [`greetings::KafkaGreetingPublisher`] Kafka
//! adapter.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use uuid::Uuid;

use greetings::{GREETINGS_TOPIC, KafkaGreetingPublisher, SendGreetingInteractor};
use kafka::config::{
    AutoOffsetReset, KAFKA_PRODUCER_DELIVERY_TIMEOUT_DEFAULT_SECONDS,
    KAFKA_PRODUCER_FLUSH_TIMEOUT_DEFAULT_SECONDS, KafkaConfig, KafkaConsumerConfig,
    KafkaProducerConfig,
};
use kafka::consumer::{Delivery, KafkaConsumer, MessageHandler};
use runtime::heartbeat::Heartbeat;
use runtime::run_error::RunError;
use runtime::shutdown::Shutdown;

fn kafka_config() -> KafkaConfig {
    KafkaConfig {
        kafka_bootstrap_servers: std::env::var("KAFKA_BOOTSTRAP_SERVERS")
            .expect("KAFKA_BOOTSTRAP_SERVERS must be set (tools/envs/greetings.env)"),
        kafka_client_id: None,
    }
}

fn producer_config() -> KafkaProducerConfig {
    KafkaProducerConfig {
        kafka_producer_delivery_timeout_seconds: KAFKA_PRODUCER_DELIVERY_TIMEOUT_DEFAULT_SECONDS,
        kafka_producer_flush_timeout_seconds: KAFKA_PRODUCER_FLUSH_TIMEOUT_DEFAULT_SECONDS,
    }
}

struct Capture {
    greeting_id: String,
    shutdown: Shutdown,
    captured: Mutex<Option<(Value, Option<Vec<u8>>)>>,
}

#[async_trait]
impl MessageHandler<Value> for Capture {
    async fn handle(&self, delivery: Delivery<Value>) -> Result<(), RunError> {
        if delivery.value.get("id").and_then(Value::as_str) == Some(self.greeting_id.as_str()) {
            let mut captured = self.captured.lock().expect("captured is not poisoned");
            *captured = Some((delivery.value, delivery.meta.key));
            self.shutdown.trigger();
        }
        Ok(())
    }
}

#[tokio::test]
async fn publishes_greetings_as_json_on_the_greetings_topic() {
    let kafka = kafka_config();
    let publisher =
        KafkaGreetingPublisher::connect(&kafka, &producer_config()).expect("publisher connects");
    let interactor = SendGreetingInteractor::new(Arc::new(publisher));

    let greeting = interactor
        .send("hello from test")
        .await
        .expect("greeting is published");

    let consumer: KafkaConsumer<Value> = KafkaConsumer::new(
        &kafka,
        &KafkaConsumerConfig {
            kafka_consumer_group_id: format!("test-{}", Uuid::now_v7()),
            kafka_consumer_auto_offset_reset: AutoOffsetReset::Earliest,
        },
        GREETINGS_TOPIC,
    )
    .expect("consumer builds");

    let shutdown = Shutdown::new();
    let heartbeat = Heartbeat::new(Duration::from_secs(30));
    let capture = Capture {
        greeting_id: greeting.id.to_string(),
        shutdown: shutdown.clone(),
        captured: Mutex::new(None),
    };

    tokio::time::timeout(
        Duration::from_secs(60),
        consumer.run(&capture, &shutdown, &heartbeat),
    )
    .await
    .expect("consumer finished in time")
    .expect("consumer stops on shutdown");

    let (value, key) = capture
        .captured
        .into_inner()
        .expect("captured is not poisoned")
        .expect("the greeting was delivered");
    assert_eq!(
        value,
        json!({ "id": greeting.id.to_string(), "message": "hello from test" })
    );
    assert_eq!(key.as_deref(), Some(greeting.id.as_bytes().as_slice()));
}
