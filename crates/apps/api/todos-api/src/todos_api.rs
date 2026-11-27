//! The todos API process ([`TodosApi`]): connects to Postgres, runs the
//! todos migrations, and hands the todos router to [`web::launch`] as
//! [`ApiParts`](web::ApiParts).

use async_trait::async_trait;
use sqlx::PgPool;

use crate::api::create_todos_api;
use crate::app_state::AppState;
use database::DbConfig;
use runtime::run_error::RunError;
use todos::MIGRATOR;
use web::{Api, ApiContext, ApiParts};

/// The todos API process: database configuration plus the startup sequence
/// `web::launch` drives.
#[derive(Debug)]
pub struct TodosApi {
    db: DbConfig,
}

impl TodosApi {
    /// Creates the process from the database configuration.
    #[must_use]
    pub fn new(db: DbConfig) -> Self {
        Self { db }
    }

    /// Builds the API parts from an existing pool: pool metrics, migrations,
    /// the todos router with the postgres readiness check, and the pool
    /// close-down hook.
    ///
    /// # Errors
    ///
    /// Returns a retryable [`RunError`] if running the migrations fails.
    pub async fn parts(pool: PgPool, context: &ApiContext) -> Result<ApiParts, RunError> {
        database::metrics::observe_pool(&pool, "todos", context.meter_provider());

        MIGRATOR.run(&pool).await.map_err(RunError::retryable)?;

        tracing::info!("migrations up to date");

        Ok(ApiParts::new(create_todos_api(AppState::new(pool.clone())))
            .readiness(database::health::check(&pool))
            .on_shutdown(async move { pool.close().await }))
    }
}

#[async_trait]
impl Api for TodosApi {
    async fn start(self, context: &ApiContext) -> Result<ApiParts, RunError> {
        let pool = self.db.connect().await.map_err(RunError::retryable)?;

        tracing::info!("connected to postgres");

        Self::parts(pool, context).await
    }
}
