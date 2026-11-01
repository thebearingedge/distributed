//! Command-line configuration for the default API binary.

use web::config::ApiConfig;

/// Command-line configuration for the default API binary.
#[derive(Debug, clap::Parser)]
pub(crate) struct Args {
    /// Web/runtime configuration.
    #[command(flatten)]
    pub(crate) api: ApiConfig,
}
