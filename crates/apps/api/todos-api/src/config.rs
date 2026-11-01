//! Command-line configuration for the todos-api binary.

use database::DbConfig;
use web::config::ApiConfig;

/// Command-line configuration for the todos API binary.
#[derive(Debug, clap::Parser)]
pub(crate) struct Args {
    /// Web/runtime configuration.
    #[command(flatten)]
    pub(crate) api: ApiConfig,
    /// Database configuration.
    #[command(flatten)]
    pub(crate) db: DbConfig,
}
