//! Command-line configuration for the default job binary.

use runtime::config::LifecycleConfig;

/// Command-line configuration for the default job binary.
#[derive(Debug, clap::Parser)]
pub(crate) struct Args {
    /// Process lifecycle configuration.
    #[command(flatten)]
    pub(crate) lifecycle: LifecycleConfig,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Args;

    #[test]
    fn parses_lifecycle_flags() {
        let args = Args::try_parse_from(["test", "--shutdown-timeout-seconds", "9"]).unwrap();
        assert_eq!(args.lifecycle.shutdown_timeout_seconds, 9);
    }
}
