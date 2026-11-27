//! The job process composition root.

use std::process::ExitCode;
use std::sync::Arc;

use runtime::config::LifecycleConfig;
use runtime::exit_status::ExitStatus;
use runtime::lifecycle::{RunOutcome, finish};
use runtime::shutdown::Shutdown;
use telemetry::Telemetry;

use crate::job::{Job, JobContext};
use crate::runner;

/// Initializes telemetry, installs shutdown signal handlers (a second signal
/// exits 143 immediately), runs `job` bounded by the shutdown deadline from
/// `lifecycle`, and ends with a telemetry flush bounded by
/// `lifecycle.flush_timeout()`. Returns the process [`ExitCode`]. It hands
/// the job the telemetry meter provider through its [`crate::JobContext`].
///
/// `service_name` is the default `service.name`, which `OTEL_SERVICE_NAME`
/// overrides.
///
/// # Panics
///
/// Panics if called outside a multi-thread Tokio runtime or twice per process
/// (see `telemetry::Telemetry::init`).
pub async fn launch<J: Job>(
    service_name: &'static str,
    job: J,
    lifecycle: &LifecycleConfig,
) -> ExitCode {
    let telemetry = Telemetry::init(service_name);

    let shutdown = Shutdown::new();
    if let Err(error) = shutdown.listen_for_signals() {
        tracing::error!(%error, "installing signal handlers failed");
        return finish(
            RunOutcome::completed(ExitStatus::Retryable),
            lifecycle.flush_timeout(),
            move || telemetry.shutdown(),
        )
        .await;
    }

    tracing::info!(job = job.name(), "job started");

    let context = JobContext::new(shutdown, Arc::new(telemetry.meter_provider().clone()));
    let outcome = runner::run_within(&job, context, lifecycle.deadline()).await;

    tracing::info!(
        job = job.name(),
        status = ?outcome.status,
        code = outcome.status.code(),
        deadline_exceeded = outcome.deadline_exceeded,
        "job finished"
    );

    finish(outcome, lifecycle.flush_timeout(), move || {
        telemetry.shutdown()
    })
    .await
}
