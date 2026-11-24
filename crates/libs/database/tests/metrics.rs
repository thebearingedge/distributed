//! Integration tests for `database::metrics::observe_pool`.

use opentelemetry::Value;
use sqlx::{PgPool, postgres::PgPoolOptions};

use database::metrics::observe_pool;
use telemetry::test_support::{MetricsHarness, SumPoint};

fn lazy_pool(postgres: &PgPool, max_connections: u32) -> PgPool {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect_lazy_with((*postgres.connect_options()).clone())
}

fn state_value(points: &[SumPoint], state: &'static str) -> i64 {
    points
        .iter()
        .find(|point| point.attribute("db.client.connection.state") == Some(&Value::from(state)))
        .map_or_else(
            || {
                panic!(
                    "no point with state {state}; states present: {:?}",
                    points
                        .iter()
                        .map(|point| point.attribute("db.client.connection.state"))
                        .collect::<Vec<_>>()
                )
            },
            |point| point.value,
        )
}

#[sqlx::test]
async fn reports_used_and_idle_connections(postgres: PgPool) {
    let pool = lazy_pool(&postgres, 3);
    let harness = MetricsHarness::default();
    observe_pool(&pool, "test_pool", harness.provider());

    let conn = pool.acquire().await.unwrap();

    let points = harness.i64_sum("db.client.connection.count");
    assert_eq!(state_value(&points, "used"), 1);
    assert_eq!(state_value(&points, "idle"), 0);
    let connect_options = postgres.connect_options();
    let database = connect_options
        .get_database()
        .unwrap_or_default()
        .to_owned();
    let expected = Value::from(database.clone());
    for point in &points {
        assert_eq!(
            point.attribute("db.client.connection.pool.name"),
            Some(&Value::from("test_pool"))
        );
        assert_eq!(
            point.attribute("db.system.name"),
            Some(&Value::from("postgresql"))
        );
        assert_eq!(point.attribute("db.namespace"), Some(&expected));
    }

    drop(conn);
    pool.close().await;
}

#[sqlx::test]
async fn reports_the_connection_limit(postgres: PgPool) {
    let pool = lazy_pool(&postgres, 3);
    let harness = MetricsHarness::default();
    observe_pool(&pool, "test_pool", harness.provider());

    let points = harness.i64_sum("db.client.connection.limit");
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].value, 3);
}

#[sqlx::test]
async fn reports_no_connections_after_the_pool_closes(postgres: PgPool) {
    let pool = lazy_pool(&postgres, 3);
    let harness = MetricsHarness::default();
    observe_pool(&pool, "test_pool", harness.provider());

    let conn = pool.acquire().await.unwrap();
    drop(conn);
    pool.close().await;

    let points = harness.i64_sum("db.client.connection.count");
    assert!(!points.is_empty());
    for point in &points {
        assert_eq!(point.value, 0);
    }
}
