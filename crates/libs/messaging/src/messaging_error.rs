use thiserror::Error;

type Source = Box<dyn std::error::Error + Send + Sync>;

/// Messaging-layer error taxonomy; transport backends classify their source
/// errors into these variants via `From` impls.
#[derive(Debug, Error)]
pub enum MessagingError {
    /// The broker is unreachable or degraded, or delivery timed out; a later
    /// attempt may succeed.
    #[error("messaging unavailable: {0}")]
    Unavailable(#[source] Source),

    /// The message or its destination was refused (unencodable payload,
    /// oversized message, unknown or unauthorized topic); retrying the same
    /// message cannot succeed.
    #[error("message rejected: {0}")]
    Rejected(#[source] Source),
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::MessagingError;

    #[test]
    fn display_includes_the_source_message() {
        let unavailable = MessagingError::Unavailable("boom".into());
        let message = unavailable.to_string();
        assert!(message.contains("messaging unavailable"));
        assert!(message.contains("boom"));

        let rejected = MessagingError::Rejected("boom".into());
        let message = rejected.to_string();
        assert!(message.contains("message rejected"));
        assert!(message.contains("boom"));
    }

    #[test]
    fn source_is_preserved() {
        let unavailable = MessagingError::Unavailable("boom".into());
        assert!(unavailable.source().is_some());

        let rejected = MessagingError::Rejected("boom".into());
        assert!(rejected.source().is_some());
    }
}
