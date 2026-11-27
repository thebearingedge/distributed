# worker

Composable runtime for long-running Kubernetes worker Deployments: the
[`Worker`](src/worker.rs) contract, its [`WorkerConfig`](src/config.rs), the
HTTP probe server ([`probes_router`](src/probes.rs)), joint supervision of the
worker and the server ([`supervise`](src/supervisor.rs)), and the
[`launch`](src/launch.rs) process composition root. One worker per binary; the
crate depends on `runtime`, `web`, `telemetry`, and the `opentelemetry` API
crate, never on a domain.

## Contents

- [`Worker` / `WorkerContext`](src/worker.rs) — the contract: a static `name`
  plus `run(context)` that keeps the heartbeat fresh, returns once shutdown is
  triggered, and classifies failures via `runtime::RunError`. The context also
  carries the meter provider (`context.meter_provider()`) for instrumentation
  such as `kafka::metrics`.
- [`WorkerConfig`](src/config.rs) — CLI/env configuration for the probe
  server, heartbeat, and process lifecycle (it flattens
  `runtime::config::LifecycleConfig`).
- [`probes_router`](src/probes.rs) — `/livez` and `/readyz` backed by the
  heartbeat and the shutdown handle.
- [`supervise`](src/supervisor.rs) — runs the worker and the probe-server
  future on a `runtime::supervisor::Supervisor`; the first task to stop
  (error, panic, or a clean return before shutdown) triggers the shutdown and
  becomes the culprit, and tasks still running at the shutdown deadline are
  aborted.
- [`launch`](src/launch.rs) — process composition root: initializes telemetry,
  installs shutdown signal handlers (a second signal exits 143 immediately),
  binds the listener, hands the worker the telemetry meter provider through
  the context, supervises under the shutdown deadline, flushes
  telemetry within `SHUTDOWN_FLUSH_TIMEOUT_SECONDS`, and returns the process
  exit code.
- [`InertWorker`](src/inert.rs) — a placeholder worker that heartbeats and
  stops cleanly on SIGTERM.

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
| `SHUTDOWN_DRAIN_SECONDS` | `0` | Seconds to keep serving after the shutdown signal, with readiness reporting shutting down, before the probe server stops accepting connections. |
| `SHUTDOWN_TIMEOUT_SECONDS` | `20` | Seconds after the drain that the worker and probe server may take to stop before they are aborted; must be at least 1. |
| `SHUTDOWN_FLUSH_TIMEOUT_SECONDS` | `5` | Seconds the final telemetry flush may take; must be at least 1. |

CLI flags (`--health-listen-address`, `--health-check-timeout-seconds`,
`--heartbeat-stale-seconds`, `--shutdown-drain-seconds`,
`--shutdown-timeout-seconds`, `--shutdown-flush-timeout-seconds`) override env
vars, which override these defaults.

The shutdown deadline (`SHUTDOWN_DRAIN_SECONDS + SHUTDOWN_TIMEOUT_SECONDS`,
measured from the moment shutdown is triggered — by a signal or by a failing
task) plus `SHUTDOWN_FLUSH_TIMEOUT_SECONDS` must fit inside the pod's
`terminationGracePeriodSeconds`: the defaults give 25 < 30. Work a task does
during shutdown must fit inside `SHUTDOWN_TIMEOUT_SECONDS`.

## Exit statuses

| Exit code | Meaning |
|---|---|
| 0 | Success: the worker stopped cleanly after a shutdown signal. |
| 1 | Retryable: the worker or probe server returned before shutdown was requested, failed transiently, panicked, or startup (signals, bind) failed; counts toward `backoffLimit`. |
| 2 | Permanent: a `RunError` classified as permanent (bad input, bad configuration). |
| 143 | Interrupted: the run was cut short by a shutdown signal (128 + SIGTERM), the shutdown deadline passed with tasks still running, or a second signal arrived. |

A telemetry flush failure is logged and never changes the exit status.
Panics in the worker or the probe server are caught by the supervisor and
exit 1.

## Composing a worker

```rust
use clap::Parser;
use worker::{InertWorker, WorkerConfig};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let config = WorkerConfig::parse();
    worker::launch("default_worker", InertWorker, &config).await
}
```
