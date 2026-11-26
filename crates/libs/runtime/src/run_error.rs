/// A type-erased error source.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Why a worker or job run failed, classified by whether retrying can help.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// A transient failure that a retry may fix.
    #[error("retryable failure: {0}")]
    Retryable(#[source] BoxError),
    /// A failure that retrying cannot fix.
    #[error("permanent failure: {0}")]
    Permanent(#[source] BoxError),
}

impl RunError {
    /// Classifies `source` as a failure a retry may fix.
    #[must_use]
    pub fn retryable(source: impl Into<BoxError>) -> Self {
        Self::Retryable(source.into())
    }

    /// Classifies `source` as a failure retrying cannot fix.
    #[must_use]
    pub fn permanent(source: impl Into<BoxError>) -> Self {
        Self::Permanent(source.into())
    }

    /// The exit status this failure maps to.
    #[must_use]
    pub fn exit_status(&self) -> crate::exit_status::ExitStatus {
        match self {
            Self::Retryable(_) => crate::exit_status::ExitStatus::Retryable,
            Self::Permanent(_) => crate::exit_status::ExitStatus::Permanent,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use crate::exit_status::ExitStatus;

    use super::RunError;

    #[test]
    fn retryable_maps_to_exit_status_retryable() {
        let err = RunError::retryable("connection reset");
        assert_eq!(err.exit_status(), ExitStatus::Retryable);
    }

    #[test]
    fn permanent_maps_to_exit_status_permanent() {
        let err = RunError::permanent("bad config");
        assert_eq!(err.exit_status(), ExitStatus::Permanent);
    }

    #[test]
    fn display_includes_the_source_message() {
        let err = RunError::permanent("bad config");
        assert_eq!(err.to_string(), "permanent failure: bad config");
    }

    #[test]
    fn source_is_preserved() {
        let err = RunError::retryable("connection reset");
        let source = Error::source(&err).expect("source is preserved");
        assert_eq!(source.to_string(), "connection reset");
    }
}
