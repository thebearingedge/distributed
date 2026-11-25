//! The job process composition root.

use std::process::ExitCode;

use runtime::exit_status::ExitStatus;
use runtime::shutdown::Shutdown;
use telemetry::Telemetry;

use crate::job::Job;
use crate::runner;

/// Initializes telemetry, installs shutdown signal handlers, runs `job`, and
/// maps the outcome to a process [`ExitCode`].
///
/// `service_name` is the default `service.name`, which `OTEL_SERVICE_NAME`
/// overrides.
///
/// # Panics
///
/// Panics if called outside a multi-thread Tokio runtime or twice per process
/// (see `telemetry::Telemetry::init`).
pub async fn launch<J: Job + ?Sized>(service_name: &'static str, job: &J) -> ExitCode {
    let telemetry = Telemetry::init(service_name);

    let shutdown = Shutdown::new();
    if let Err(error) = shutdown.listen_for_signals() {
        tracing::error!(%error, "installing signal handlers failed");
        flush(&telemetry);
        return ExitStatus::Retryable.into();
    }

    tracing::info!(job = job.name(), "job started");

    let status = runner::run(job, shutdown).await;

    tracing::info!(
        job = job.name(),
        ?status,
        code = status.code(),
        "job finished"
    );

    flush(&telemetry);

    status.into()
}

fn flush(telemetry: &Telemetry) {
    if let Err(error) = telemetry.shutdown() {
        tracing::warn!(%error, "telemetry shutdown failed");
    }
}
