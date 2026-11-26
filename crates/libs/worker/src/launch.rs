//! The worker process composition root.

use std::process::ExitCode;
use std::sync::Arc;

use tokio::net::TcpListener;

use runtime::exit_status::ExitStatus;
use runtime::heartbeat::Heartbeat;
use runtime::lifecycle::{RunOutcome, finish};
use runtime::shutdown::Shutdown;
use telemetry::Telemetry;
use web::server::serve;

use crate::config::WorkerConfig;
use crate::probes::probes_router;
use crate::supervisor::supervise;
use crate::worker::{Worker, WorkerContext};

/// Initializes telemetry, installs shutdown signal handlers (a second signal
/// exits 143 immediately), binds the probe listener, and supervises the worker
/// and its probe server on a [`runtime::supervisor::Supervisor`] with the
/// shutdown deadline from `config.lifecycle`, ending with a bounded telemetry
/// flush. Returns the process [`ExitCode`].
///
/// `service_name` is the default `service.name`, which `OTEL_SERVICE_NAME`
/// overrides.
///
/// # Panics
///
/// Panics if called outside a multi-thread Tokio runtime or twice per process
/// (see `telemetry::Telemetry::init`).
pub async fn launch<W: Worker + 'static>(
    service_name: &'static str,
    worker: W,
    config: &WorkerConfig,
) -> ExitCode {
    let telemetry = Telemetry::init(service_name);
    let lifecycle = &config.lifecycle;

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

    let listener = match TcpListener::bind(config.health_listen_address).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(
                %error,
                address = %config.health_listen_address,
                "binding probe listener failed"
            );
            return finish(
                RunOutcome::completed(ExitStatus::Retryable),
                lifecycle.flush_timeout(),
                move || telemetry.shutdown(),
            )
            .await;
        }
    };

    let heartbeat = Heartbeat::new(config.heartbeat_stale_after());
    let context = WorkerContext::new(shutdown.clone(), heartbeat.clone());

    let router = probes_router(&heartbeat, config.health_check_timeout(), shutdown.clone());
    let server = serve(listener, router, shutdown.clone(), lifecycle.drain_delay());

    let name = worker.name();
    tracing::info!(
        worker = name,
        address = %config.health_listen_address,
        "worker started"
    );

    let supervised = supervise(Arc::new(worker), context, server, lifecycle.deadline()).await;

    tracing::info!(
        worker = name,
        status = ?supervised.outcome.status,
        code = supervised.outcome.status.code(),
        culprit = ?supervised.culprit,
        deadline_exceeded = supervised.outcome.deadline_exceeded,
        "worker stopped"
    );

    finish(supervised.outcome, lifecycle.flush_timeout(), move || {
        telemetry.shutdown()
    })
    .await
}
