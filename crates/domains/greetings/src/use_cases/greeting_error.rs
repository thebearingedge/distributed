//! Errors raised by the greetings use cases.

use thiserror::Error;

use messaging::MessagingError;

/// Errors surfaced by the greetings use cases.
#[derive(Debug, Error)]
pub enum GreetingError {
    /// The message failed validation (see
    /// [`GreetingModel::has_valid_message`](crate::models::greeting::GreetingModel::has_valid_message)).
    #[error("greeting message must not be blank")]
    BlankMessage,

    /// The greeting could not be published.
    #[error(transparent)]
    Messaging(#[from] MessagingError),
}
