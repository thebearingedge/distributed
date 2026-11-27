# kafka

Typed JSON producer and consumer for Kafka on
[rdkafka](https://crates.io/crates/rdkafka), over PLAINTEXT only — transport
security is provided in-cluster by Cilium mutual authentication. Depends on
[`messaging`](../messaging/src/messaging_error.rs), [`runtime`](../runtime/src/run_error.rs),
and [`telemetry`](../telemetry/src/propagation.rs) — whose
[`metrics`](../telemetry/src/metrics.rs) helpers and the `opentelemetry` API
crate it uses for its metric instruments — never on a domain. There are no
root re-exports, so callers use module paths such as
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
- [`metrics`](src/metrics.rs) — [`ProducerMetrics`](src/metrics/producer_metrics.rs)
  and [`ConsumerMetrics`](src/metrics/consumer_metrics.rs), messaging metric
  handles built once per meter provider and passed to
  [`KafkaProducer::new`](src/producer.rs) /
  [`KafkaConsumer::new`](src/consumer.rs).
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
| `KAFKA_CONSUMER_STATISTICS_INTERVAL_SECONDS` | `15` | Seconds between librdkafka statistics reports, which feed the consumer lag gauge; must be at least 1. |
| `KAFKA_PRODUCER_DELIVERY_TIMEOUT_SECONDS` | `30` | Seconds librdkafka may spend delivering a message before failing it; must be at least 1. |
| `KAFKA_PRODUCER_FLUSH_TIMEOUT_SECONDS` | `10` | Seconds a shutdown flush may wait for in-flight deliveries; must be at least 1. |

CLI flags (`--kafka-bootstrap-servers`, `--kafka-client-id`,
`--kafka-consumer-group-id`, `--kafka-consumer-auto-offset-reset`,
`--kafka-consumer-statistics-interval-seconds`,
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

Only consumers set `statistics.interval.ms`, derived from
`KAFKA_CONSUMER_STATISTICS_INTERVAL_SECONDS`; producers and admin clients
emit no statistics.

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
  (`otel.kind = "producer"`, `messaging.system.name = "kafka"`,
  `messaging.operation.type = "send"`,
  `messaging.destination.name = <topic>`) and injects the current W3C
  `traceparent` into the message headers.
- The consumer opens a `kafka.process` span per message
  (`otel.kind = "consumer"`, `messaging.system.name = "kafka"`,
  `messaging.operation.type = "process"`,
  `messaging.destination.name = <topic>`,
  `messaging.destination.partition.id` recorded as a string, and
  `kafka.offset`) and links it to the parent trace context carried in the
  message's `traceparent` header.
- librdkafka's own logs reach `tracing` through rdkafka's `tracing`
  feature.

## Metrics

[`ProducerMetrics`](src/metrics/producer_metrics.rs) and
[`ConsumerMetrics`](src/metrics/consumer_metrics.rs) hold the messaging
instruments. Build them once per meter provider with
`ProducerMetrics::new(provider)` / `ConsumerMetrics::new(provider)` and pass
them by reference into [`KafkaProducer::new`](src/producer.rs) /
[`KafkaConsumer::new`](src/consumer.rs). The instruments are created under
the `kafka` instrumentation scope of the provider that was passed in — never
the global one — and domain adapters take `&ProducerMetrics` /
`&ConsumerMetrics`, so domains never depend on `opentelemetry`. Every
instrument also carries `messaging.system.name = kafka` and
`messaging.destination.name = <topic>`.

| Instrument | Type | Unit | Additional attributes |
|---|---|---|---|
| `messaging.client.sent.messages` | `Counter<u64>` | `{message}` | `messaging.operation.name = send`, `messaging.operation.type = send`; on success `messaging.destination.partition.id`, on failure `error.type` |
| `messaging.client.operation.duration` | duration histogram | `s` | as for `messaging.client.sent.messages`; measures the whole `send()` call, including JSON encoding |
| `messaging.client.consumed.messages` | `Counter<u64>` | `{message}` | `messaging.consumer.group.name`, `messaging.operation.name = poll`, `messaging.destination.partition.id`; counted for every received message, skipped ones included |
| `messaging.process.duration` | duration histogram | `s` | `messaging.consumer.group.name`, `messaging.operation.name = process`, `messaging.destination.partition.id`; on a handler failure `error.type` |
| `messaging.kafka.consumer.skipped.messages` (custom) | `Counter<u64>` | `{message}` | `messaging.consumer.group.name`, `messaging.destination.partition.id`, `error.type` |
| `messaging.kafka.consumer.lag` (custom) | observable gauge (`i64`) | `{message}` | `messaging.consumer.group.name`, `messaging.destination.partition.id` |

The first four follow the OpenTelemetry messaging semantic conventions; the
two `messaging.kafka.consumer.*` instruments are custom to this crate.

The `error.type` attribute is always bounded:

- A failed send records `encode` when the payload could not be
  JSON-encoded, the librdkafka error-code name (for example
  `MessageTimedOut`) when the delivery failed, or `_OTHER` when there is no
  code — including a failed shutdown flush.
- A failed handler run records `retryable` or `permanent`, the
  [`RunError`](../runtime/src/run_error.rs) variant the handler returned.
- A skipped message records `empty_payload` or `invalid_json`.

### Consumer lag

`messaging.kafka.consumer.lag` is fed by librdkafka statistics, which arrive
every `KAFKA_CONSUMER_STATISTICS_INTERVAL_SECONDS`:

- Only assigned partitions are reported.
- The value is the lag behind the stored offset, falling back to the lag
  behind the committed offset right after an assignment.
- Unknown lag is not reported; `-1` is never exported.
- Revoked partitions and closed consumers drop out: each statistics report
  replaces the consumer's snapshot, and a dropped consumer's points
  disappear. A report that fails to parse logs a warning and clears the
  snapshot.

Statistics are only emitted while the consumer waits for messages, so the
gauge can trail reality by one statistics interval plus one handler run.
Build one `ConsumerMetrics` per meter provider: each `ConsumerMetrics::new`
registers a lag callback for that provider's lifetime, which observes every
live consumer built from the handle and prunes dropped ones.

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
Build the metrics handles there too, from `context.meter_provider()`, which
`worker::launch` fills in from the telemetry it initializes. Map the lib's
errors with `RunError::from`:

```rust
use async_trait::async_trait;
use kafka::config::{KafkaConfig, KafkaConsumerConfig};
use kafka::consumer::{Delivery, KafkaConsumer, MessageHandler};
use kafka::metrics::ConsumerMetrics;
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
        let metrics = ConsumerMetrics::new(context.meter_provider());
        let consumer = KafkaConsumer::new(
            &self.kafka,
            &self.consumer,
            "greetings",
            &metrics,
        )
        .map_err(RunError::from)?;
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

## Producing from an API

An API already owns its `telemetry`, so it passes
`telemetry.meter_provider()` straight to `ProducerMetrics::new`:

```rust
use clap::Parser;
use kafka::config::{KafkaConfig, KafkaProducerConfig};
use kafka::kafka_error::KafkaClientError;
use kafka::metrics::ProducerMetrics;
use kafka::producer::KafkaProducer;
use serde::Serialize;
use telemetry::Telemetry;

#[derive(Serialize)]
struct OrderPlaced {
    id: String,
}

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    kafka: KafkaConfig,
    #[command(flatten)]
    producer: KafkaProducerConfig,
}

async fn main() -> Result<(), KafkaClientError> {
    let args = Args::parse();
    let telemetry = Telemetry::init("orders_api");
    let producer_metrics = ProducerMetrics::new(telemetry.meter_provider());
    let producer: KafkaProducer<OrderPlaced> =
        KafkaProducer::new(&args.kafka, &args.producer, "orders", &producer_metrics)?;
    let ack = producer
        .send(None, &OrderPlaced { id: "1".into() })
        .await
        .expect("delivery failure");
    tracing::info!(partition = ack.partition, offset = ack.offset, "order placed");
    Ok(())
}
```

Sends classify into [`messaging::MessagingError`](../messaging/src/messaging_error.rs)
like any other producer call site; jobs get their provider from
`context.meter_provider()` of the [`JobContext`](../jobs/src/job.rs) a
`Job::run` receives.
