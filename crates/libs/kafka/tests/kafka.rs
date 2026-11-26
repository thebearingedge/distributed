//! Broker-backed integration tests for the kafka producer, consumer and
//! admin helpers.

use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use clap::Parser;
use rdkafka::ClientConfig;
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::util::Timeout;
use serde::{Deserialize, Serialize};

use kafka::admin::{create_topic, delete_topic};
use kafka::config::{
    AutoOffsetReset, KAFKA_PRODUCER_DELIVERY_TIMEOUT_DEFAULT_SECONDS,
    KAFKA_PRODUCER_FLUSH_TIMEOUT_DEFAULT_SECONDS, KafkaConfig, KafkaConsumerConfig,
    KafkaProducerConfig,
};
use kafka::consumer::{Delivery, KafkaConsumer, MessageHandler};
use kafka::kafka_error::ConsumeError;
use kafka::producer::KafkaProducer;
use runtime::exit_status::ExitStatus;
use runtime::heartbeat::Heartbeat;
use runtime::run_error::RunError;
use runtime::shutdown::Shutdown;

fn kafka_config() -> KafkaConfig {
    KafkaConfig::try_parse_from(["test"])
        .expect("KAFKA_BOOTSTRAP_SERVERS must be set (tools/envs/kafka.env)")
}

fn consumer_config(group: &str) -> KafkaConsumerConfig {
    KafkaConsumerConfig {
        kafka_consumer_group_id: group.to_owned(),
        kafka_consumer_auto_offset_reset: AutoOffsetReset::Earliest,
    }
}

fn producer_config() -> KafkaProducerConfig {
    KafkaProducerConfig {
        kafka_producer_delivery_timeout_seconds: KAFKA_PRODUCER_DELIVERY_TIMEOUT_DEFAULT_SECONDS,
        kafka_producer_flush_timeout_seconds: KAFKA_PRODUCER_FLUSH_TIMEOUT_DEFAULT_SECONDS,
    }
}

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::now_v7())
}

async fn fresh_topic() -> String {
    let topic = unique("topic");
    create_topic(&kafka_config(), &topic, 1, 1)
        .await
        .expect("topic is created");
    topic
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct TestEvent {
    seq: u32,
}

struct Collect {
    seen: Mutex<Vec<(u32, i64)>>,
    stop_after: usize,
    shutdown: Shutdown,
    fail_on_seq: Option<u32>,
}

#[async_trait]
impl MessageHandler<TestEvent> for Collect {
    async fn handle(&self, delivery: Delivery<TestEvent>) -> Result<(), RunError> {
        if self.fail_on_seq == Some(delivery.value.seq) {
            return Err(RunError::retryable("injected"));
        }
        let stop = {
            let mut seen = self.seen.lock().expect("seen is not poisoned");
            seen.push((delivery.value.seq, delivery.meta.offset));
            seen.len() == self.stop_after
        };
        if stop {
            self.shutdown.trigger();
        }
        Ok(())
    }
}

async fn consume(
    topic: &str,
    group: &str,
    stop_after: usize,
    fail_on_seq: Option<u32>,
) -> (Result<(), ConsumeError>, Vec<(u32, i64)>) {
    let consumer: KafkaConsumer<TestEvent> =
        KafkaConsumer::new(&kafka_config(), &consumer_config(group), topic)
            .expect("consumer builds");
    let shutdown = Shutdown::new();
    let heartbeat = Heartbeat::new(Duration::from_secs(30));
    let handler = Collect {
        seen: Mutex::new(Vec::new()),
        stop_after,
        shutdown: shutdown.clone(),
        fail_on_seq,
    };
    let result = tokio::time::timeout(
        Duration::from_secs(60),
        consumer.run(&handler, &shutdown, &heartbeat),
    )
    .await
    .expect("consumer finished in time");
    let seen = handler.seen.into_inner().expect("seen is not poisoned");
    (result, seen)
}

#[tokio::test]
async fn create_topic_is_idempotent_and_delete_tolerates_missing() {
    let kafka = kafka_config();
    let topic = unique("topic");
    create_topic(&kafka, &topic, 1, 1)
        .await
        .expect("first create succeeds");
    create_topic(&kafka, &topic, 1, 1)
        .await
        .expect("second create succeeds");
    delete_topic(&kafka, &topic)
        .await
        .expect("first delete succeeds");
    delete_topic(&kafka, &topic)
        .await
        .expect("second delete succeeds");
}

#[tokio::test]
async fn send_acks_sequential_offsets() {
    let kafka = kafka_config();
    let topic = fresh_topic().await;
    let producer: KafkaProducer<TestEvent> =
        KafkaProducer::new(&kafka, &producer_config(), topic.as_str()).expect("producer builds");

    let mut acks = Vec::new();
    for seq in 0..3 {
        acks.push(
            producer
                .send(None, &TestEvent { seq })
                .await
                .expect("send succeeds"),
        );
    }

    assert_eq!(
        acks.iter().map(|ack| ack.partition).collect::<Vec<_>>(),
        [0, 0, 0]
    );
    assert_eq!(
        acks.iter().map(|ack| ack.offset).collect::<Vec<_>>(),
        [0, 1, 2]
    );

    delete_topic(&kafka, &topic)
        .await
        .expect("topic is deleted");
}

#[tokio::test]
async fn produced_messages_are_consumed_in_order() {
    let kafka = kafka_config();
    let topic = fresh_topic().await;
    let producer: KafkaProducer<TestEvent> =
        KafkaProducer::new(&kafka, &producer_config(), topic.as_str()).expect("producer builds");
    for seq in 0..3 {
        producer
            .send(None, &TestEvent { seq })
            .await
            .expect("send succeeds");
    }

    let (result, seen) = consume(&topic, &unique("group"), 3, None).await;
    assert!(result.is_ok());
    assert_eq!(
        seen.iter().map(|(seq, _)| *seq).collect::<Vec<_>>(),
        [0, 1, 2]
    );

    delete_topic(&kafka, &topic)
        .await
        .expect("topic is deleted");
}

#[tokio::test]
async fn restarted_consumer_resumes_after_the_last_processed_message() {
    let kafka = kafka_config();
    let topic = fresh_topic().await;
    let producer: KafkaProducer<TestEvent> =
        KafkaProducer::new(&kafka, &producer_config(), topic.as_str()).expect("producer builds");
    for seq in 0..3 {
        producer
            .send(None, &TestEvent { seq })
            .await
            .expect("send succeeds");
    }

    let group = unique("group");
    let (result, seen) = consume(&topic, &group, 2, None).await;
    assert!(result.is_ok());
    assert_eq!(seen.iter().map(|(seq, _)| *seq).collect::<Vec<_>>(), [0, 1]);

    let (result, seen) = consume(&topic, &group, 1, None).await;
    assert!(result.is_ok());
    assert_eq!(seen.iter().map(|(seq, _)| *seq).collect::<Vec<_>>(), [2]);

    delete_topic(&kafka, &topic)
        .await
        .expect("topic is deleted");
}

#[tokio::test]
async fn handler_failure_stops_without_acking_and_redelivers() {
    let kafka = kafka_config();
    let topic = fresh_topic().await;
    let producer: KafkaProducer<TestEvent> =
        KafkaProducer::new(&kafka, &producer_config(), topic.as_str()).expect("producer builds");
    producer
        .send(None, &TestEvent { seq: 0 })
        .await
        .expect("send succeeds");

    let group = unique("group");
    let (result, _) = consume(&topic, &group, 1, Some(0)).await;
    let error = result.expect_err("handler failure stops the consumer");
    assert!(matches!(error, ConsumeError::Handler { offset: 0, .. }));
    assert_eq!(RunError::from(error).exit_status(), ExitStatus::Retryable);

    let (result, seen) = consume(&topic, &group, 1, None).await;
    assert!(result.is_ok());
    assert_eq!(seen.iter().map(|(seq, _)| *seq).collect::<Vec<_>>(), [0]);

    delete_topic(&kafka, &topic)
        .await
        .expect("topic is deleted");
}

#[tokio::test]
async fn undecodable_and_empty_payloads_are_skipped() {
    let kafka = kafka_config();
    let topic = fresh_topic().await;

    let mut raw_config = ClientConfig::new();
    raw_config.set("bootstrap.servers", kafka.kafka_bootstrap_servers.as_str());
    raw_config.set("broker.address.family", "v4");
    let raw: FutureProducer = raw_config.create().expect("raw producer builds");
    raw.send(
        FutureRecord::<[u8], [u8]>::to(topic.as_str()).payload(b"not json"),
        Timeout::Never,
    )
    .await
    .expect("undecodable payload is delivered");
    raw.send(FutureRecord::<(), ()>::to(topic.as_str()), Timeout::Never)
        .await
        .expect("empty payload is delivered");

    let producer: KafkaProducer<TestEvent> =
        KafkaProducer::new(&kafka, &producer_config(), topic.as_str()).expect("producer builds");
    producer
        .send(None, &TestEvent { seq: 7 })
        .await
        .expect("send succeeds");

    let (result, seen) = consume(&topic, &unique("group"), 1, None).await;
    assert!(result.is_ok());
    assert_eq!(seen, vec![(7, 2)]);

    delete_topic(&kafka, &topic)
        .await
        .expect("topic is deleted");
}

#[tokio::test]
async fn consumer_stops_on_shutdown_with_an_idle_topic() {
    let kafka = kafka_config();
    let topic = fresh_topic().await;
    let group = unique("group");
    let consumer: KafkaConsumer<TestEvent> =
        KafkaConsumer::new(&kafka, &consumer_config(&group), topic.as_str())
            .expect("consumer builds");
    let shutdown = Shutdown::new();
    let heartbeat = Heartbeat::new(Duration::from_secs(30));
    let handler = Collect {
        seen: Mutex::new(Vec::new()),
        stop_after: usize::MAX,
        shutdown: shutdown.clone(),
        fail_on_seq: None,
    };
    let run_shutdown = shutdown.clone();
    let run_heartbeat = heartbeat.clone();
    let task =
        tokio::spawn(async move { consumer.run(&handler, &run_shutdown, &run_heartbeat).await });

    // wait for the group join so shutdown races an assigned consumer
    tokio::time::sleep(Duration::from_secs(2)).await;
    shutdown.trigger();

    let outcome = tokio::time::timeout(Duration::from_secs(60), task)
        .await
        .expect("consumer finished in time")
        .expect("consumer task does not panic");
    assert!(outcome.is_ok());
    assert!(heartbeat.check().is_ok());

    delete_topic(&kafka, &topic)
        .await
        .expect("topic is deleted");
}

#[tokio::test]
async fn flush_succeeds_after_sends() {
    let kafka = kafka_config();
    let topic = fresh_topic().await;
    let producer: KafkaProducer<TestEvent> =
        KafkaProducer::new(&kafka, &producer_config(), topic.as_str()).expect("producer builds");
    for seq in 0..3 {
        producer
            .send(None, &TestEvent { seq })
            .await
            .expect("send succeeds");
    }

    producer.flush().await.expect("flush succeeds");

    delete_topic(&kafka, &topic)
        .await
        .expect("topic is deleted");
}
