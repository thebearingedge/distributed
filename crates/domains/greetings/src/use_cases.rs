//! Business rules for the greetings domain: the
//! [`greeting_publisher::GreetingPublisher`] port, the
//! [`greeting_error::GreetingError`] error set, and the
//! [`send_greeting_interactor::SendGreetingInteractor`] and
//! [`receive_greeting_interactor::ReceiveGreetingInteractor`] that drive it.

pub mod greeting_error;
pub mod greeting_publisher;
pub mod receive_greeting_interactor;
pub mod send_greeting_interactor;

pub use greeting_error::GreetingError;
