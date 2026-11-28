# jobs

Composable runtime for run-to-completion Kubernetes Job processes: the
[`Job`](src/job.rs) contract, the [`run_within`](src/runner.rs) function that
maps the job's outcome to an exit status, and the
[`launch`](src/launch.rs) composition root. One job per
binary; the crate has no HTTP surface and does not depend on axum or `web`.
It depends on the `opentelemetry` API crate for the meter provider it hands
to the job.
Jobs should be idempotent, because a retried Job runs again from the start.

## Contents

- [`Job`](src/job.rs) — the contract: a static `name` plus
  `run(context)` that classifies failures as retryable or permanent via
  `runtime::RunError`. The [`JobContext`](src/job.rs) carries the shutdown
  handle (`context.shutdown()`) and the meter provider
  (`context.meter_provider()`) for instrumentation such as `kafka::metrics`.
- [`run_within`](src/runner.rs) — runs the job bounded by the shutdown
  deadline and maps the outcome: `Ok` gives success even when shutdown was
  triggered, an error after shutdown gives interrupted, and any other error
  gives the error's own classification. Once shutdown is triggered, the job
  has the deadline to return; if it does not, it is dropped and the outcome
  is interrupted with `deadline_exceeded` set. Before the trigger, the job
  may run for any length of time.
- [`launch`](src/launch.rs) — process composition root: initializes telemetry,
  installs shutdown signal handlers immediately before running the job (a
  second signal exits 143 immediately), hands the job the telemetry meter
  provider through its `JobContext`,
  runs the job bounded by the deadline from
  [`runtime::config::LifecycleConfig`](../runtime/src/config.rs), ends with
  a telemetry flush bounded by the flush timeout, and returns the process
  exit code.

Every job binary is a lib plus a thin bin: the job value lives in the app's
lib target, and `main.rs` only parses args, constructs the job, and calls
[`launch`](src/launch.rs). The placeholder job app,
[`default-job`](../../apps/job/default-job), implements one in its lib
target.

## Configuration

Jobs take only the shared lifecycle vars from
[`runtime::config::LifecycleConfig`](../runtime/src/config.rs), which the
app's `Args` flattens. See the [runtime README](../runtime/README.md#configuration)
for the vars, defaults, and CLI flags.

## Exit statuses

Exit codes follow the [runtime taxonomy](../runtime/README.md#exit-codes).
Exit 1 counts toward the Job's `backoffLimit`. Exit 2 is meant for a
`podFailurePolicy` `FailJob` rule. Here is a policy that fails the Job on
exit code 2 and ignores disruption-driven restarts:

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

The job receives a `JobContext` whose `shutdown()` handle it is expected to
observe, returning promptly once it is triggered. The runner neither races nor
aborts the job; an error returned after shutdown is reported as interrupted
(exit code 143), while `Ok` stays success. Once shutdown is triggered, the job
has the deadline (`SHUTDOWN_DRAIN_SECONDS + SHUTDOWN_TIMEOUT_SECONDS`) to
return; a job that does not is dropped, and the process exits 143 after the
bounded telemetry flush.

## Composing a job

The job value lives in the app's lib target; `main.rs` is parse → construct →
launch, and [`launch`](src/launch.rs) takes the job **by value**:

```rust
// crates/apps/job/example-job/src/lib.rs
use async_trait::async_trait;

use jobs::{Job, JobContext};
use runtime::run_error::RunError;

/// The example job process: a run-to-completion, idempotent cleanup.
#[derive(Debug)]
pub struct ExampleJob;

#[async_trait]
impl Job for ExampleJob {
    fn name(&self) -> &'static str {
        "example"
    }

    async fn run(&self, context: JobContext) -> Result<(), RunError> {
        // Idempotent work, observing context.shutdown().
        Ok(())
    }
}
```

```rust
// crates/apps/job/example-job/src/main.rs
use std::process::ExitCode;

use clap::Parser;

use config::Args;
use example_job::ExampleJob;

mod config;

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    jobs::launch("example_job", ExampleJob, &args.lifecycle).await
}
```

where `src/config.rs` is the bin-private `Args` that flattens
[`runtime::config::LifecycleConfig`](../runtime/src/config.rs). The
placeholder job app, [`default-job`](../../apps/job/default-job), shows a
complete implementation.
