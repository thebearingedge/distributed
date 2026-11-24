//! default-api application binary.

mod config;

use anyhow::Context;
use axum::{Router, http::StatusCode, routing::get, serve};
use clap::Parser;
use tokio::net::TcpListener;

use config::Args;
use telemetry::Telemetry;
use web::{
    graceful_shutdown::{Draining, graceful_shutdown},
    health,
    metrics::with_http_metrics,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let telemetry = Telemetry::init("default_api");

    let draining = Draining::default();

    let address = args.api.listen_address;

    let listener = TcpListener::bind(&address)
        .await
        .context("binding listener")?;

    tracing::info!("listening at {address}");

    let router = with_http_metrics(
        Router::new().route("/", get(|| async { StatusCode::OK })),
        telemetry.meter_provider(),
    )
    .merge(health::router(
        vec![],
        args.api.readiness_timeout(),
        draining.clone(),
    ));

    let shutdown_server = serve(listener, router)
        .with_graceful_shutdown(graceful_shutdown(draining, args.api.drain_delay()))
        .await;

    tracing::info!("server stopped. cleaning up...");

    let shutdown_telemetry = telemetry.shutdown();

    shutdown_server?;
    shutdown_telemetry?;

    Ok(())
}
