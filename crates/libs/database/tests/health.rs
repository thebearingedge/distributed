//! Integration tests for the [`database::health`] Postgres readiness probe.

use sqlx::PgPool;

use database::health::{POSTGRES_HEALTH_CHECK, check, ping};

#[sqlx::test]
async fn ping_succeeds_on_a_live_pool(postgres: PgPool) {
    ping(postgres).await.unwrap();
}

#[sqlx::test]
async fn check_is_named_postgres(postgres: PgPool) {
    assert_eq!(check(&postgres).name(), POSTGRES_HEALTH_CHECK);
}

#[sqlx::test]
async fn check_passes_on_a_live_pool(postgres: PgPool) {
    check(&postgres).run().await.unwrap();
}

#[sqlx::test]
async fn check_fails_once_the_pool_is_closed(postgres: PgPool) {
    postgres.close().await;

    let result = check(&postgres).run().await;

    assert!(result.is_err());
}
