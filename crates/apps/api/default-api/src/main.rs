//! default-api application binary.

mod config;

use std::process::ExitCode;

use clap::Parser;

use config::Args;
use default_api::DefaultApi;

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    web::launch("default_api", DefaultApi, &args.api).await
}
