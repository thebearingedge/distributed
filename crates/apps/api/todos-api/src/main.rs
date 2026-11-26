//! todos-api application binary.

use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;
use tokio::net::TcpListener;

use config::Args;
use runtime::lifecycle::finish;
use runtime::shutdown::Shutdown;
use runtime::supervisor::Supervisor;
use telemetry::Telemetry;
use todos::MIGRATOR;
use todos_api::{api::create_todos_api, app_state::AppState};
use web::{
    cors::{CorsConfig, with_cors},
    guardrails::with_guardrails,
    health::{HealthCheck, HealthRouter},
    metrics::with_http_metrics,
    middleware::with_http_tracing,
    server::{self, HTTP_SERVER_TASK},
};

mod config;

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let args = Args::parse();
    let lifecycle = &args.api.lifecycle;

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
        HealthRouter::new(args.api.health_check_timeout(), shutdown.clone())
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

    if !supervised.outcome.deadline_exceeded {
        pool.close().await;
    }

    Ok(
        finish(supervised.outcome, lifecycle.flush_timeout(), move || {
            telemetry.shutdown()
        })
        .await,
    )
}
