//! The worker process composition root.

use std::future::IntoFuture;
use std::process::ExitCode;

use tokio::net::TcpListener;

use runtime::exit_status::ExitStatus;
use runtime::heartbeat::Heartbeat;
use runtime::shutdown::Shutdown;
use telemetry::Telemetry;

use crate::config::WorkerConfig;
use crate::probes::probes_router;
use crate::supervisor::supervise;
use crate::worker::{Worker, WorkerContext};

/// Initializes telemetry, installs shutdown signal handlers, binds the probe
/// listener, and supervises the worker and its probe server until both have
/// stopped, returning the process [`ExitCode`].
///
/// `service_name` is the default `service.name`, which `OTEL_SERVICE_NAME`
/// overrides.
///
/// # Panics
///
/// Panics if called outside a multi-thread Tokio runtime or twice per process
/// (see `telemetry::Telemetry::init`).
pub async fn launch<W: Worker + ?Sized>(
    service_name: &'static str,
    worker: &W,
    config: &WorkerConfig,
) -> ExitCode {
    let telemetry = Telemetry::init(service_name);

    let shutdown = Shutdown::new();
    if let Err(error) = shutdown.listen_for_signals() {
        tracing::error!(%error, "installing signal handlers failed");
        flush(&telemetry);
        return ExitStatus::Retryable.into();
    }

    let listener = match TcpListener::bind(config.health_listen_address).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(
                %error,
                address = %config.health_listen_address,
                "binding probe listener failed"
            );
            flush(&telemetry);
            return ExitStatus::Retryable.into();
        }
    };

    let heartbeat = Heartbeat::new(config.heartbeat_stale_after());
    let context = WorkerContext::new(shutdown.clone(), heartbeat.clone());

    let router = probes_router(&heartbeat, config.health_check_timeout(), shutdown.clone());
    let server_shutdown = shutdown.clone();
    let server = axum::serve(listener, router)
        .with_graceful_shutdown(async move { server_shutdown.triggered().await })
        .into_future();

    tracing::info!(
        worker = worker.name(),
        address = %config.health_listen_address,
        "worker started"
    );

    let status = supervise(worker, context, server).await;

    tracing::info!(
        worker = worker.name(),
        ?status,
        code = status.code(),
        "worker stopped"
    );

    flush(&telemetry);

    status.into()
}

fn flush(telemetry: &Telemetry) {
    if let Err(error) = telemetry.shutdown() {
        tracing::warn!(%error, "telemetry shutdown failed");
    }
}
