//! Runtime configuration for axum API binaries.

use std::net::SocketAddr;
use std::time::Duration;

use axum::http::HeaderValue;

use crate::cors::{ParseOriginsError, parse_allowed_origins};

/// Runtime configuration for an axum API binary: listener address, drain
/// delay, readiness timeout, and CORS origins. CLI args fall back to env
/// vars, which fall back to declared defaults.
#[derive(Debug, clap::Parser)]
pub struct ApiConfig {
    /// Address and port to bind the listener to.
    #[arg(long, env = "LISTEN_ADDRESS", default_value = "0.0.0.0:3000")]
    pub listen_address: SocketAddr,
    /// Seconds to keep draining in-flight requests after the shutdown
    /// signal before the server stops.
    #[arg(long, env = "SHUTDOWN_DRAIN_SECONDS", default_value_t = 0)]
    pub shutdown_drain_seconds: u64,
    /// Seconds allowed for readiness probes to complete.
    #[arg(long, env = "READINESS_TIMEOUT_SECONDS", default_value_t = 2)]
    pub readiness_timeout_seconds: u64,
    /// Comma-separated exact origins allowed to make cross-origin
    /// requests; unset or empty means CORS is off.
    #[arg(long, env = "CORS_ALLOWED_ORIGINS")]
    pub cors_allowed_origins: Option<String>,
}

impl ApiConfig {
    /// Time to keep draining in-flight requests after the shutdown signal.
    #[must_use]
    pub fn drain_delay(&self) -> Duration {
        Duration::from_secs(self.shutdown_drain_seconds)
    }

    /// Time allowed for readiness probes to complete.
    #[must_use]
    pub fn readiness_timeout(&self) -> Duration {
        Duration::from_secs(self.readiness_timeout_seconds)
    }

    /// Exact origins allowed to make cross-origin requests; empty means
    /// CORS is off.
    ///
    /// # Errors
    ///
    /// Returns [`ParseOriginsError`] when `cors_allowed_origins` contains
    /// an entry ending with a slash or an entry that is not a valid header
    /// value.
    pub fn allowed_origins(&self) -> Result<Vec<HeaderValue>, ParseOriginsError> {
        self.cors_allowed_origins
            .as_deref()
            .map_or(Ok(Vec::new()), parse_allowed_origins)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::http::HeaderValue;
    use clap::Parser;

    use super::ApiConfig;

    #[test]
    fn defaults_apply_when_no_args_or_env() {
        let config = ApiConfig::try_parse_from(["todos-api"]).unwrap();
        assert_eq!(config.listen_address.to_string(), "0.0.0.0:3000");
        assert_eq!(config.shutdown_drain_seconds, 0);
        assert_eq!(config.readiness_timeout_seconds, 2);
        assert_eq!(config.cors_allowed_origins, None);
    }

    #[test]
    fn allowed_origins_is_empty_when_unset() {
        let config = ApiConfig::try_parse_from(["todos-api"]).unwrap();
        assert_eq!(config.allowed_origins().unwrap(), Vec::<HeaderValue>::new());
    }

    #[test]
    fn allowed_origins_trims_and_splits_the_configured_value() {
        let config = ApiConfig::try_parse_from([
            "todos-api",
            "--cors-allowed-origins",
            " http://a , http://b ",
        ])
        .unwrap();
        let values = config.allowed_origins().unwrap();
        let origins: Vec<_> = values.iter().map(|value| value.to_str().unwrap()).collect();
        assert_eq!(origins, vec!["http://a", "http://b"]);
    }

    #[test]
    fn allowed_origins_delegates_parse_errors() {
        let config =
            ApiConfig::try_parse_from(["todos-api", "--cors-allowed-origins", "http://a/"])
                .unwrap();
        let error = config.allowed_origins().unwrap_err();
        assert!(
            error.to_string().contains("must not end with a slash"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn flattens_into_an_app_level_args_struct() {
        #[derive(Parser)]
        struct Args {
            #[command(flatten)]
            api: ApiConfig,
        }
        let args = Args::try_parse_from([
            "todos-api",
            "--listen-address",
            "127.0.0.1:8080",
            "--shutdown-drain-seconds",
            "3",
            "--readiness-timeout-seconds",
            "4",
            "--cors-allowed-origins",
            "http://a",
        ])
        .unwrap();
        assert_eq!(args.api.listen_address.to_string(), "127.0.0.1:8080");
        assert_eq!(args.api.shutdown_drain_seconds, 3);
        assert_eq!(args.api.readiness_timeout_seconds, 4);
        assert_eq!(args.api.cors_allowed_origins.as_deref(), Some("http://a"));
    }

    #[test]
    fn accessors_convert_seconds_to_durations() {
        let config = ApiConfig::try_parse_from([
            "todos-api",
            "--shutdown-drain-seconds",
            "5",
            "--readiness-timeout-seconds",
            "7",
        ])
        .unwrap();
        assert_eq!(config.drain_delay(), Duration::from_secs(5));
        assert_eq!(config.readiness_timeout(), Duration::from_secs(7));
    }
}
