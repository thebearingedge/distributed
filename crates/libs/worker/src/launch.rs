//! The worker process composition root.

use std::process::ExitCode;
use std::sync::Arc;

use axum::Router;

use runtime::heartbeat::Heartbeat;
use runtime::shutdown::Shutdown;
use telemetry::Telemetry;
use web::ApiParts;
use web::Label;
use web::launch_with;

use crate::config::WorkerConfig;
use crate::probes::heartbeat_check;
use crate::worker::{Worker, WorkerContext};

/// Initializes telemetry, creates the process shutdown and heartbeat, hands
/// the worker the telemetry meter provider through its
/// [`crate::WorkerContext`], binds the probe listener, installs shutdown
/// signal handlers immediately before supervision (a second signal exits 143
/// immediately), and supervises the worker task (named [`Worker::name`]) and
/// the probe server through [`launch_with`] with the shutdown deadline from
/// `config.lifecycle`, ending with a bounded telemetry flush. Returns the
/// process [`ExitCode`]. A signal during startup keeps its default
/// disposition.
///
/// `/livez` is heartbeat-backed and fails once the heartbeat goes stale;
/// `/readyz` flips to `503 shutting_down` on shutdown.
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
    let shutdown = Shutdown::new();
    let heartbeat = Heartbeat::new(config.heartbeat_stale_after());
    let context = WorkerContext::new(
        shutdown.clone(),
        heartbeat.clone(),
        Arc::new(telemetry.meter_provider().clone()),
    );

    let name = worker.name();
    let parts = ApiParts::new(Router::new())
        .liveness(heartbeat_check(&heartbeat))
        .task(name, async move { worker.run(context).await });
    let router = parts.probes(config.health_check_timeout(), &shutdown);

    launch_with(
        telemetry,
        shutdown,
        &config.lifecycle,
        config.health_listen_address,
        router,
        parts,
        Label::Worker(name),
    )
    .await
}
