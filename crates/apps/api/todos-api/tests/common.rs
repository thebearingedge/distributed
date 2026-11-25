//! Shared helpers for the todos-api integration test binaries. Keep this
//! module to helpers every test binary uses — endpoint-specific helpers
//! belong in the test file that uses them, so no dead-code allows are
//! needed here.

use axum::Router;
use sqlx::PgPool;

use todos_api::{api::create_todos_api, app_state::AppState};
use web::{
    guardrails::{GuardrailsConfig, with_guardrails},
    middleware::with_http_tracing,
};

/// Builds the todos API router against the test database, layered like `main.rs`
/// (request guardrails inside request-id tracing).
pub fn todos_api(postgres: PgPool) -> Router {
    with_http_tracing(with_guardrails(
        create_todos_api(AppState::new(postgres)),
        GuardrailsConfig::default(),
    ))
}
