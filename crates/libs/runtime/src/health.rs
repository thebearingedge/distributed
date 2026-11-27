//! Named, type-erased health checks for liveness/readiness probes
//! ([`crate::health::HealthCheck`]).

use std::{fmt, future::Future, pin::Pin, sync::Arc};

/// The result type of a health check: errors are erased to a `String`
/// message so a generic probe server can log and report them without
/// knowing the underlying error types.
pub type HealthCheckFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;

/// A named health check (readiness or liveness; e.g. "database") wrapping an
/// arbitrary async fallible closure. Checks are only ever asked to succeed or
/// fail; the error message is surfaced via logs, not the response body.
#[derive(Clone)]
pub struct HealthCheck {
    name: &'static str,
    run: Arc<dyn Fn() -> HealthCheckFuture + Send + Sync>,
}

impl HealthCheck {
    /// Creates a check from a closure producing a fallible future.
    pub fn new<F, Fut, E>(name: &'static str, check: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), E>> + Send + 'static,
        E: std::fmt::Display,
    {
        Self {
            name,
            run: Arc::new(move || {
                let fut = check();
                Box::pin(async move { fut.await.map_err(|e| e.to_string()) })
            }),
        }
    }

    /// Creates a check whose closure captures a cloned resource (e.g. a
    /// database pool), cloning it anew for each probe.
    pub fn with_resource<R, F, Fut, E>(name: &'static str, resource: R, check: F) -> Self
    where
        R: Clone + Send + Sync + 'static,
        F: Fn(R) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), E>> + Send + 'static,
        E: std::fmt::Display,
    {
        Self::new(name, move || check(resource.clone()))
    }

    /// The check's name, as reported in probe responses.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Runs the check once. A failing check reports the `Display` of its
    /// error.
    pub fn run(&self) -> HealthCheckFuture {
        (self.run)()
    }
}

impl fmt::Debug for HealthCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HealthCheck")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::HealthCheck;

    #[tokio::test]
    async fn name_is_reported() {
        let check = HealthCheck::new("postgres", || async { Ok::<(), String>(()) });
        assert_eq!(check.name(), "postgres");
    }

    #[tokio::test]
    async fn passing_check_runs_ok() {
        let check = HealthCheck::new("up", || async { Ok::<(), String>(()) });
        assert!(check.run().await.is_ok());
    }

    #[tokio::test]
    async fn failing_check_reports_the_error_display() {
        #[derive(Debug)]
        struct Boom;

        impl std::fmt::Display for Boom {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "boom happened")
            }
        }

        let check = HealthCheck::new("down", || async { Err::<(), Boom>(Boom) });
        let error = check.run().await.unwrap_err();
        assert_eq!(error, "boom happened");
    }

    #[tokio::test]
    async fn with_resource_clones_the_resource_per_run() {
        let counter = Arc::new(AtomicUsize::new(0));
        let check = HealthCheck::with_resource(
            "counted",
            Arc::clone(&counter),
            |counter: Arc<AtomicUsize>| async move {
                counter.fetch_add(1, Ordering::Relaxed);
                Ok::<(), String>(())
            },
        );

        check.run().await.unwrap();
        check.run().await.unwrap();

        assert_eq!(counter.load(Ordering::Relaxed), 2);
    }
}
