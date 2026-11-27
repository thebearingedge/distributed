# greetings-consumer

A worker that consumes greetings from the `greetings` Kafka topic with
at-least-once delivery — each message is committed after its handler runs,
blank greetings are logged, acked, and skipped, and a failed handler stops
consumption without committing that message's offset, so it is redelivered on
restart. It serves `/livez` and `/readyz` on port `3000` by default (env vars
`HEALTH_LISTEN_ADDRESS`, `HEALTH_CHECK_TIMEOUT_SECONDS`,
`HEARTBEAT_STALE_SECONDS`, `SHUTDOWN_DRAIN_SECONDS`, `SHUTDOWN_TIMEOUT_SECONDS`,
`SHUTDOWN_FLUSH_TIMEOUT_SECONDS`) and requires `KAFKA_CONSUMER_GROUP_ID` plus
`KAFKA_BOOTSTRAP_SERVERS` (`KAFKA_CLIENT_ID`,
`KAFKA_CONSUMER_AUTO_OFFSET_RESET`, `KAFKA_CONSUMER_STATISTICS_INTERVAL_SECONDS`
optional); see
[`crates/libs/kafka/README.md`](../../../libs/kafka/README.md) for the
`KAFKA_*` vars and [`crates/libs/worker/README.md`](../../../libs/worker/README.md)
for probe semantics, configuration, and exit codes. It emits the
`kafka::metrics` consumer instruments — consumed messages, process duration,
skipped messages, and consumer lag — through the worker's meter provider. Run
it with `just cargo-run greetings-consumer`; build the image with
`just docker-build greetings-consumer`.
