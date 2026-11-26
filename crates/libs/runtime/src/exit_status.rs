/// How a process run ended, mapped to an exit code that Kubernetes Job
/// `backoffLimit` and `podFailurePolicy` rules can match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitStatus {
    /// The run completed; exit code `0`.
    Success,
    /// A transient failure; exit code `1`. Counts toward the Job's
    /// `backoffLimit`.
    Retryable,
    /// A failure that retrying cannot fix (bad input, bad configuration);
    /// exit code `2`. Match it with a `podFailurePolicy` `FailJob` rule.
    Permanent,
    /// The run was cut short by a shutdown signal; exit code `143`
    /// (128 + SIGTERM).
    Interrupted,
}

impl ExitStatus {
    /// The process exit code for this status.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Retryable => 1,
            Self::Permanent => 2,
            Self::Interrupted => 143,
        }
    }
}

impl From<ExitStatus> for std::process::ExitCode {
    fn from(status: ExitStatus) -> Self {
        Self::from(status.code())
    }
}

#[cfg(test)]
mod tests {
    use std::process::ExitCode;

    use super::ExitStatus;

    #[test]
    fn codes_follow_the_taxonomy() {
        assert_eq!(ExitStatus::Success.code(), 0);
        assert_eq!(ExitStatus::Retryable.code(), 1);
        assert_eq!(ExitStatus::Permanent.code(), 2);
        assert_eq!(ExitStatus::Interrupted.code(), 143);
    }

    #[test]
    fn converts_into_a_process_exit_code() {
        assert_eq!(ExitCode::from(ExitStatus::Permanent), ExitCode::from(2));
    }
}
