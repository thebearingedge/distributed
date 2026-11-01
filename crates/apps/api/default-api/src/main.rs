//! default-api application binary.

mod config;

use anyhow::Context;
use axum::{Router, http::StatusCode, routing::get, serve};
use clap::Parser;
use tokio::net::TcpListener;

use config::Args;
use telemetry::provider::init_tracer_provider;
use web::{
    graceful_shutdown::{Draining, graceful_shutdown},
    health,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let tracer_provider = init_tracer_provider("default_api");

    let draining = Draining::default();

    let address = args.api.listen_address;

    let listener = TcpListener::bind(&address)
        .await
        .context("binding listener")?;

    tracing::info!("listening at {address}");

    let router = Router::new()
        .route("/", get(|| async { StatusCode::OK }))
        .merge(health::router(
            vec![],
            args.api.readiness_timeout(),
            draining.clone(),
        ));

    let shutdown_server = serve(listener, router)
        .with_graceful_shutdown(graceful_shutdown(draining, args.api.drain_delay()))
        .await;

    tracing::info!("server stopped. cleaning up...");

    let shutdown_telemetry = tracer_provider.shutdown();

    shutdown_server?;
    shutdown_telemetry?;

    Ok(())
}
