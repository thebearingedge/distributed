//! The greetings producer application: publishes a greeting on an interval
//! until shutdown, then flushes.

pub mod greetings_producer_worker;

pub use greetings_producer_worker::GreetingsProducerWorker;
