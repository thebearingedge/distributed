//! Shared helpers for the todos-api integration test binaries. Keep this
//! module to helpers every test binary uses — endpoint-specific helpers
//! belong in the test file that uses them, so no dead-code allows are
//! needed here.

use axum::Router;
use opentelemetry::metrics::NoopMeterProvider;
use sqlx::PgPool;

use todos_api::{api::create_todos_api, app_state::AppState};
use web::stack::{StackConfig, with_api_stack};

/// Builds the todos API router against the test database with the same
/// standard API layer stack as production, measured by a no-op meter
/// provider.
pub fn todos_api(postgres: PgPool) -> Router {
    with_api_stack(
        create_todos_api(AppState::new(postgres)),
        StackConfig::default(),
        &NoopMeterProvider::new(),
    )
}
