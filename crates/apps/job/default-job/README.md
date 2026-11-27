# default-job

A placeholder job image: the app value is
[`NoopJob`](src/noop_job.rs), defined in the crate's
lib target (`default_job`); `src/main.rs` only parses its args and calls
`jobs::launch("default_job", NoopJob, &args.lifecycle)`. It initializes
telemetry, logs `"job started"` and `"job finished"`, runs the no-op job, and
exits with `0`. It reserves the
default Kubernetes Job / CronJob slot so deployments can swap in real jobs
without touching the wiring. See
[`crates/libs/jobs/README.md`](../../../libs/jobs/README.md) for exit codes,
shutdown semantics, and the lifecycle env vars
(`SHUTDOWN_DRAIN_SECONDS`, `SHUTDOWN_TIMEOUT_SECONDS`,
`SHUTDOWN_FLUSH_TIMEOUT_SECONDS`, all flattening
`runtime::config::LifecycleConfig` into this binary's `Args`); build the
image with `just docker-build default-job`.
