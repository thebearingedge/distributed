use std::time::Duration;

/// Default for [`crate::config::LifecycleConfig::shutdown_drain_seconds`].
pub const SHUTDOWN_DRAIN_DEFAULT_SECONDS: u64 = 5;
/// Default for [`crate::config::LifecycleConfig::shutdown_timeout_seconds`].
pub const SHUTDOWN_TIMEOUT_DEFAULT_SECONDS: u64 = 15;
/// Default for [`crate::config::LifecycleConfig::shutdown_flush_timeout_seconds`].
pub const SHUTDOWN_FLUSH_TIMEOUT_DEFAULT_SECONDS: u64 = 5;

/// Process lifecycle configuration shared by every process shape (API,
/// worker, job): how long to drain after the shutdown signal, how long tasks
/// may then take to stop, and how long the final telemetry flush may take.
///
/// The whole shutdown must fit inside the pod's
/// `terminationGracePeriodSeconds`:
/// `SHUTDOWN_DRAIN_SECONDS + SHUTDOWN_TIMEOUT_SECONDS +
/// SHUTDOWN_FLUSH_TIMEOUT_SECONDS < terminationGracePeriodSeconds`
/// (see [`crate::config::LifecycleConfig::termination_budget`]). CLI args
/// fall back to env vars, which fall back to declared defaults.
#[derive(Debug, clap::Parser)]
pub struct LifecycleConfig {
    /// Seconds to keep serving after the shutdown signal, with readiness
    /// reporting shutting down, before servers stop accepting connections.
    #[arg(
        long,
        env = "SHUTDOWN_DRAIN_SECONDS",
        default_value_t = SHUTDOWN_DRAIN_DEFAULT_SECONDS
    )]
    pub shutdown_drain_seconds: u64,
    /// Seconds after the drain that tasks may take to stop before they are
    /// abandoned and the process exits 143; must be at least 1.
    #[arg(
        long,
        env = "SHUTDOWN_TIMEOUT_SECONDS",
        default_value_t = SHUTDOWN_TIMEOUT_DEFAULT_SECONDS,
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    pub shutdown_timeout_seconds: u64,
    /// Seconds the final telemetry flush may take; must be at least 1.
    #[arg(
        long,
        env = "SHUTDOWN_FLUSH_TIMEOUT_SECONDS",
        default_value_t = SHUTDOWN_FLUSH_TIMEOUT_DEFAULT_SECONDS,
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    pub shutdown_flush_timeout_seconds: u64,
}

impl LifecycleConfig {
    /// Time to keep serving after the shutdown signal.
    #[must_use]
    pub fn drain_delay(&self) -> Duration {
        Duration::from_secs(self.shutdown_drain_seconds)
    }

    /// Time tasks may take to stop after the drain.
    #[must_use]
    pub fn shutdown_timeout(&self) -> Duration {
        Duration::from_secs(self.shutdown_timeout_seconds)
    }

    /// Time the final telemetry flush may take.
    #[must_use]
    pub fn flush_timeout(&self) -> Duration {
        Duration::from_secs(self.shutdown_flush_timeout_seconds)
    }

    /// Time from the shutdown trigger until unfinished tasks are abandoned:
    /// the drain delay plus the shutdown timeout.
    #[must_use]
    pub fn deadline(&self) -> Duration {
        self.drain_delay() + self.shutdown_timeout()
    }

    /// The longest a shutdown can take: the deadline plus the flush
    /// timeout. Keep it below the pod's `terminationGracePeriodSeconds`.
    #[must_use]
    pub fn termination_budget(&self) -> Duration {
        self.deadline() + self.flush_timeout()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use clap::Parser;

    use super::LifecycleConfig;

    #[test]
    fn defaults() {
        let config = LifecycleConfig::try_parse_from(["test"]).unwrap();
        assert_eq!(config.shutdown_drain_seconds, 5);
        assert_eq!(config.shutdown_timeout_seconds, 15);
        assert_eq!(config.shutdown_flush_timeout_seconds, 5);
    }

    #[test]
    fn default_drain_is_nonzero() {
        let config = LifecycleConfig::try_parse_from(["test"]).unwrap();
        assert!(config.drain_delay() > Duration::ZERO);
    }

    #[test]
    fn cli_args_override_defaults() {
        let config = LifecycleConfig::try_parse_from([
            "test",
            "--shutdown-drain-seconds",
            "3",
            "--shutdown-timeout-seconds",
            "7",
            "--shutdown-flush-timeout-seconds",
            "2",
        ])
        .unwrap();
        assert_eq!(config.shutdown_drain_seconds, 3);
        assert_eq!(config.shutdown_timeout_seconds, 7);
        assert_eq!(config.shutdown_flush_timeout_seconds, 2);
    }

    #[test]
    fn accessors_convert_to_durations() {
        let config = LifecycleConfig::try_parse_from([
            "test",
            "--shutdown-drain-seconds",
            "3",
            "--shutdown-timeout-seconds",
            "7",
            "--shutdown-flush-timeout-seconds",
            "2",
        ])
        .unwrap();
        assert_eq!(config.drain_delay(), Duration::from_secs(3));
        assert_eq!(config.shutdown_timeout(), Duration::from_secs(7));
        assert_eq!(config.flush_timeout(), Duration::from_secs(2));
        assert_eq!(config.deadline(), Duration::from_secs(10));
        assert_eq!(config.termination_budget(), Duration::from_secs(12));
    }

    #[test]
    fn default_budget_fits_the_default_termination_grace_period() {
        let config = LifecycleConfig::try_parse_from(["test"]).unwrap();
        assert!(config.termination_budget() < Duration::from_secs(30));
    }

    #[test]
    fn zero_shutdown_timeout_is_rejected() {
        assert!(
            LifecycleConfig::try_parse_from(["test", "--shutdown-timeout-seconds", "0"]).is_err()
        );
    }

    #[test]
    fn zero_flush_timeout_is_rejected() {
        assert!(
            LifecycleConfig::try_parse_from(["test", "--shutdown-flush-timeout-seconds", "0"])
                .is_err()
        );
    }

    #[test]
    fn zero_drain_is_accepted() {
        assert!(LifecycleConfig::try_parse_from(["test", "--shutdown-drain-seconds", "0"]).is_ok());
    }

    #[test]
    fn flattens_into_an_outer_parser() {
        #[derive(clap::Parser)]
        struct Args {
            #[command(flatten)]
            lifecycle: LifecycleConfig,
        }
        let args = Args::try_parse_from(["test", "--shutdown-timeout-seconds", "9"]).unwrap();
        assert_eq!(args.lifecycle.shutdown_timeout_seconds, 9);
    }
}
