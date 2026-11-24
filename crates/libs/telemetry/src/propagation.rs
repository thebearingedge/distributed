//! Trace-context propagation between HTTP headers and [`tracing`] spans.
//!
//! [`inject_current`](crate::propagation::inject_current) writes the current
//! span's context into a [`HeaderMap`](http::HeaderMap);
//! [`link_parent`](crate::propagation::link_parent) does the reverse,
//! extracting a parent context from request headers and attaching it to a
//! new span. Both go through the globally installed text-map propagator (W3C
//! `TraceContext`, installed by
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
    let parent = global::get_text_map_propagator(|p| p.extract(&HeaderExtractor(headers)));
    if parent.span().span_context().is_valid() {
        let _ = span.set_parent(parent);
    }
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
    global::get_text_map_propagator(|p| p.inject_context(cx, &mut HeaderInjector(headers)));
}

/// Writes the current span's trace context into `headers`.
pub fn inject_current(headers: &mut HeaderMap) {
    inject_context(&Span::current().context(), headers);
}

#[cfg(test)]
mod tests {
    use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
    use opentelemetry_sdk::propagation::TraceContextPropagator;

    use super::*;

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
}
