//! default-api application library: a placeholder API that serves `GET /`
//! with `200 OK` behind the standard API layer stack and JSON fallbacks.

use async_trait::async_trait;
use axum::{Router, http::StatusCode, routing::get};

use runtime::run_error::RunError;
use web::{Api, ApiContext, ApiParts, fallback::with_json_fallbacks};

/// Builds the placeholder API router: `GET /` → `200 OK`, with JSON 404 and
/// 405 fallbacks for everything else.
pub fn create_default_api() -> Router {
    with_json_fallbacks(Router::new().route("/", get(|| async { StatusCode::OK })))
}

/// A placeholder API process: it serves [`create_default_api`] and has no
/// backends, migrations, readiness checks, or shutdown hooks.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultApi;

#[async_trait]
impl Api for DefaultApi {
    async fn start(self, _context: &ApiContext) -> Result<ApiParts, RunError> {
        Ok(ApiParts::new(create_default_api()))
    }
}
