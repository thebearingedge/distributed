//! greetings-consumer application binary.

mod config;
mod greetings_consumer_worker;

use std::process::ExitCode;

use clap::Parser;

use config::Args;
use greetings_consumer_worker::GreetingsConsumerWorker;

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let worker = GreetingsConsumerWorker::new(args.kafka, args.consumer);
    worker::launch("greetings_consumer", &worker, &args.worker).await
}
