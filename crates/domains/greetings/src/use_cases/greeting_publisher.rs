//! Publisher port for greetings, implemented by infrastructure adapters.

use async_trait::async_trait;

use crate::models::greeting::GreetingModel;
use messaging::MessagingError;

/// Outbound port that publishes greetings to subscribers.
///
/// Implementations own the delivery to the messaging backend and classify
/// failures into [`MessagingError`] variants.
#[async_trait]
pub trait GreetingPublisher: std::fmt::Debug + Send + Sync {
    /// Publishes `greeting` to subscribers.
    ///
    /// # Errors
    ///
    /// Returns a [`MessagingError`] classifying why the greeting could not be
    /// published.
    async fn publish(&self, greeting: &GreetingModel) -> Result<(), MessagingError>;
}
