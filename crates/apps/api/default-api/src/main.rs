//! default-api application binary.

use std::{net::SocketAddr, time::Duration};

use anyhow::Context;
use axum::{Router, http::StatusCode, routing::get, serve};
use tokio::net::TcpListener;

use telemetry::provider::init_tracer_provider;
use web::{
    graceful_shutdown::{Draining, graceful_shutdown},
    health,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let tracer_provider = init_tracer_provider("default_api");

    let drain_delay = std::env::var("SHUTDOWN_DRAIN_SECONDS")
        .ok()
        .map_or(Duration::ZERO, |raw| {
            if let Ok(secs) = raw.parse::<u64>() {
                Duration::from_secs(secs)
            } else {
                tracing::warn!(raw, "unparsable SHUTDOWN_DRAIN_SECONDS, defaulting to 0");
                Duration::ZERO
            }
        });

    let draining = Draining::default();

    let address: SocketAddr = std::env::var("LISTEN_ADDRESS")
        .unwrap_or_else(|_| "0.0.0.0:3000".to_string())
        .parse()
        .context("getting socket address")?;

    let listener = TcpListener::bind(&address)
        .await
        .context("binding listener")?;

    tracing::info!("listening at {address}");

    let router = Router::new()
        .route("/", get(|| async { StatusCode::OK }))
        .merge(health::router(
            vec![],
            Duration::from_secs(2),
            draining.clone(),
        ));

    let shutdown_server = serve(listener, router)
        .with_graceful_shutdown(graceful_shutdown(draining, drain_delay))
        .await;

    tracing::info!("server stopped. cleaning up...");

    let shutdown_telemetry = tracer_provider.shutdown();

    shutdown_server?;
    shutdown_telemetry?;

    Ok(())
}
