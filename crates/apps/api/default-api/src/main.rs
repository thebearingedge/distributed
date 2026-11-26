//! default-api application binary.

mod config;

use std::process::ExitCode;

use anyhow::Context;
use axum::{Router, http::StatusCode, routing::get};
use clap::Parser;
use tokio::net::TcpListener;

use config::Args;
use runtime::lifecycle::finish;
use runtime::shutdown::Shutdown;
use runtime::supervisor::Supervisor;
use telemetry::Telemetry;
use web::{
    guardrails::with_guardrails,
    health,
    metrics::with_http_metrics,
    server::{self, HTTP_SERVER_TASK},
};

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let args = Args::parse();
    let lifecycle = &args.api.lifecycle;

    let telemetry = Telemetry::init("default_api");

    let shutdown = Shutdown::new();

    let address = args.api.listen_address;

    let listener = TcpListener::bind(&address)
        .await
        .context("binding listener")?;

    tracing::info!("listening at {address}");

    let router = with_http_metrics(
        with_guardrails(
            Router::new().route("/", get(|| async { StatusCode::OK })),
            args.api.guardrails(),
        ),
        telemetry.meter_provider(),
    )
    .merge(
        health::HealthRouter::new(args.api.health_check_timeout(), shutdown.clone()).into_router(),
    );

    shutdown
        .listen_for_signals()
        .context("installing signal handlers")?;

    let mut supervisor = Supervisor::new(shutdown.clone());
    supervisor.spawn(
        HTTP_SERVER_TASK,
        server::serve(listener, router, shutdown, lifecycle.drain_delay()),
    );
    let supervised = supervisor.run(lifecycle.deadline()).await;

    tracing::info!(
        status = ?supervised.outcome.status,
        code = supervised.outcome.status.code(),
        culprit = ?supervised.culprit,
        deadline_exceeded = supervised.outcome.deadline_exceeded,
        "server stopped. cleaning up..."
    );

    Ok(
        finish(supervised.outcome, lifecycle.flush_timeout(), move || {
            telemetry.shutdown()
        })
        .await,
    )
}
