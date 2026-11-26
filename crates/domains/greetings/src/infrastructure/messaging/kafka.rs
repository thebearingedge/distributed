mod kafka_greeting_message;
mod kafka_greeting_publisher;
mod kafka_greeting_subscriber;

pub use kafka_greeting_publisher::KafkaGreetingPublisher;
pub use kafka_greeting_subscriber::KafkaGreetingSubscriber;

/// Topic greetings are published to and consumed from.
pub const GREETINGS_TOPIC: &str = "greetings";
