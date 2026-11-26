use uuid::Uuid;

/// A greeting as published by the producer and received by the consumer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GreetingModel {
    /// Unique identifier of the greeting, assigned at creation time.
    pub id: Uuid,
    /// Greeting message; valid when it has non-whitespace content (see
    /// [`Self::has_valid_message`]).
    pub message: String,
}

impl GreetingModel {
    /// A new greeting with a fresh time-ordered id.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            id: Uuid::now_v7(),
            message: message.into(),
        }
    }

    /// Whether the message has non-whitespace content.
    #[must_use]
    pub fn has_valid_message(&self) -> bool {
        !self.message.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_assigns_a_v7_id() {
        let greeting = GreetingModel::new("hello");

        assert_eq!(greeting.id.get_version_num(), 7);
    }

    #[test]
    fn blank_messages_are_invalid() {
        for message in ["", "   ", "\n"] {
            let greeting = GreetingModel::new(message);

            assert!(!greeting.has_valid_message(), "invalid: {message:?}");
        }
    }

    #[test]
    fn non_blank_messages_are_valid() {
        for message in ["hello", "  hello  ", "h"] {
            let greeting = GreetingModel::new(message);

            assert!(greeting.has_valid_message(), "valid: {message:?}");
        }
    }
}
