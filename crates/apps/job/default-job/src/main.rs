//! default-job application binary.

mod config;

use std::process::ExitCode;

use clap::Parser;

use config::Args;
use default_job::NoopJob;

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    jobs::launch("default_job", NoopJob, &args.lifecycle).await
}
