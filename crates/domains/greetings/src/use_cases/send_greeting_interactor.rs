//! Primary entry point for publishing greetings, driving a
//! [`GreetingPublisher`](crate::use_cases::greeting_publisher::GreetingPublisher)
//! implementation behind the [`SendGreetingInteractor`].

use std::sync::Arc;

use tracing::instrument;

use crate::{
    models::greeting::GreetingModel, use_cases::greeting_error::GreetingError,
    use_cases::greeting_publisher::GreetingPublisher,
};

/// Publishes greetings through the injected publisher port.
#[derive(Clone, Debug)]
pub struct SendGreetingInteractor {
    publisher: Arc<dyn GreetingPublisher>,
}

impl SendGreetingInteractor {
    /// Creates an interactor backed by the given publisher.
    #[must_use]
    pub fn new(publisher: Arc<dyn GreetingPublisher>) -> Self {
        Self { publisher }
    }

    /// Builds a greeting from `message` and publishes it.
    ///
    /// # Errors
    ///
    /// Returns [`GreetingError::BlankMessage`] if the message fails validation
    /// (see
    /// [`GreetingModel::has_valid_message`](crate::models::greeting::GreetingModel::has_valid_message)),
    /// in which case nothing is published, and [`GreetingError::Messaging`] if
    /// the publisher cannot deliver the greeting.
    #[instrument(skip_all)]
    pub async fn send(&self, message: &str) -> Result<GreetingModel, GreetingError> {
        let greeting = GreetingModel::new(message);

        if !greeting.has_valid_message() {
            return Err(GreetingError::BlankMessage);
        }

        self.publisher.publish(&greeting).await?;

        Ok(greeting)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use messaging::MessagingError;

    use super::*;

    #[derive(Debug, Default)]
    struct RecordingPublisher {
        published: Mutex<Vec<GreetingModel>>,
        fail_with_unavailable: bool,
    }

    #[async_trait]
    impl GreetingPublisher for RecordingPublisher {
        async fn publish(&self, greeting: &GreetingModel) -> Result<(), MessagingError> {
            if self.fail_with_unavailable {
                return Err(MessagingError::Unavailable("broker unreachable".into()));
            }

            self.published
                .lock()
                .expect("publisher lock")
                .push(greeting.clone());

            Ok(())
        }
    }

    #[tokio::test]
    async fn send_publishes_a_valid_greeting() {
        let publisher = Arc::new(RecordingPublisher::default());
        let interactor = SendGreetingInteractor::new(publisher.clone());

        let greeting = interactor.send("hello").await.expect("send succeeds");

        let recorded = publisher.published.lock().expect("publisher lock");
        assert_eq!(recorded.as_slice(), &[greeting]);
    }

    #[tokio::test]
    async fn send_rejects_a_blank_message_without_publishing() {
        let publisher = Arc::new(RecordingPublisher::default());
        let interactor = SendGreetingInteractor::new(publisher.clone());

        for message in ["", "   ", "\n"] {
            let error = interactor
                .send(message)
                .await
                .expect_err("blank message is rejected");

            assert!(
                matches!(error, GreetingError::BlankMessage),
                "error: {error:?}"
            );
        }

        assert!(
            publisher
                .published
                .lock()
                .expect("publisher lock")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn send_surfaces_publisher_failures() {
        let publisher = Arc::new(RecordingPublisher {
            fail_with_unavailable: true,
            ..RecordingPublisher::default()
        });
        let interactor = SendGreetingInteractor::new(publisher);

        let error = interactor.send("hello").await.expect_err("send fails");

        assert!(
            matches!(
                error,
                GreetingError::Messaging(MessagingError::Unavailable(_))
            ),
            "error: {error:?}"
        );
    }
}
