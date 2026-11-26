use opentelemetry::{Value, global, trace::TracerProvider};
use opentelemetry_otlp::{MetricExporter, SpanExporter};
use opentelemetry_sdk::{
    Resource, metrics::SdkMeterProvider, propagation::TraceContextPropagator,
    trace::SdkTracerProvider,
};
use tracing_opentelemetry::layer;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// Log level used when `RUST_LOG` is unset or unparseable.
const DEFAULT_FILTER_LEVEL: &str = "info";

/// The `OTEL_SERVICE_NAME` environment variable, whose value, when set and
/// non-empty, overrides `default_service_name` in [`resource`].
const OTEL_SERVICE_NAME: &str = "OTEL_SERVICE_NAME";

/// Resolves the `service.name` value: the environment override when it is set
/// and non-empty, otherwise `default_service_name`.
fn service_name(from_env: Option<String>, default_service_name: &'static str) -> Value {
    from_env
        .filter(|name| !name.is_empty())
        .map_or_else(|| Value::from(default_service_name), Value::from)
}

/// The OpenTelemetry resource identifying the service, combined with the
/// SDK's default resource detectors.
///
/// `service.name` is `default_service_name`, unless `OTEL_SERVICE_NAME` is set
/// and non-empty, in which case its value overrides it. Other attributes from
/// `OTEL_RESOURCE_ATTRIBUTES` are still applied by the SDK's detectors.
///
/// Build one resource and share it between
/// [`crate::provider::tracer_provider`] and
/// [`crate::provider::meter_provider`], so spans and metrics report the same
/// `service.name`.
#[must_use]
pub fn resource(default_service_name: &'static str) -> Resource {
    Resource::builder()
        .with_service_name(service_name(
            std::env::var(OTEL_SERVICE_NAME).ok(),
            default_service_name,
        ))
        .build()
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
/// `tracer_provider`. Log lines emitted inside a traced span carry its
/// `trace_id` and `span_id`.
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
        .with(crate::trace_ids::json_log_layer(std::io::stdout))
        .init();
}

#[cfg(test)]
mod tests {
    use opentelemetry::{Key, Value};

    use super::*;

    #[test]
    fn service_name_prefers_a_non_empty_override() {
        assert_eq!(
            service_name(Some("from_env".into()), "svc"),
            Value::from("from_env")
        );
    }

    #[test]
    fn service_name_falls_back_when_the_override_is_unset() {
        assert_eq!(service_name(None, "svc"), Value::from("svc"));
    }

    #[test]
    fn service_name_falls_back_when_the_override_is_empty() {
        assert_eq!(service_name(Some(String::new()), "svc"), Value::from("svc"));
    }

    #[test]
    fn resource_carries_the_service_name() {
        assert_eq!(
            resource("svc").get(&Key::from_static_str("service.name")),
            Some(service_name(std::env::var("OTEL_SERVICE_NAME").ok(), "svc"))
        );
    }
}
