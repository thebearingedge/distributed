use opentelemetry::{global, trace::TracerProvider};
use opentelemetry_otlp::{MetricExporter, SpanExporter};
use opentelemetry_sdk::{
    Resource, metrics::SdkMeterProvider, propagation::TraceContextPropagator,
    trace::SdkTracerProvider,
};
use tracing_opentelemetry::layer;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Log level used when `RUST_LOG` is unset or unparseable.
const DEFAULT_FILTER_LEVEL: &str = "info";

/// The OpenTelemetry resource identifying the service, combined with the
/// SDK's default resource detectors.
///
/// Build one resource and share it between
/// [`crate::provider::tracer_provider`] and
/// [`crate::provider::meter_provider`], so spans and metrics report the same
/// `service.name`.
#[must_use]
pub fn resource(service_name: &'static str) -> Resource {
    Resource::builder().with_service_name(service_name).build()
}

/// Builds an OTLP gRPC batch-exporting tracer provider and installs no global
/// state.
///
/// The caller owns the returned provider and **must call
/// [`SdkTracerProvider::shutdown`] on process exit** to flush in-flight span
/// batches.
///
/// Must be called from within a Tokio runtime.
///
/// # Panics
///
/// Panics on invalid [`SpanExporter`] configuration, so that startup fails
/// loudly before any global state is installed.
#[must_use]
pub fn tracer_provider(resource: Resource) -> SdkTracerProvider {
    let exporter = SpanExporter::builder()
        .with_tonic()
        .build()
        .expect("invalid otlp exporter configuration");

    SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build()
}

/// Builds an OTLP gRPC meter provider that exports periodically (every 60 s by
/// default; the `OTEL_METRIC_EXPORT_INTERVAL` environment variable overrides
/// it, in milliseconds) and installs no global state.
///
/// The caller owns the returned provider and **must call
/// [`SdkMeterProvider::shutdown`] on process exit**, which performs the final
/// metric export.
///
/// Must be called from within a Tokio runtime.
///
/// # Panics
///
/// Panics on invalid [`MetricExporter`] configuration, so that startup fails
/// loudly before any global state is installed.
#[must_use]
pub fn meter_provider(resource: Resource) -> SdkMeterProvider {
    let exporter = MetricExporter::builder()
        .with_tonic()
        .build()
        .expect("invalid otlp metric exporter configuration");

    SdkMeterProvider::builder()
        .with_periodic_exporter(exporter)
        .with_resource(resource)
        .build()
}

/// Installs the global W3C `TraceContext` propagator and the global tracing
/// subscriber: JSON to stdout, an [`EnvFilter`] driven by `RUST_LOG` with a
/// `DEFAULT_FILTER_LEVEL` fallback, and spans exported through
/// `tracer_provider`.
///
/// This is a one-time, process-global side effect.
///
/// # Panics
///
/// Panics if a global subscriber is already installed.
pub fn init_subscriber(tracer_provider: &SdkTracerProvider, service_name: &'static str) {
    global::set_text_map_propagator(TraceContextPropagator::new());

    let tracer = tracer_provider.tracer(service_name);

    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER_LEVEL)),
        )
        .with(layer().with_tracer(tracer))
        .with(
            fmt::layer()
                .json()
                .with_current_span(true)
                .with_span_list(false)
                .flatten_event(true),
        )
        .init();
}

#[cfg(test)]
mod tests {
    use opentelemetry::{Key, Value};

    use super::*;

    #[test]
    fn resource_carries_the_service_name() {
        assert_eq!(
            resource("svc").get(&Key::from_static_str("service.name")),
            Some(Value::from("svc"))
        );
    }
}
