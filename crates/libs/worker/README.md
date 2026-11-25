# worker

Composable runtime for long-running Kubernetes worker Deployments: the
[`Worker`](src/worker.rs) contract, its [`WorkerConfig`](src/config.rs), the
HTTP probe server ([`probes_router`](src/probes.rs)), joint supervision of the
worker and the server ([`supervise`](src/supervisor.rs)), and the
[`launch`](src/launch.rs) process composition root. One worker per binary; the
crate depends on `runtime`, `web`, and `telemetry`, never on a domain.

## Contents

- [`Worker` / `WorkerContext`](src/worker.rs) — the contract: a static `name`
  plus `run(context)` that keeps the heartbeat fresh, returns once shutdown is
  triggered, and classifies failures via `runtime::RunError`.
- [`WorkerConfig`](src/config.rs) — CLI/env configuration for the probe server
  and heartbeat.
- [`probes_router`](src/probes.rs) — `/livez` and `/readyz` backed by the
  heartbeat and the shutdown handle.
- [`supervise`](src/supervisor.rs) — races the worker and the probe-server
  future; whichever stops first triggers the shutdown and drains the other.
- [`launch`](src/launch.rs) — process composition root: initializes telemetry,
  installs shutdown signal handlers, binds the listener, supervises, flushes
  telemetry, and returns the process exit code.
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

CLI flags (`--health-listen-address`, `--health-check-timeout-seconds`,
`--heartbeat-stale-seconds`) override env vars, which override these defaults.

## Exit statuses

| Exit code | Meaning |
|---|---|
| 0 | Success: the worker stopped cleanly after a shutdown signal. |
| 1 | Retryable: the worker or probe server returned before shutdown was requested, failed transiently, or startup (signals, bind) failed; counts toward `backoffLimit`. |
| 2 | Permanent: a `RunError` classified as permanent (bad input, bad configuration). |
| 143 | Interrupted: the run was cut short by a shutdown signal (128 + SIGTERM). |

A telemetry flush failure is logged and never changes the exit status.
Panics in the worker or the server propagate to the caller.

## Composing a worker

```rust
use clap::Parser;
use worker::{InertWorker, WorkerConfig};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let config = WorkerConfig::parse();
    worker::launch("default_worker", &InertWorker, &config).await
}
```
