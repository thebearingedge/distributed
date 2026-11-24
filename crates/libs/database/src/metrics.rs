//! Connection-pool usage metrics for a `sqlx` Postgres pool
//! ([`crate::metrics::observe_pool`]).

use opentelemetry::{InstrumentationScope, Key, KeyValue, metrics::MeterProvider};
use sqlx::PgPool;

const DB_CLIENT_CONNECTION_COUNT: &str = "db.client.connection.count";
const DB_CLIENT_CONNECTION_LIMIT: &str = "db.client.connection.limit";
const DB_CLIENT_CONNECTION_POOL_NAME: Key = Key::from_static_str("db.client.connection.pool.name");
const DB_CLIENT_CONNECTION_STATE: Key = Key::from_static_str("db.client.connection.state");
const DB_SYSTEM_NAME: Key = Key::from_static_str("db.system.name");
const DB_NAMESPACE: Key = Key::from_static_str("db.namespace");
const DB_SYSTEM_NAME_POSTGRESQL: &str = "postgresql";
const DB_CLIENT_CONNECTION_UNIT: &str = "{connection}";

#[derive(Clone, Copy)]
enum ConnectionState {
    Idle,
    Used,
}

impl ConnectionState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Used => "used",
        }
    }
}

fn attributes_with_state(base: &[KeyValue], state: ConnectionState) -> Vec<KeyValue> {
    base.iter()
        .cloned()
        .chain([KeyValue::new(DB_CLIENT_CONNECTION_STATE, state.as_str())])
        .collect()
}

/// Reports the connection-pool metrics of `pool` against `meter_provider`:
/// `db.client.connection.count` with the `db.client.connection.state`
/// attribute values `idle` and `used`, and `db.client.connection.limit`.
///
/// Every point is labelled with `db.client.connection.pool.name` set to
/// `pool_name`, `db.system.name` set to `postgresql`, and — when the pool's
/// connect options name a database — `db.namespace`.
///
/// Values are read at each metric collection.
///
/// Each call registers new callbacks for the lifetime of `meter_provider`,
/// so call it once per pool. The callbacks hold a clone of `pool`.
pub fn observe_pool<P>(pool: &PgPool, pool_name: &'static str, meter_provider: &P)
where
    P: MeterProvider + ?Sized,
{
    let meter = meter_provider.meter_with_scope(
        InstrumentationScope::builder(env!("CARGO_PKG_NAME"))
            .with_version(env!("CARGO_PKG_VERSION"))
            .build(),
    );
    let mut base = vec![
        KeyValue::new(DB_CLIENT_CONNECTION_POOL_NAME, pool_name),
        KeyValue::new(DB_SYSTEM_NAME, DB_SYSTEM_NAME_POSTGRESQL),
    ];
    base.extend(
        pool.connect_options()
            .get_database()
            .map(|database| KeyValue::new(DB_NAMESPACE, database.to_owned())),
    );
    let idle = attributes_with_state(&base, ConnectionState::Idle);
    let used = attributes_with_state(&base, ConnectionState::Used);
    let observed = pool.clone();
    let _ = meter
        .i64_observable_up_down_counter(DB_CLIENT_CONNECTION_COUNT)
        .with_unit(DB_CLIENT_CONNECTION_UNIT)
        .with_description(
            "The number of connections that are currently in the state described by the state attribute.",
        )
        .with_callback(move |observer| {
            let size = i64::from(observed.size());
            let idle_count = i64::try_from(observed.num_idle()).unwrap_or(i64::MAX);
            observer.observe(idle_count, &idle);
            observer.observe(size.saturating_sub(idle_count).max(0), &used);
        })
        .build();
    let limit = i64::from(pool.options().get_max_connections());
    let _ = meter
        .i64_observable_up_down_counter(DB_CLIENT_CONNECTION_LIMIT)
        .with_unit(DB_CLIENT_CONNECTION_UNIT)
        .with_description("The maximum number of open connections allowed.")
        .with_callback(move |observer| observer.observe(limit, &base))
        .build();
}
