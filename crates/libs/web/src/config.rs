//! Runtime configuration for axum API binaries.

use std::net::SocketAddr;
use std::time::Duration;

use axum::http::HeaderValue;

use runtime::config::LifecycleConfig;

use crate::cors::{ParseOriginsError, parse_allowed_origins};
use crate::guardrails::GuardrailsConfig;

/// Default for [`crate::config::ApiConfig::request_timeout_seconds`].
pub const REQUEST_TIMEOUT_DEFAULT_SECONDS: u64 = 30;
/// Default for [`crate::config::ApiConfig::request_body_limit_bytes`] (1 MiB).
pub const REQUEST_BODY_LIMIT_DEFAULT_BYTES: usize = 1_048_576;

/// Runtime configuration for an axum API binary: listener address, process
/// lifecycle configuration, health-check timeout, CORS origins, request
/// timeout, and request body limit. CLI args fall back to env vars, which
/// fall back to declared defaults.
#[derive(Debug, clap::Parser)]
pub struct ApiConfig {
    /// Address and port to bind the listener to.
    #[arg(long, env = "LISTEN_ADDRESS", default_value = "0.0.0.0:3000")]
    pub listen_address: SocketAddr,
    /// Process lifecycle configuration: drain delay, shutdown deadline,
    /// and final flush budget.
    #[command(flatten)]
    pub lifecycle: LifecycleConfig,
    /// Seconds each liveness or readiness check may take before it counts
    /// as failed.
    #[arg(long, env = "HEALTH_CHECK_TIMEOUT_SECONDS", default_value_t = 2)]
    pub health_check_timeout_seconds: u64,
    /// Comma-separated exact origins allowed to make cross-origin
    /// requests; unset or empty means CORS is off.
    #[arg(long, env = "CORS_ALLOWED_ORIGINS")]
    pub cors_allowed_origins: Option<String>,
    /// Seconds a request may take to produce a response before it is
    /// answered with `503 Service Unavailable`.
    #[arg(long, env = "REQUEST_TIMEOUT_SECONDS", default_value_t = REQUEST_TIMEOUT_DEFAULT_SECONDS)]
    pub request_timeout_seconds: u64,
    /// Maximum request body size in bytes; larger bodies are rejected with
    /// `413 Payload Too Large`.
    #[arg(long, env = "REQUEST_BODY_LIMIT_BYTES", default_value_t = REQUEST_BODY_LIMIT_DEFAULT_BYTES)]
    pub request_body_limit_bytes: usize,
}

impl ApiConfig {
    /// Time each liveness or readiness check may take before it counts as
    /// failed.
    #[must_use]
    pub fn health_check_timeout(&self) -> Duration {
        Duration::from_secs(self.health_check_timeout_seconds)
    }

    /// Time a request may take to produce a response before it is answered
    /// with `503 Service Unavailable`.
    #[must_use]
    pub fn request_timeout(&self) -> Duration {
        Duration::from_secs(self.request_timeout_seconds)
    }

    /// Maximum request body size in bytes; larger bodies are rejected with
    /// `413 Payload Too Large`.
    #[must_use]
    pub fn request_body_limit(&self) -> usize {
        self.request_body_limit_bytes
    }

    /// The request guardrails configured for this binary.
    #[must_use]
    pub fn guardrails(&self) -> GuardrailsConfig {
        GuardrailsConfig {
            request_timeout: self.request_timeout(),
            body_limit_bytes: self.request_body_limit(),
        }
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

    use super::{ApiConfig, GuardrailsConfig};

    #[test]
    fn defaults_apply_when_no_args_or_env() {
        let config = ApiConfig::try_parse_from(["todos-api"]).unwrap();
        assert_eq!(config.listen_address.to_string(), "0.0.0.0:3000");
        assert_eq!(config.health_check_timeout_seconds, 2);
        assert_eq!(config.lifecycle.shutdown_drain_seconds, 0);
        assert_eq!(config.lifecycle.shutdown_timeout_seconds, 20);
        assert_eq!(config.lifecycle.shutdown_flush_timeout_seconds, 5);
        assert_eq!(config.cors_allowed_origins, None);
        assert_eq!(config.request_timeout_seconds, 30);
        assert_eq!(config.request_body_limit_bytes, 1_048_576);
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
            "--shutdown-timeout-seconds",
            "12",
            "--health-check-timeout-seconds",
            "4",
            "--cors-allowed-origins",
            "http://a",
            "--request-timeout-seconds",
            "9",
            "--request-body-limit-bytes",
            "2048",
        ])
        .unwrap();
        assert_eq!(args.api.listen_address.to_string(), "127.0.0.1:8080");
        assert_eq!(args.api.lifecycle.shutdown_drain_seconds, 3);
        assert_eq!(args.api.lifecycle.shutdown_timeout_seconds, 12);
        assert_eq!(args.api.health_check_timeout_seconds, 4);
        assert_eq!(args.api.cors_allowed_origins.as_deref(), Some("http://a"));
        assert_eq!(args.api.request_timeout_seconds, 9);
        assert_eq!(args.api.request_body_limit_bytes, 2048);
    }

    #[test]
    fn accessors_convert_seconds_to_durations() {
        let config = ApiConfig::try_parse_from([
            "todos-api",
            "--health-check-timeout-seconds",
            "7",
            "--request-timeout-seconds",
            "11",
        ])
        .unwrap();
        assert_eq!(config.health_check_timeout(), Duration::from_secs(7));
        assert_eq!(config.request_timeout(), Duration::from_secs(11));
    }

    #[test]
    fn lifecycle_flags_flatten_without_id_collisions() {
        let config = ApiConfig::try_parse_from([
            "todos-api",
            "--shutdown-drain-seconds",
            "1",
            "--shutdown-flush-timeout-seconds",
            "3",
        ])
        .unwrap();
        assert_eq!(config.lifecycle.deadline(), Duration::from_secs(21));
    }

    #[test]
    fn request_body_limit_returns_the_configured_bytes() {
        let config =
            ApiConfig::try_parse_from(["todos-api", "--request-body-limit-bytes", "4096"]).unwrap();
        assert_eq!(config.request_body_limit(), 4096);
    }

    #[test]
    fn guardrails_defaults_match_the_guardrails_config_default() {
        let config = ApiConfig::try_parse_from(["todos-api"]).unwrap();
        assert_eq!(config.guardrails(), GuardrailsConfig::default());
    }
}
