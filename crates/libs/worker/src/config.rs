//! Runtime configuration for a worker binary: probe-server address,
//! health-check timeout, heartbeat staleness, and process lifecycle. CLI args
//! fall back to env vars, which fall back to declared defaults.

use std::net::SocketAddr;
use std::time::Duration;

use runtime::config::LifecycleConfig;

/// Runtime configuration for a worker binary: probe-server address,
/// health-check timeout, heartbeat staleness, and process lifecycle. CLI args
/// fall back to env vars, which fall back to declared defaults.
#[derive(Debug, clap::Parser)]
pub struct WorkerConfig {
    /// Address and port the liveness/readiness probe server binds to.
    #[arg(long, env = "HEALTH_LISTEN_ADDRESS", default_value = "0.0.0.0:3000")]
    pub health_listen_address: SocketAddr,
    /// Seconds each liveness or readiness check may take before it counts as
    /// failed.
    #[arg(long, env = "HEALTH_CHECK_TIMEOUT_SECONDS", default_value_t = 2)]
    pub health_check_timeout_seconds: u64,
    /// Seconds without a worker heartbeat after which liveness fails; must be
    /// at least 1.
    #[arg(long, env = "HEARTBEAT_STALE_SECONDS", default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..))]
    pub heartbeat_stale_seconds: u64,
    /// Process lifecycle configuration: drain delay, shutdown deadline,
    /// and final flush budget.
    #[command(flatten)]
    pub lifecycle: LifecycleConfig,
}

impl WorkerConfig {
    /// Time each liveness or readiness check may take before it counts as
    /// failed.
    #[must_use]
    pub fn health_check_timeout(&self) -> Duration {
        Duration::from_secs(self.health_check_timeout_seconds)
    }

    /// Time without a worker heartbeat after which liveness fails.
    #[must_use]
    pub fn heartbeat_stale_after(&self) -> Duration {
        Duration::from_secs(self.heartbeat_stale_seconds)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use clap::Parser;

    use super::WorkerConfig;

    #[test]
    fn defaults() {
        let config = WorkerConfig::try_parse_from(["test"]).unwrap();
        assert_eq!(config.health_listen_address.to_string(), "0.0.0.0:3000");
        assert_eq!(config.health_check_timeout_seconds, 2);
        assert_eq!(config.heartbeat_stale_seconds, 30);
        assert_eq!(config.lifecycle.shutdown_timeout_seconds, 20);
    }

    #[test]
    fn cli_args_override_defaults() {
        let config = WorkerConfig::try_parse_from([
            "test",
            "--health-listen-address",
            "127.0.0.1:8080",
            "--health-check-timeout-seconds",
            "5",
            "--heartbeat-stale-seconds",
            "60",
        ])
        .unwrap();
        assert_eq!(config.health_listen_address.to_string(), "127.0.0.1:8080");
        assert_eq!(config.health_check_timeout_seconds, 5);
        assert_eq!(config.heartbeat_stale_seconds, 60);
    }

    #[test]
    fn accessors_convert_to_durations() {
        let config = WorkerConfig::try_parse_from([
            "test",
            "--health-check-timeout-seconds",
            "7",
            "--heartbeat-stale-seconds",
            "45",
        ])
        .unwrap();
        assert_eq!(config.health_check_timeout(), Duration::from_secs(7));
        assert_eq!(config.heartbeat_stale_after(), Duration::from_secs(45));
    }

    #[test]
    fn lifecycle_flags_flatten_without_id_collisions() {
        let config = WorkerConfig::try_parse_from([
            "test",
            "--shutdown-drain-seconds",
            "2",
            "--shutdown-timeout-seconds",
            "8",
            "--heartbeat-stale-seconds",
            "45",
        ])
        .unwrap();
        assert_eq!(config.lifecycle.deadline(), Duration::from_secs(10));
        assert_eq!(config.heartbeat_stale_seconds, 45);
    }

    #[test]
    fn zero_heartbeat_stale_seconds_is_rejected() {
        let result = WorkerConfig::try_parse_from(["test", "--heartbeat-stale-seconds", "0"]);
        assert!(result.is_err());
    }
}
