//! default-api application binary.

mod config;

use anyhow::Context;
use axum::{Router, http::StatusCode, routing::get, serve};
use clap::Parser;
use tokio::net::TcpListener;

use config::Args;
use runtime::shutdown::Shutdown;
use telemetry::Telemetry;
use web::{
    graceful_shutdown::graceful_shutdown, guardrails::with_guardrails, health,
    metrics::with_http_metrics,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

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
    .merge(health::HealthRouter::new(args.api.readiness_timeout(), shutdown.clone()).into_router());

    shutdown
        .listen_for_signals()
        .context("installing signal handlers")?;

    let shutdown_server = serve(listener, router)
        .with_graceful_shutdown(graceful_shutdown(shutdown, args.api.drain_delay()))
        .await;

    tracing::info!("server stopped. cleaning up...");

    let shutdown_telemetry = telemetry.shutdown();

    shutdown_server?;
    shutdown_telemetry?;

    Ok(())
}
