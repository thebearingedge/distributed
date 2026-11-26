//! default-job application binary.

mod config;

use std::process::ExitCode;

use clap::Parser;

use config::Args;
use jobs::NoopJob;

#[tokio::main]
async fn main() -> ExitCode {
    Args::parse();
    jobs::launch("default_job", &NoopJob).await
}
