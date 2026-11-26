# default-job

A placeholder job image: it initializes telemetry, logs `"job started"` and
`"job finished"`, runs a no-op job, and exits with `0`. It reserves the
default Kubernetes Job / CronJob slot so deployments can swap in real jobs
without touching the wiring. See
[`crates/libs/jobs/README.md`](../../../libs/jobs/README.md) for exit codes and
shutdown semantics; build the image with `just docker-build default-job`.
