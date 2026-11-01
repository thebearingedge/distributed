//! todos-api application binary.

use anyhow::Context;
use axum::serve;
use clap::Parser;
use tokio::net::TcpListener;

use config::Args;
use telemetry::provider::init_tracer_provider;
use todos::MIGRATOR;
use todos_api::{api::create_todos_api, app_state::AppState};
use web::{
    cors::{CorsConfig, with_cors},
    graceful_shutdown::{Draining, graceful_shutdown},
    health::{self, HealthCheck},
    middleware::with_http_tracing,
};

mod config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let tracer_provider = init_tracer_provider("todos_api");

    let draining = Draining::default();

    let pool = args.db.connect().await?;

    tracing::info!("connected to postgres. running migrations...");

    MIGRATOR.run(&pool).await.context("running migrations")?;

    tracing::info!("migrations up to date");

    let allowed_origins = args
        .api
        .allowed_origins()
        .context("invalid CORS_ALLOWED_ORIGINS")?;

    let router = with_cors(
        with_http_tracing(create_todos_api(AppState::new(pool.clone()))),
        CorsConfig {
            allowed_origins,
            ..CorsConfig::default()
        },
    )
    .merge(health::router(
        vec![HealthCheck::with_resource(
            "postgres",
            pool.clone(),
            database::health::ping,
        )],
        args.api.readiness_timeout(),
        draining.clone(),
    ));

    let listener = TcpListener::bind(args.api.listen_address)
        .await
        .context("binding listener")?;

    tracing::info!("listening at {}", args.api.listen_address);

    let shutdown_server = serve(listener, router)
        .with_graceful_shutdown(graceful_shutdown(draining, args.api.drain_delay()))
        .await;

    tracing::info!("server stopped. cleaning up...");

    pool.close().await;

    let shutdown_telemetry = tracer_provider.shutdown();

    shutdown_server?;
    shutdown_telemetry?;

    Ok(())
}
