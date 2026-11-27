use sqlx::PgPool;

use runtime::health::HealthCheck;

use crate::{DbContext, DbError};

/// The name the readiness [`check`] reports as in readiness probe responses.
pub const POSTGRES_HEALTH_CHECK: &str = "postgres";

/// Pings the database with a `select 1` query.
///
/// # Errors
///
/// Returns a [`DbError`] if database is unreachable.
pub(crate) async fn ping(pool: PgPool) -> Result<(), DbError> {
    sqlx::query("select 1")
        .execute(&pool)
        .await
        .map(|_| ())
        .db_context("pinging database")
}

/// A readiness check that pings the given connection pool (`ping`).
#[must_use]
pub fn check(pool: &PgPool) -> HealthCheck {
    HealthCheck::with_resource(POSTGRES_HEALTH_CHECK, pool.clone(), ping)
}
