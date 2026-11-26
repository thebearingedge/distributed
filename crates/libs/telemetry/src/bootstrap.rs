//! The [`crate::Telemetry`] guard.

use opentelemetry::global;
use opentelemetry_sdk::{
    error::OTelSdkResult, metrics::SdkMeterProvider, trace::SdkTracerProvider,
};

use crate::provider;

/// The process's OTLP tracer and meter providers, initialized together by
/// [`Telemetry::init`] and flushed together by [`Telemetry::shutdown`].
pub struct Telemetry {
    tracer_provider: SdkTracerProvider,
    meter_provider: SdkMeterProvider,
}

impl Telemetry {
    /// Builds both providers on one shared [`opentelemetry_sdk::Resource`] for
    /// `service_name`, and
    /// installs the global tracing subscriber and W3C `TraceContext`
    /// propagator ([`crate::provider::init_subscriber`]) and the global meter
    /// provider.
    ///
    /// `service_name` is the compiled-in default; `OTEL_SERVICE_NAME`, when
    /// set and non-empty, overrides it. Other `OTEL_RESOURCE_ATTRIBUTES` keys
    /// are still applied.
    ///
    /// Call it once, from within a Tokio runtime, before creating any meters.
    ///
    /// # Panics
    ///
    /// Panics on invalid exporter configuration, and if called twice (a global
    /// subscriber can only be installed once). The exporters are built before
    /// any global state is installed, so a panic leaves the process
    /// unmodified.
    #[must_use]
    pub fn init(service_name: &'static str) -> Self {
        let resource = provider::resource(service_name);
        let tracer_provider = provider::tracer_provider(resource.clone());
        let meter_provider = provider::meter_provider(resource);
        provider::init_subscriber(&tracer_provider, service_name);
        global::set_meter_provider(meter_provider.clone());
        Self {
            tracer_provider,
            meter_provider,
        }
    }

    /// The meter provider to pass to instrumentation such as `web::metrics`
    /// and `database::metrics`.
    #[must_use]
    pub fn meter_provider(&self) -> &SdkMeterProvider {
        &self.meter_provider
    }

    /// Shuts down the meter provider, performing the final metric export, and
    /// then the tracer provider, flushing in-flight span batches. Both
    /// shutdowns always run.
    ///
    /// # Errors
    ///
    /// Returns the first [`opentelemetry_sdk::error::OTelSdkError`] in that
    /// order.
    pub fn shutdown(&self) -> OTelSdkResult {
        let metrics = self.meter_provider.shutdown();
        let traces = self.tracer_provider.shutdown();
        metrics.and(traces)
    }
}

#[cfg(test)]
mod tests {
    use opentelemetry_sdk::error::OTelSdkError;

    use super::*;

    #[test]
    fn shutdown_succeeds_for_fresh_providers() {
        let telemetry = Telemetry {
            tracer_provider: SdkTracerProvider::builder().build(),
            meter_provider: SdkMeterProvider::builder().build(),
        };

        assert!(telemetry.shutdown().is_ok());
    }

    #[test]
    fn shutdown_runs_the_tracer_shutdown_even_when_the_meter_shutdown_fails() {
        let telemetry = Telemetry {
            tracer_provider: SdkTracerProvider::builder().build(),
            meter_provider: SdkMeterProvider::builder().build(),
        };

        telemetry.meter_provider.shutdown().unwrap();

        assert!(matches!(
            telemetry.shutdown(),
            Err(OTelSdkError::AlreadyShutdown)
        ));
        assert!(matches!(
            telemetry.tracer_provider.shutdown(),
            Err(OTelSdkError::AlreadyShutdown)
        ));
    }
}
