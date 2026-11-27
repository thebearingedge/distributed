//! The API process composition root ([`crate::launch()`]).

use std::process::ExitCode;
use std::sync::Arc;

use runtime::config::LifecycleConfig;
use runtime::exit_status::ExitStatus;
use runtime::lifecycle::{RunOutcome, finish};
use runtime::shutdown::Shutdown;
use telemetry::Telemetry;
use tokio::net::TcpListener;

use crate::api::{Api, ApiContext, run};
use crate::config::ApiConfig;

/// Initializes telemetry, runs [`Api::start`], binds the listener from
/// `config`, installs shutdown signal handlers (a second signal exits 143
/// immediately), supervises the HTTP server on a
/// [`runtime::supervisor::Supervisor`] with the shutdown deadline from
/// `config.lifecycle`, and ends with a bounded telemetry flush. Returns the
/// process [`ExitCode`]. It hands the API the telemetry meter provider
/// through its [`crate::ApiContext`].
///
/// The signal handlers are installed only after [`Api::start`] and the bind
/// succeed, so a signal during startup keeps its default disposition.
///
/// Every startup failure is logged with `tracing::error!` and ends through
/// [`runtime::lifecycle::finish`], so telemetry is flushed and the
/// classification of the [`runtime::run_error::RunError`] becomes the exit
/// status.
///
/// `service_name` is the default `service.name`, which `OTEL_SERVICE_NAME`
/// overrides.
///
/// # Panics
///
/// Panics if called outside a multi-thread Tokio runtime or twice per process
/// (see `telemetry::Telemetry::init`).
pub async fn launch<A: Api>(service_name: &'static str, api: A, config: &ApiConfig) -> ExitCode {
    let telemetry = Telemetry::init(service_name);
    let lifecycle = &config.lifecycle;

    let context = ApiContext::new(Arc::new(telemetry.meter_provider().clone()));
    let parts = match api.start(&context).await {
        Ok(parts) => parts,
        Err(error) => {
            tracing::error!(%error, "api startup failed");
            return finish_failure(telemetry, lifecycle, error.exit_status()).await;
        }
    };

    let listener = match TcpListener::bind(config.listen_address).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(
                %error,
                address = %config.listen_address,
                "binding listener failed"
            );
            return finish_failure(telemetry, lifecycle, ExitStatus::Retryable).await;
        }
    };

    let shutdown = Shutdown::new();
    if let Err(error) = shutdown.listen_for_signals() {
        tracing::error!(%error, "installing signal handlers failed");
        return finish_failure(telemetry, lifecycle, ExitStatus::Retryable).await;
    }

    tracing::info!(address = %config.listen_address, "api started");

    let outcome = run(
        parts,
        listener,
        config,
        telemetry.meter_provider(),
        shutdown,
    )
    .await;

    finish(outcome, lifecycle.flush_timeout(), move || {
        telemetry.shutdown()
    })
    .await
}

/// Ends a failed startup with a bounded telemetry flush.
async fn finish_failure(
    telemetry: Telemetry,
    lifecycle: &LifecycleConfig,
    status: ExitStatus,
) -> ExitCode {
    finish(
        RunOutcome::completed(status),
        lifecycle.flush_timeout(),
        move || telemetry.shutdown(),
    )
    .await
}
