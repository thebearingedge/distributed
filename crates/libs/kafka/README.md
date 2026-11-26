# kafka

Typed JSON producer and consumer for Kafka on
[rdkafka](https://crates.io/crates/rdkafka), over PLAINTEXT only — transport
security is provided in-cluster by Cilium mutual authentication. Depends on
[`messaging`](../messaging/src/messaging_error.rs), [`runtime`](../runtime/src/run_error.rs),
and [`telemetry`](../telemetry/src/propagation.rs), never on a domain. There
are no root re-exports, so callers use module paths such as
`kafka::producer::KafkaProducer`.

## Contents

- [`config`](src/config.rs) — [`KafkaConfig`](src/config.rs),
  [`KafkaConsumerConfig`](src/config.rs), and
  [`KafkaProducerConfig`](src/config.rs), CLI/env configuration parsed with
  `clap::Parser`, plus the [`AutoOffsetReset`](src/config.rs) value enum.
- [`producer`](src/producer.rs) — [`KafkaProducer`](src/producer.rs), an
  idempotent JSON producer for one topic, and the [`Ack`](src/producer.rs)
  it returns per delivery.
- [`consumer`](src/consumer.rs) — [`KafkaConsumer`](src/consumer.rs), the
  [`MessageHandler`](src/consumer.rs) port, and the
  [`Delivery`](src/consumer.rs) / [`DeliveryMeta`](src/consumer.rs) a
  handler receives.
- [`admin`](src/admin.rs) — [`create_topic`](src/admin.rs) and
  [`delete_topic`](src/admin.rs), idempotent topic administration helpers.
- [`kafka_error`](src/kafka_error.rs) — the error types
  [`KafkaClientError`](src/kafka_error.rs), [`SendError`](src/kafka_error.rs),
  [`ConsumeError`](src/kafka_error.rs), [`KafkaAdminError`](src/kafka_error.rs),
  and their classification.
- [`headers`](src/headers.rs) — OpenTelemetry
  [`Injector`](src/headers.rs)/[`Extractor`](src/headers.rs) adapters over
  rdkafka message headers for W3C trace propagation.

## Configuration

| Env var | Default | Meaning |
|---|---|---|
| `KAFKA_BOOTSTRAP_SERVERS` | — (required) | Comma-separated `host:port` bootstrap brokers. |
| `KAFKA_CLIENT_ID` | librdkafka's default | Client id reported to brokers. |
| `KAFKA_CONSUMER_GROUP_ID` | — (required) | Consumer group id. |
| `KAFKA_CONSUMER_AUTO_OFFSET_RESET` | `earliest` | Where a group with no committed offset starts reading; `earliest` or `latest`. |
| `KAFKA_PRODUCER_DELIVERY_TIMEOUT_SECONDS` | `30` | Seconds librdkafka may spend delivering a message before failing it; must be at least 1. |
| `KAFKA_PRODUCER_FLUSH_TIMEOUT_SECONDS` | `10` | Seconds a shutdown flush may wait for in-flight deliveries; must be at least 1. |

CLI flags (`--kafka-bootstrap-servers`, `--kafka-client-id`,
`--kafka-consumer-group-id`, `--kafka-consumer-auto-offset-reset`,
`--kafka-producer-delivery-timeout-seconds`,
`--kafka-producer-flush-timeout-seconds`) override env vars, which override
these defaults. Topic names are code constants owned by the domain; there is
no arbitrary librdkafka settings passthrough.

The client config also fixes settings no config can change:

| librdkafka setting | Value | Meaning |
|---|---|---|
| `broker.address.family` | `v4` | Only IPv4 broker addresses are used. |
| `enable.idempotence` | `true` | The producer delivers exactly once per partition session. |
| `enable.auto.commit` | `true` | librdkafka periodically commits the stored offsets. |
| `enable.auto.offset.store` | `false` | Offsets are stored explicitly, after the handler succeeds. |

## Delivery semantics

- **At-least-once delivery.** Each consumer handles one message at a time:
  a message's offset is stored only after its handler succeeds, and a final
  sync commit runs when the consumer closes on shutdown. Anything not stored
  or committed is redelivered on the next run.
- **Producer is idempotent** and [`send`](src/producer.rs) waits for the
  broker's delivery acknowledgement, bounded by
  `KAFKA_PRODUCER_DELIVERY_TIMEOUT_SECONDS`.
- **Undecodable or empty payloads are skipped:** they are logged, acked by
  storing the offset, and never reach the handler.
- **A handler error stops consumption** without storing the failing
  message's offset, so the message is redelivered after the restart.
- **A fatal client error stops consumption**, which classifies as a
  retryable [`RunError`](../runtime/src/run_error.rs); the client must be
  recreated. Transient consumer errors are logged at `warn` and the loop
  continues.
- **No transactions or exactly-once delivery.** Message keys are passed by
  the caller as `Option<&[u8]>`.

## Error classification

Producing errors classify into
[`messaging::MessagingError`](../messaging/src/messaging_error.rs), which
domain ports return; the whole `SendError` is kept as the source:

| `SendError` variant | Classifies as | Meaning |
|---|---|---|
| `Encode` | `Rejected` | The payload is not valid JSON for the type. |
| `Delivery` with the broker refusing the message (oversized, invalid, unknown topic, authorization, …) | `Rejected` | Retrying the same message cannot succeed. |
| `Delivery` with a transient or fatal broker failure | `Unavailable` | A restart can recover from it. |
| `Flush` / `FlushTask` | `Unavailable` | The shutdown flush failed or the blocking task panicked. |

Consumption and client-construction errors classify into
[`runtime::run_error::RunError`](../runtime/src/run_error.rs), which drives
the process exit status:

| Source | Classifies as | Meaning |
|---|---|---|
| `ConsumeError::Handler` | The handler's own `RunError`, unchanged | Retryable exits `1`, permanent exits `2`. |
| `ConsumeError::Fatal` / `Commit` / `CloseTask` | `RunError::retryable` | A restart can recover from it. |
| `KafkaClientError` | `RunError::permanent` | A bad client config or subscription cannot be fixed by a restart. |

[`KafkaAdminError`](src/kafka_error.rs) covers the topic administration
helpers and has no further classification; it is only used at boot.

## Tracing

- The producer opens a `kafka.send` span per message
  (`otel.kind = "producer"`, `messaging.system = "kafka"`,
  `messaging.operation.type = "send"`,
  `messaging.destination.name = <topic>`) and injects the current W3C
  `traceparent` into the message headers.
- The consumer opens a `kafka.process` span per message
  (`otel.kind = "consumer"`, `messaging.operation.type = "process"`, the
  topic, partition id, and offset) and links it to the parent trace context
  carried in the message's `traceparent` header.
- librdkafka's own logs reach `tracing` through rdkafka's `tracing`
  feature.

## Building

librdkafka is compiled from source (`cmake-build`), so building the crate
needs CMake and a C/C++ toolchain. They are declared in
[`tools/Brewfile`](../../../tools/Brewfile) for local development and
installed in the build image by
[`tools/docker/Dockerfile.rust-builder`](../../../tools/docker/Dockerfile.rust-builder).
The first build takes a while; after that it is cached. Local Kafka runs
over PLAINTEXT via `infrastructure/docker-compose.yaml`, with
`KAFKA_BOOTSTRAP_SERVERS` set in `tools/envs/kafka.env`.

## Composing a consumer worker

`Worker` and `WorkerContext` live in the
[`worker`](../worker/src/worker.rs) lib. Build the Kafka clients inside
`Worker::run` — never earlier — because a rdkafka client captures the log
level at construction and `worker::launch` initializes telemetry first.
Map the lib's errors with `RunError::from`:

```rust
use async_trait::async_trait;
use kafka::config::{KafkaConfig, KafkaConsumerConfig};
use kafka::consumer::{Delivery, KafkaConsumer, MessageHandler};
use runtime::run_error::RunError;
use serde::Deserialize;
use worker::{Worker, WorkerContext};

#[derive(Deserialize)]
struct GreetingPublished {
    text: String,
}

struct LogGreeting;

#[async_trait]
impl MessageHandler<GreetingPublished> for LogGreeting {
    async fn handle(&self, delivery: Delivery<GreetingPublished>) -> Result<(), RunError> {
        tracing::info!(text = %delivery.value.text, "greeting consumed");
        Ok(())
    }
}

struct GreetingConsumer {
    kafka: KafkaConfig,
    consumer: KafkaConsumerConfig,
}

#[async_trait]
impl Worker for GreetingConsumer {
    fn name(&self) -> &'static str {
        "greeting_consumer"
    }

    async fn run(&self, context: WorkerContext) -> Result<(), RunError> {
        let consumer =
            KafkaConsumer::new(&self.kafka, &self.consumer, "greetings").map_err(RunError::from)?;
        consumer
            .run(&LogGreeting, context.shutdown(), context.heartbeat())
            .await
            .map_err(RunError::from)
    }
}
```

The consumer beats the heartbeat on every loop iteration, on idle ticks,
and while the handler runs, and returns once `context.shutdown()` is
triggered.
