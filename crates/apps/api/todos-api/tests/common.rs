//! Shared helpers for the todos-api integration test binaries. Keep this
//! module to helpers every test binary uses — endpoint-specific helpers
//! belong in the test file that uses them, so no dead-code allows are
//! needed here.

use axum::Router;
use clap::Parser;
use opentelemetry::metrics::NoopMeterProvider;
use sqlx::PgPool;

use todos_api::{api::create_todos_api, app_state::AppState};
use web::{config::ApiConfig, stack::with_api_stack};

/// Builds the todos API router against the test database with the same
/// standard API layer stack as production, measured by a no-op meter
/// provider.
///
/// # Panics
///
/// Panics when the test configuration fails to parse.
pub fn todos_api(postgres: PgPool) -> Router {
    let config = ApiConfig::try_parse_from(["todos-api-test", "--cors-allowed-origins="]).unwrap();
    with_api_stack(
        create_todos_api(AppState::new(postgres)),
        &config,
        &NoopMeterProvider::new(),
    )
}
