//! The API process composition roots ([`crate::launch()`],
//! [`crate::launch_with()`]).

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;

use axum::Router;
use runtime::config::LifecycleConfig;
use runtime::exit_status::ExitStatus;
use runtime::lifecycle::{RunOutcome, finish};
use runtime::shutdown::Shutdown;
use telemetry::Telemetry;
use tokio::net::TcpListener;

use crate::api::{Api, ApiContext, ApiParts, run};
use crate::config::ApiConfig;
use crate::server::{HTTP_SERVER_TASK, PROBE_SERVER_TASK};

/// How the runner names the process in its logs and its server task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Label {
    /// Logs "api started" / "api stopped" without a `worker` field; the
    /// server task is `http_server`.
    Api,
    /// Logs "worker started" / "worker stopped" with a `worker` field naming
    /// `name`; the server task is `probe_server`.
    Worker(&'static str),
}

impl Label {
    /// The noun in the started / stopped log messages.
    pub(crate) const fn noun(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Worker(_) => "worker",
        }
    }

    /// The `worker` log field, omitted for [`Label::Api`].
    pub(crate) const fn worker(self) -> Option<&'static str> {
        match self {
            Self::Api => None,
            Self::Worker(name) => Some(name),
        }
    }

    /// The supervisor task name for the server.
    pub(crate) const fn server_task(self) -> &'static str {
        match self {
            Self::Api => HTTP_SERVER_TASK,
            Self::Worker(_) => PROBE_SERVER_TASK,
        }
    }

    /// The noun in the bind-failure log message.
    pub(crate) const fn listener(self) -> &'static str {
        match self {
            Self::Api => "listener",
            Self::Worker(_) => "probe listener",
        }
    }
}

/// Initializes telemetry, creates the process shutdown, runs [`Api::start`]
/// with an [`crate::ApiContext`] carrying the meter provider and that
/// shutdown, composes the router with [`ApiParts::router`], binds the
/// listener from `config`, installs shutdown signal handlers (a second
/// signal exits 143 immediately), and supervises the HTTP server and
/// [`ApiParts::task`] tasks via [`launch_with`] with the shutdown deadline
/// from `config.lifecycle`, ending with a bounded telemetry flush. Returns
/// the process [`ExitCode`].
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

    let shutdown = Shutdown::new();
    let context = ApiContext::new(
        shutdown.clone(),
        Arc::new(telemetry.meter_provider().clone()),
    );
    let parts = match api.start(&context).await {
        Ok(parts) => parts,
        Err(error) => {
            tracing::error!(%error, "api startup failed");
            return finish_failure(telemetry, lifecycle, error.exit_status()).await;
        }
    };

    let router = parts.router(config, &shutdown, telemetry.meter_provider());

    launch_with(
        telemetry,
        shutdown,
        lifecycle,
        config.listen_address,
        router,
        parts,
        Label::Api,
    )
    .await
}

/// Binds `listen_address`, installs the shutdown signal handlers immediately
/// before supervision (a second signal exits 143 immediately), serves
/// `router`, and supervises the server task and `parts`' tasks (see
/// [`ApiParts::task`]) on one [`runtime::supervisor::Supervisor`] under
/// `lifecycle`'s deadline, runs `parts`' shutdown hooks unless the deadline
/// was exceeded, and ends with a bounded telemetry flush. Returns the
/// process [`ExitCode`].
///
/// `router` is served as-is: compose it with [`ApiParts::router`] or
/// [`ApiParts::probes`]; only `parts`' tasks and shutdown hooks are read.
/// `shutdown` must be the handle given to the process's
/// [`crate::ApiContext`].
///
/// Every startup failure is logged with `tracing::error!` and ends through
/// [`runtime::lifecycle::finish`], so telemetry is flushed and the process
/// exits retryable.
///
/// # Panics
///
/// Panics if called outside a Tokio runtime.
pub async fn launch_with(
    telemetry: Telemetry,
    shutdown: Shutdown,
    lifecycle: &LifecycleConfig,
    listen_address: SocketAddr,
    router: Router,
    parts: ApiParts,
    label: Label,
) -> ExitCode {
    let listener = match TcpListener::bind(listen_address).await {
        Ok(listener) => listener,
        Err(error) => {
            let listener = label.listener();
            tracing::error!(
                %error,
                address = %listen_address,
                "binding {listener} failed"
            );
            return finish_failure(telemetry, lifecycle, ExitStatus::Retryable).await;
        }
    };

    if let Err(error) = shutdown.listen_for_signals() {
        tracing::error!(%error, "installing signal handlers failed");
        return finish_failure(telemetry, lifecycle, ExitStatus::Retryable).await;
    }

    let noun = label.noun();
    tracing::info!(worker = label.worker(), address = %listen_address, "{noun} started");

    let supervised = run(router, parts, listener, shutdown, lifecycle, label).await;

    finish(supervised.outcome, lifecycle.flush_timeout(), move || {
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
