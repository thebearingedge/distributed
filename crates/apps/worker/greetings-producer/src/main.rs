//! greetings-producer application binary.

mod config;

use std::process::ExitCode;

use clap::Parser;

use config::Args;
use greetings_producer::GreetingsProducerWorker;

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let interval = args.greeting_interval();
    let worker =
        GreetingsProducerWorker::new(args.kafka, args.producer, interval, args.greeting_message);
    worker::launch("greetings_producer", worker, &args.worker).await
}
