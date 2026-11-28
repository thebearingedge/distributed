# worker

Composable runtime for long-running Kubernetes worker Deployments: the
[`Worker`](src/worker.rs) contract, its [`WorkerConfig`](src/config.rs), and
the [`launch`](src/launch.rs) process composition root, which supervises the
worker and its probe server through `web::launch_with`. One worker per
binary; the crate depends on `runtime`, `web`, `telemetry`, and the
`opentelemetry` API crate, never on a domain.

## Contents

- [`Worker` / `WorkerContext`](src/worker.rs) — the contract: a static `name`
  plus `run(context)` that keeps the heartbeat fresh, returns once shutdown is
  triggered, and classifies failures via `runtime::RunError`. The context also
  carries the meter provider (`context.meter_provider()`) for instrumentation
  such as `kafka::metrics`.
- [`WorkerConfig`](src/config.rs) — CLI/env configuration for the probe
  server, heartbeat, and process lifecycle (it flattens
  `runtime::config::LifecycleConfig`).
- [`launch`](src/launch.rs) — process composition root: initializes
  telemetry, creates the process shutdown and heartbeat, hands the worker
  the telemetry meter provider through the context, binds the probe
  listener, installs shutdown signal handlers immediately before
  supervision (a second signal exits 143 immediately), and supervises the
  worker task (named after `Worker::name`) and the probe server through
  [`web::launch_with`](../web/src/launch.rs) with the shutdown deadline;
  the first task to stop (error, panic, or a clean return before shutdown)
  triggers the shutdown and becomes the culprit, and tasks still running at
  the deadline are aborted. Ends with a telemetry flush bounded by
  `SHUTDOWN_FLUSH_TIMEOUT_SECONDS` and returns the process exit code.

Every worker binary is a lib plus a thin bin: the worker value lives in the
app's lib target, and `main.rs` only parses args, constructs the worker, and
calls [`launch`](src/launch.rs). The placeholder worker app,
[`default-worker`](../../apps/worker/default-worker), implements one in its
lib target.

## Probes

- `GET /livez` — `200` while the heartbeat is fresh; `503` with
  `{"status":"unavailable","checks":{"heartbeat":"failed"}}` once no beat has
  landed within `HEARTBEAT_STALE_SECONDS`. Unaffected by shutdown.
- `GET /readyz` — `200` until shutdown is triggered; `503` with
  `{"status":"shutting_down"}` after SIGTERM. There are no readiness checks.

## Configuration

| Env var | Default | Meaning |
|---|---|---|
| `HEALTH_LISTEN_ADDRESS` | `0.0.0.0:3000` | Address and port the probe server binds to. |
| `HEALTH_CHECK_TIMEOUT_SECONDS` | `2` | Seconds each liveness or readiness check may take before it counts as failed. |
| `HEARTBEAT_STALE_SECONDS` | `30` | Seconds without a heartbeat after which liveness fails; must be at least 1. |

CLI flags (`--health-listen-address`, `--health-check-timeout-seconds`,
`--heartbeat-stale-seconds`) override env vars, which override these
defaults.

The shared `SHUTDOWN_*` vars and the shutdown budget are in the
[runtime README](../runtime/README.md#configuration). During the drain the
probe server keeps serving with `/readyz` answering `503 shutting_down`, so
the drain delays worker exit by `SHUTDOWN_DRAIN_SECONDS`.

## Exit statuses

Exit codes follow the [runtime taxonomy](../runtime/README.md#exit-codes).
For a worker, exit 1 also covers the worker or probe server returning before
shutdown was requested, a panic in either, or a startup (bind, signal)
failure; a Deployment restarts the pod.

## Composing a worker

The worker value lives in the app's lib target; `main.rs` is parse →
construct → launch:

```rust
// crates/apps/worker/example-worker/src/lib.rs
use async_trait::async_trait;

use runtime::run_error::RunError;
use worker::{Worker, WorkerContext};

/// The example worker process: consumes work until shutdown is triggered.
#[derive(Debug)]
pub struct ExampleWorker;

#[async_trait]
impl Worker for ExampleWorker {
    fn name(&self) -> &'static str {
        "example"
    }

    async fn run(&self, context: WorkerContext) -> Result<(), RunError> {
        let mut ticker = tokio::time::interval(context.heartbeat().interval());
        loop {
            tokio::select! {
                () = context.shutdown().triggered() => return Ok(()),
                _ = ticker.tick() => context.heartbeat().beat(),
            }
        }
    }
}
```

```rust
// crates/apps/worker/example-worker/src/main.rs
use std::process::ExitCode;

use clap::Parser;

use config::Args;
use example_worker::ExampleWorker;

mod config;

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    worker::launch("example_worker", ExampleWorker, &args.worker).await
}
```

where `src/config.rs` is the bin-private `Args` that flattens
[`WorkerConfig`](src/config.rs). The placeholder worker app,
[`default-worker`](../../apps/worker/default-worker), shows a complete
implementation.
