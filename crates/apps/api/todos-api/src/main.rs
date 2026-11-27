//! todos-api application binary.

use std::process::ExitCode;

use clap::Parser;

use config::Args;
use todos_api::TodosApi;

mod config;

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    web::launch("todos_api", TodosApi::new(args.db), &args.api).await
}
