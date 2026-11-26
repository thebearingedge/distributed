# greetings-producer

A worker that publishes a greeting to the `greetings` topic every
`GREETING_INTERVAL_SECONDS` (default `3`, message `GREETING_MESSAGE`, default
`Hello, Kafka!`), heartbeating on an idle tick and while each send runs so
`/livez` stays `200`, and serving `/readyz` (which flips to `503` once SIGTERM
arrives) on port `3000` by default (`HEALTH_LISTEN_ADDRESS`); on shutdown it
flushes in-flight deliveries before exiting. Kafka connection and producer
settings come from the `KAFKA_*` environment variables — see
[`crates/libs/kafka/README.md`](../../../libs/kafka/README.md) for the full
list — and
[`crates/libs/worker/README.md`](../../../libs/worker/README.md) documents
probe semantics, configuration, and exit codes. Run it locally with
`just cargo-run greetings-producer`, or build the image with
`just docker-build greetings-producer`.
