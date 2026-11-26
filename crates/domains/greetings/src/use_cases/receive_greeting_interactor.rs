//! Primary entry point for receiving greetings: validates and logs an
//! incoming [`GreetingModel`] behind the [`ReceiveGreetingInteractor`].

use tracing::instrument;

use crate::{models::greeting::GreetingModel, use_cases::greeting_error::GreetingError};

/// Receives greetings handed over by the messaging subscriber.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReceiveGreetingInteractor;

impl ReceiveGreetingInteractor {
    /// Validates and logs an incoming greeting.
    ///
    /// # Errors
    ///
    /// Returns [`GreetingError::BlankMessage`] if the greeting fails validation
    /// (see
    /// [`GreetingModel::has_valid_message`](crate::models::greeting::GreetingModel::has_valid_message)).
    #[instrument(skip_all)]
    pub fn receive(&self, greeting: &GreetingModel) -> Result<(), GreetingError> {
        if !greeting.has_valid_message() {
            return Err(GreetingError::BlankMessage);
        }

        tracing::info!(
            greeting.id = %greeting.id,
            greeting.message = %greeting.message,
            "greeting received"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receive_accepts_a_valid_greeting() {
        let interactor = ReceiveGreetingInteractor;

        let result = interactor.receive(&GreetingModel::new("hello"));

        assert!(result.is_ok(), "result: {result:?}");
    }

    #[test]
    fn receive_rejects_a_blank_greeting() {
        let interactor = ReceiveGreetingInteractor;

        for message in ["", "   ", "\n"] {
            let error = interactor
                .receive(&GreetingModel::new(message))
                .expect_err("blank greeting is rejected");

            assert!(
                matches!(error, GreetingError::BlankMessage),
                "error: {error:?}"
            );
        }
    }
}
