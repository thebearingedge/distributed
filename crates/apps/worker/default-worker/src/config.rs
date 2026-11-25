//! Command-line configuration for the default worker binary.

use worker::WorkerConfig;

/// Command-line configuration for the default worker binary.
#[derive(Debug, clap::Parser)]
pub(crate) struct Args {
    /// Worker runtime configuration.
    #[command(flatten)]
    pub(crate) worker: WorkerConfig,
}
