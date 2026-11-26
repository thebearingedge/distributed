# jobs

Composable runtime for run-to-completion Kubernetes Job processes: the
[`Job`](src/job.rs) contract, the [`run`](src/runner.rs) function that maps the
job's outcome to an exit status, and the [`launch`](src/launch.rs) composition
root. One job per binary; the crate has no HTTP surface and does not depend on
axum or `web`. Jobs should be idempotent, because a retried Job runs again from
the start.

## Contents

- [`Job`](src/job.rs) — the contract: a static `name` plus
  `run(shutdown)` that classifies failures as retryable or permanent via
  `runtime::RunError`.
- [`run`](src/runner.rs) — runs the job and maps the outcome: `Ok` gives
  success even when shutdown was triggered, an error after shutdown gives
  interrupted, and any other error gives the error's own classification.
- [`launch`](src/launch.rs) — process composition root: initializes telemetry,
  installs shutdown signal handlers, runs the job, flushes telemetry, and
  returns the process exit code.
- [`NoopJob`](src/noop.rs) — a placeholder job that does nothing and succeeds.

## Exit statuses

| Exit code | Meaning |
|---|---|
| 0 | Success: the run completed. |
| 1 | Retryable: a transient failure; counts toward the Job's `backoffLimit`. |
| 2 | Permanent: a failure retrying cannot fix (bad input, bad configuration). |
| 143 | Interrupted: the run was cut short by a shutdown signal (128 + SIGTERM). |

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
code 143), while `Ok` stays success. There is no shutdown deadline yet.

## Composing a job

```rust
use jobs::NoopJob;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    jobs::launch("default_job", &NoopJob).await
}
```
