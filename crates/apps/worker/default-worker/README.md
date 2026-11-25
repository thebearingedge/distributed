# default-worker

A placeholder worker image: it runs the inert worker, which heartbeats so
`/livez` stays `200`, serves `/readyz` (which flips to `503` once SIGTERM
arrives), and exits `0` on a clean shutdown — reserving the default
Kubernetes worker Deployment slot (port `3000`; env vars
`HEALTH_LISTEN_ADDRESS`, `HEALTH_CHECK_TIMEOUT_SECONDS`,
`HEARTBEAT_STALE_SECONDS`) so deployments can swap in real workers without
touching the wiring. See
[`crates/libs/worker/README.md`](../../../libs/worker/README.md) for probe
semantics, configuration, and exit codes; build the image with
`just docker-build default-worker`.
