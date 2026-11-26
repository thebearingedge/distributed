# jobs

Composable runtime for run-to-completion Kubernetes Job processes: the
[`Job`](src/job.rs) contract, the [`run`](src/runner.rs) and
[`run_within`](src/runner.rs) functions that map the job's outcome to an exit
status, and the [`launch`](src/launch.rs) composition root. One job per
binary; the crate has no HTTP surface and does not depend on axum or `web`.
Jobs should be idempotent, because a retried Job runs again from the start.

## Contents

- [`Job`](src/job.rs) — the contract: a static `name` plus
  `run(shutdown)` that classifies failures as retryable or permanent via
  `runtime::RunError`.
- [`run`](src/runner.rs) — runs the job and maps the outcome: `Ok` gives
  success even when shutdown was triggered, an error after shutdown gives
  interrupted, and any other error gives the error's own classification.
- [`run_within`](src/runner.rs) — runs the job like [`run`](src/runner.rs),
  bounded by the shutdown deadline: once shutdown is triggered, the job has
  the deadline to return; if it does not, it is dropped and the outcome is
  interrupted with `deadline_exceeded` set. Before the trigger, the job may
  run for any length of time.
- [`launch`](src/launch.rs) — process composition root: initializes telemetry,
  installs shutdown signal handlers (a second signal exits 143 immediately),
  runs the job bounded by the deadline from
  [`runtime::config::LifecycleConfig`](../../runtime/src/config.rs), ends with
  a telemetry flush bounded by the flush timeout, and returns the process
  exit code.
- [`NoopJob`](src/noop.rs) — a placeholder job that does nothing and succeeds.

## Configuration

| Env var | Default | Meaning |
|---|---|---|
| `SHUTDOWN_DRAIN_SECONDS` | `0` | Seconds to keep serving after the shutdown signal. Jobs have no server to drain but accept the knob. |
| `SHUTDOWN_TIMEOUT_SECONDS` | `20` | Seconds after the drain the job may take to stop before it is abandoned and the process exits 143; must be at least 1. |
| `SHUTDOWN_FLUSH_TIMEOUT_SECONDS` | `5` | Seconds the final telemetry flush may take; must be at least 1. |

These come from `runtime::config::LifecycleConfig`, which the app's `Args`
flattens. CLI flags (`--shutdown-drain-seconds`,
`--shutdown-timeout-seconds`, `--shutdown-flush-timeout-seconds`) override
env vars, which override these defaults.

## Exit statuses

| Exit code | Meaning |
|---|---|
| 0 | Success: the run completed. |
| 1 | Retryable: a transient failure; counts toward the Job's `backoffLimit`. |
| 2 | Permanent: a failure retrying cannot fix (bad input, bad configuration). |
| 143 | Interrupted: the run was cut short by a shutdown signal (128 + SIGTERM), the shutdown deadline passed, or a second signal arrived. |

A telemetry flush failure is logged and never changes the exit status.

A `podFailurePolicy` that fails the Job on exit code 2 and ignores
disruption-driven restarts:

```yaml
podFailurePolicy:
  rules:
    - action: FailJob
      onExitCodes:
        containerName: app
        operator: In
        values: [2]
    - action: Ignore
      onPodConditions:
        - type: DisruptionTarget
```

## Shutdown

The job receives a `runtime::Shutdown` handle and is expected to observe it,
returning promptly once it is triggered. The runner neither races nor aborts
the job; an error returned after shutdown is reported as interrupted (exit
code 143), while `Ok` stays success. Once shutdown is triggered, the job has
the deadline (`SHUTDOWN_DRAIN_SECONDS + SHUTDOWN_TIMEOUT_SECONDS`) to return;
a job that does not is dropped, and the process exits 143 after the bounded
telemetry flush. A second `SIGTERM`/`SIGINT` exits 143 immediately, skipping
the drain, the deadline, and the flush.

## Composing a job

```rust
use clap::Parser;
use jobs::NoopJob;
use runtime::config::LifecycleConfig;

/// Command-line configuration for the job binary.
#[derive(Debug, clap::Parser)]
struct Args {
    /// Process lifecycle configuration.
    #[command(flatten)]
    lifecycle: LifecycleConfig,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = Args::parse();
    jobs::launch("default_job", &NoopJob, &args.lifecycle).await
}
```
