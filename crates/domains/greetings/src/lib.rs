//! The greetings domain: an example of publishing and receiving domain
//! events through a publisher port ([`GreetingPublisher`]), with Kafka
//! adapters in `infrastructure`.

pub mod models;

mod infrastructure;
mod use_cases;

pub use infrastructure::messaging::kafka::{
    GREETINGS_TOPIC, KafkaGreetingPublisher, KafkaGreetingSubscriber,
};
pub use use_cases::{
    GreetingError, greeting_publisher::GreetingPublisher,
    receive_greeting_interactor::ReceiveGreetingInteractor,
    send_greeting_interactor::SendGreetingInteractor,
};
