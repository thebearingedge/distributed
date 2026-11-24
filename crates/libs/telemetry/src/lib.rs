//! Telemetry bootstrap: the [`Telemetry`] guard for one-call initialization
//! and shutdown of the global tracing subscriber, the OpenTelemetry span
//! exporter and meter provider, plus trace-context propagation helpers,
//! shared metric helpers for instrumentation libraries, and an in-memory
//! metrics harness behind the `test-support` feature.

/// The [`crate::Telemetry`] guard: one-call initialization and shutdown of the
/// tracer and meter providers.
pub mod bootstrap;

/// Shared metric helpers for instrumentation libraries: seconds-based
/// duration histograms ([`crate::metrics::duration_histogram`]) and the
/// [`crate::metrics::InFlightGuard`] for in-flight work.
pub mod metrics;

/// Trace-context propagation between HTTP headers and [`tracing`] spans.
pub mod propagation;

/// OTLP tracer/meter provider building blocks and the one-shot global
/// subscriber initialization.
pub mod provider;

/// An in-memory meter provider and readable data points for asserting
/// on recorded metrics ([`crate::test_support::MetricsHarness`]) —
/// compiled only for tests or when the `test-support` feature is enabled.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use bootstrap::Telemetry;
