//! default-worker application binary.

mod config;

use std::process::ExitCode;

use clap::Parser;

use config::Args;
use default_worker::InertWorker;

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    worker::launch("default_worker", InertWorker, &args.worker).await
}
