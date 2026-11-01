//! Integration tests for the [`database::DbConfig`] connection settings.

use sqlx::{ConnectOptions, PgPool};

use database::DbConfig;

#[sqlx::test]
async fn connect_opens_a_working_pool(postgres: PgPool) {
    let config = DbConfig {
        database_url: postgres.connect_options().to_url_lossy().to_string(),
        min_connections: 2,
        max_connections: 10,
        acquire_timeout_seconds: 3,
    };
    let pool = config.connect().await.unwrap();
    sqlx::query("select 1").execute(&pool).await.unwrap();
}

#[tokio::test]
async fn connect_fails_for_unreachable_database() {
    let config = DbConfig {
        database_url: "postgres://postgres:password@127.0.0.1:1/none".into(),
        min_connections: 2,
        max_connections: 10,
        acquire_timeout_seconds: 3,
    };
    let error = config.connect().await.unwrap_err();
    assert!(
        error.to_string().contains("connecting to postgres"),
        "unexpected error: {error}"
    );
}
