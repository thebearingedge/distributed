//! Trace-context propagation between any text-map carrier and [`tracing`]
//! spans.
//!
//! The generic entry points work over any OpenTelemetry
//! [`Extractor`](opentelemetry::propagation::Extractor) or
//! [`Injector`](opentelemetry::propagation::Injector) carrier:
//! [`link_parent_from`](crate::propagation::link_parent_from) attaches the
//! parent context extracted from a carrier to a new span, and
//! [`inject_context_into`](crate::propagation::inject_context_into) /
//! [`inject_current_into`](crate::propagation::inject_current_into) write a
//! context into a carrier.
//!
//! For HTTP, [`HeaderMap`](http::HeaderMap) helpers wrap the generic ones:
//! [`inject_current`](crate::propagation::inject_current) writes the current
//! span's context into a header map;
//! [`link_parent`](crate::propagation::link_parent) does the reverse,
//! extracting a parent context from request headers and attaching it to a
//! new span. All of them go through the globally installed text-map
//! propagator (W3C `TraceContext`, installed by
//! [`crate::provider::init_subscriber`]).

use http::{HeaderMap, HeaderName, HeaderValue};
use opentelemetry::{
    Context, global,
    propagation::{Extractor, Injector},
    trace::TraceContextExt,
};
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// [`Extractor`] adapter that reads a parent context out of a
/// [`HeaderMap`].
struct HeaderExtractor<'a>(&'a HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(http::HeaderName::as_str).collect()
    }
}

/// Links `span` to the parent trace context carried in `headers`, if any.
///
/// A missing or invalid parent context is ignored and `span` stays a root
/// span; the result of [`OpenTelemetrySpanExt::set_parent`] is discarded.
/// Propagation is silent and does not fail the surrounding request.
pub fn link_parent(span: &Span, headers: &HeaderMap) {
    link_parent_from(span, &HeaderExtractor(headers));
}

/// [`Injector`] adapter that writes trace context into a [`HeaderMap`].
///
/// Invalid header names/values are dropped without logging.
struct HeaderInjector<'a>(&'a mut HeaderMap);

impl Injector for HeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            self.0.insert(name, value);
        }
    }
}

/// Writes `cx`'s trace context into `headers` using the global propagator.
pub fn inject_context(cx: &Context, headers: &mut HeaderMap) {
    inject_context_into(cx, &mut HeaderInjector(headers));
}

/// Writes the current span's trace context into `headers`.
pub fn inject_current(headers: &mut HeaderMap) {
    inject_current_into(&mut HeaderInjector(headers));
}

/// Links `span` to the parent trace context extracted from `extractor`'s
/// carrier, if it carries one.
///
/// A missing or invalid parent context is ignored and `span` stays a root
/// span; the result of [`OpenTelemetrySpanExt::set_parent`] is discarded.
/// Propagation is silent and does not fail the surrounding operation.
pub fn link_parent_from(span: &Span, extractor: &dyn Extractor) {
    let parent = global::get_text_map_propagator(|p| p.extract(extractor));
    if parent.span().span_context().is_valid() {
        let _ = span.set_parent(parent);
    }
}

/// Writes `cx`'s trace context into `injector`'s carrier using the global
/// propagator.
pub fn inject_context_into(cx: &Context, injector: &mut dyn Injector) {
    global::get_text_map_propagator(|p| p.inject_context(cx, injector));
}

/// Writes the current span's trace context into `injector`'s carrier.
pub fn inject_current_into(injector: &mut dyn Injector) {
    inject_context_into(&Span::current().context(), injector);
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use tracing_subscriber::layer::SubscriberExt;

    use super::*;

    fn remote_context(trace_id: TraceId, span_id: SpanId) -> Context {
        Context::new().with_remote_span_context(SpanContext::new(
            trace_id,
            span_id,
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        ))
    }

    fn observed_context(link: impl FnOnce(&tracing::Span)) -> opentelemetry::trace::SpanContext {
        let provider = SdkTracerProvider::builder().build();
        tracing::subscriber::with_default(
            tracing_subscriber::registry()
                .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test"))),
            || {
                let span = tracing::info_span!("work");
                link(&span);
                span.context().span().span_context().clone()
            },
        )
    }

    #[test]
    fn injected_context_can_be_extracted_again() {
        global::set_text_map_propagator(TraceContextPropagator::new());

        let trace_id = TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e4736").unwrap();
        let span_id = SpanId::from_hex("00f067aa0ba902b7").unwrap();
        let cx = Context::new().with_remote_span_context(SpanContext::new(
            trace_id,
            span_id,
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        ));

        let mut headers = HeaderMap::new();
        inject_context(&cx, &mut headers);

        assert_eq!(
            headers["traceparent"],
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
        );

        let extracted = global::get_text_map_propagator(|p| p.extract(&HeaderExtractor(&headers)));
        assert_eq!(extracted.span().span_context().trace_id(), trace_id);
    }

    #[test]
    fn inject_context_into_writes_traceparent_to_any_injector() {
        global::set_text_map_propagator(TraceContextPropagator::new());

        let trace_id = TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e4736").unwrap();
        let span_id = SpanId::from_hex("00f067aa0ba902b7").unwrap();
        let cx = remote_context(trace_id, span_id);

        let mut map = HashMap::new();
        inject_context_into(&cx, &mut map);

        assert_eq!(
            map["traceparent"],
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
        );
    }

    #[test]
    fn link_parent_from_links_a_span_to_the_extracted_parent() {
        global::set_text_map_propagator(TraceContextPropagator::new());

        let trace_id = TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e4736").unwrap();
        let span_id = SpanId::from_hex("00f067aa0ba902b7").unwrap();
        let cx = remote_context(trace_id, span_id);
        let mut map = HashMap::new();
        inject_context_into(&cx, &mut map);

        let observed = observed_context(|span| link_parent_from(span, &map));

        assert_eq!(observed.trace_id(), trace_id);
    }

    #[test]
    fn link_parent_from_ignores_a_missing_parent() {
        global::set_text_map_propagator(TraceContextPropagator::new());

        let trace_id = TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e4736").unwrap();
        let map: HashMap<String, String> = HashMap::new();

        let observed = observed_context(|span| link_parent_from(span, &map));

        assert_ne!(observed.trace_id(), trace_id);
    }
}
