# default-job

A placeholder job image: it initializes telemetry, logs `"job started"` and
`"job finished"`, runs a no-op job, and exits with `0`. It reserves the
default Kubernetes Job / CronJob slot so deployments can swap in real jobs
without touching the wiring. See
[`crates/libs/jobs/README.md`](../../../libs/jobs/README.md) for exit codes,
shutdown semantics, and the lifecycle env vars
(`SHUTDOWN_DRAIN_SECONDS`, `SHUTDOWN_TIMEOUT_SECONDS`,
`SHUTDOWN_FLUSH_TIMEOUT_SECONDS`, all flattening
`runtime::config::LifecycleConfig` into this binary's `Args`); build the
image with `just docker-build default-job`.
