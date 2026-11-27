# default-worker

A placeholder worker image: the app value is
[`InertWorker`](src/inert_worker.rs), defined in the
crate's lib target (`default_worker`); `src/main.rs` only parses its args and
calls `worker::launch("default_worker", InertWorker, &args.worker)`. The inert
worker heartbeats so
`/livez` stays `200`, serves `/readyz` (which flips to `503` once SIGTERM
arrives), and exits `0` on a clean shutdown — reserving the default
Kubernetes worker Deployment slot (port `3000`; env vars
`HEALTH_LISTEN_ADDRESS`, `HEALTH_CHECK_TIMEOUT_SECONDS`,
`HEARTBEAT_STALE_SECONDS`, `SHUTDOWN_DRAIN_SECONDS`, `SHUTDOWN_TIMEOUT_SECONDS`,
`SHUTDOWN_FLUSH_TIMEOUT_SECONDS`) so deployments can swap in real workers
without touching the wiring. See
[`crates/libs/worker/README.md`](../../../libs/worker/README.md) for probe
semantics, configuration, and exit codes; build the image with
`just docker-build default-worker`.
