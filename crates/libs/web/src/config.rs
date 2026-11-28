//! Runtime configuration for axum API binaries.

use std::net::SocketAddr;
use std::time::Duration;

use runtime::config::LifecycleConfig;

use crate::cors::{AllowedOrigins, CorsConfig};
use crate::guardrails::GuardrailsConfig;

/// Default for [`crate::config::ApiConfig::request_timeout_seconds`].
pub(crate) const REQUEST_TIMEOUT_DEFAULT_SECONDS: u64 = 30;
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
    /// requests; entries must not end with a slash, and the wildcard `*` is
    /// not supported; unset or empty means CORS is off.
    #[arg(
        long,
        env = "CORS_ALLOWED_ORIGINS",
        value_parser = AllowedOrigins::parse
    )]
    pub cors_allowed_origins: Option<AllowedOrigins>,
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

    /// The request guardrails configured for this binary.
    #[must_use]
    pub(crate) fn guardrails(&self) -> GuardrailsConfig {
        GuardrailsConfig {
            request_timeout: Duration::from_secs(self.request_timeout_seconds),
            body_limit_bytes: self.request_body_limit_bytes,
        }
    }

    /// The CORS policy configured for this binary; unset origins mean CORS
    /// is off.
    #[must_use]
    pub(crate) fn cors(&self) -> CorsConfig {
        CorsConfig {
            origins: self
                .cors_allowed_origins
                .as_ref()
                .map_or_else(Vec::new, |origins| origins.as_slice().to_vec()),
            ..CorsConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use clap::Parser;

    use super::{ApiConfig, GuardrailsConfig};

    #[test]
    fn defaults_apply_when_no_args_or_env() {
        let config = ApiConfig::try_parse_from(["todos-api"]).unwrap();
        assert_eq!(config.listen_address.to_string(), "0.0.0.0:3000");
        assert_eq!(config.health_check_timeout_seconds, 2);
        assert_eq!(config.lifecycle.shutdown_drain_seconds, 5);
        assert_eq!(config.lifecycle.shutdown_timeout_seconds, 15);
        assert_eq!(config.lifecycle.shutdown_flush_timeout_seconds, 5);
        assert_eq!(config.cors_allowed_origins, None);
        assert_eq!(config.request_timeout_seconds, 30);
        assert_eq!(config.request_body_limit_bytes, 1_048_576);
    }

    #[test]
    fn cors_is_disabled_when_unset() {
        let config = ApiConfig::try_parse_from(["todos-api"]).unwrap();
        assert_eq!(config.cors().origins, [] as [axum::http::HeaderValue; 0]);
    }

    #[test]
    fn cors_origins_are_trimmed_and_split() {
        let config = ApiConfig::try_parse_from([
            "todos-api",
            "--cors-allowed-origins",
            " http://a , http://b ",
        ])
        .unwrap();
        let cors = config.cors();
        let origins: Vec<_> = cors
            .origins
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect();
        assert_eq!(origins, vec!["http://a", "http://b"]);
    }

    #[test]
    fn invalid_origins_fail_to_parse() {
        let error = ApiConfig::try_parse_from([
            "todos-api",
            "--cors-allowed-origins",
            "https://a.example/",
        ])
        .unwrap_err();
        assert!(
            error.to_string().contains("must not end with a slash"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn wildcard_origin_fails_to_parse() {
        let error =
            ApiConfig::try_parse_from(["todos-api", "--cors-allowed-origins", "*"]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("CORS origin `*` is not supported; list explicit origins"),
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
        let cors = args.api.cors();
        let cors_origins: Vec<_> = cors
            .origins
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect();
        assert_eq!(cors_origins, vec!["http://a"]);
        assert_eq!(args.api.request_timeout_seconds, 9);
        assert_eq!(args.api.request_body_limit_bytes, 2048);
    }

    #[test]
    fn accessors_convert_seconds_to_durations() {
        let config =
            ApiConfig::try_parse_from(["todos-api", "--health-check-timeout-seconds", "7"])
                .unwrap();
        assert_eq!(config.health_check_timeout(), Duration::from_secs(7));
    }

    #[test]
    fn lifecycle_flags_flatten_without_id_collisions() {
        let config = ApiConfig::try_parse_from([
            "todos-api",
            "--shutdown-drain-seconds",
            "1",
            "--shutdown-timeout-seconds",
            "20",
            "--shutdown-flush-timeout-seconds",
            "3",
        ])
        .unwrap();
        assert_eq!(config.lifecycle.deadline(), Duration::from_secs(21));
    }

    #[test]
    fn guardrails_read_the_request_flags() {
        let config = ApiConfig::try_parse_from([
            "todos-api",
            "--request-timeout-seconds",
            "9",
            "--request-body-limit-bytes",
            "2048",
        ])
        .unwrap();
        assert_eq!(
            config.guardrails(),
            GuardrailsConfig {
                request_timeout: Duration::from_secs(9),
                body_limit_bytes: 2048,
            }
        );
    }
}
