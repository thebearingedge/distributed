//! todos-api application binary.

use anyhow::Context;
use axum::serve;
use clap::Parser;
use tokio::net::TcpListener;

use config::Args;
use runtime::shutdown::Shutdown;
use telemetry::Telemetry;
use todos::MIGRATOR;
use todos_api::{api::create_todos_api, app_state::AppState};
use web::{
    cors::{CorsConfig, with_cors},
    graceful_shutdown::graceful_shutdown,
    guardrails::with_guardrails,
    health::{HealthCheck, HealthRouter},
    metrics::with_http_metrics,
    middleware::with_http_tracing,
};

mod config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let telemetry = Telemetry::init("todos_api");

    let shutdown = Shutdown::new();

    let pool = args.db.connect().await?;

    database::metrics::observe_pool(&pool, "todos", telemetry.meter_provider());

    tracing::info!("connected to postgres. running migrations...");

    MIGRATOR.run(&pool).await.context("running migrations")?;

    tracing::info!("migrations up to date");

    let allowed_origins = args
        .api
        .allowed_origins()
        .context("invalid CORS_ALLOWED_ORIGINS")?;

    let router = with_http_metrics(
        with_cors(
            with_http_tracing(with_guardrails(
                create_todos_api(AppState::new(pool.clone())),
                args.api.guardrails(),
            )),
            CorsConfig {
                allowed_origins,
                ..CorsConfig::default()
            },
        ),
        telemetry.meter_provider(),
    )
    .merge(
        HealthRouter::new(args.api.readiness_timeout(), shutdown.clone())
            .readiness(HealthCheck::with_resource(
                "postgres",
                pool.clone(),
                database::health::ping,
            ))
            .into_router(),
    );

    let listener = TcpListener::bind(args.api.listen_address)
        .await
        .context("binding listener")?;

    tracing::info!("listening at {}", args.api.listen_address);

    shutdown
        .listen_for_signals()
        .context("installing signal handlers")?;

    let shutdown_server = serve(listener, router)
        .with_graceful_shutdown(graceful_shutdown(shutdown, args.api.drain_delay()))
        .await;

    tracing::info!("server stopped. cleaning up...");

    pool.close().await;

    let shutdown_telemetry = telemetry.shutdown();

    shutdown_server?;
    shutdown_telemetry?;

    Ok(())
}
