use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tracing::instrument;

use crate::{DbContext, DbError};

/// Database connection configuration for app binaries: connection string
/// and pool sizing. CLI args fall back to env vars, which fall back to
/// declared defaults.
#[derive(Debug, clap::Parser)]
pub struct DbConfig {
    /// Postgres connection string, e.g.
    /// `postgres://user:password@host/database`.
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: String,
    /// Minimum number of connections kept in the pool.
    #[arg(long, env = "DB_MIN_CONNECTIONS", default_value_t = 2)]
    pub min_connections: u32,
    /// Maximum number of connections in the pool.
    #[arg(long, env = "DB_MAX_CONNECTIONS", default_value_t = 10)]
    pub max_connections: u32,
    /// Seconds to wait for a connection to become available.
    #[arg(long, env = "DB_ACQUIRE_TIMEOUT_SECONDS", default_value_t = 3)]
    pub acquire_timeout_seconds: u64,
}

impl DbConfig {
    /// Opens a connection pool with this configuration.
    ///
    /// # Errors
    ///
    /// Returns a [`DbError`] if connecting fails.
    #[instrument(skip(self))]
    pub async fn connect(&self) -> Result<PgPool, DbError> {
        PgPoolOptions::new()
            .min_connections(self.min_connections)
            .max_connections(self.max_connections)
            .acquire_timeout(Duration::from_secs(self.acquire_timeout_seconds))
            .connect(&self.database_url)
            .await
            .db_context("connecting to postgres")
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use clap::{CommandFactory, Parser};

    use super::DbConfig;

    #[test]
    fn defaults_apply_when_only_the_url_is_given() {
        let config =
            DbConfig::try_parse_from(["app", "--database-url", "postgres://localhost/x"]).unwrap();
        assert_eq!(config.min_connections, 2);
        assert_eq!(config.max_connections, 10);
        assert_eq!(config.acquire_timeout_seconds, 3);
    }

    #[test]
    fn database_url_argument_is_required_without_a_default() {
        let command = DbConfig::command();
        let arg = command
            .get_arguments()
            .find(|arg| arg.get_long() == Some("database-url"))
            .expect("database_url argument is defined");
        assert!(arg.is_required_set());
        assert_eq!(arg.get_default_values(), &[] as &[std::ffi::OsString]);
        assert_eq!(arg.get_env(), Some(OsStr::new("DATABASE_URL")));
    }
}
