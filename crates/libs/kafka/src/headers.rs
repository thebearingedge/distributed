//! Message-header adapters for W3C trace propagation.
//!
//! [`KafkaHeaderInjector`](crate::headers::KafkaHeaderInjector) and
//! [`KafkaHeaderExtractor`](crate::headers::KafkaHeaderExtractor) adapt
//! rdkafka message headers to the OpenTelemetry
//! [`Injector`](opentelemetry::propagation::Injector) and
//! [`Extractor`](opentelemetry::propagation::Extractor) traits, so
//! [`telemetry::propagation`] can carry trace context through Kafka
//! messages the same way it does through HTTP headers.

use opentelemetry::propagation::{Extractor, Injector};
use rdkafka::message::{Header, Headers, OwnedHeaders};
use tracing::Span;

use telemetry::propagation::{inject_current_into, link_parent_from};

/// [`Injector`] adapter that appends trace-context entries to rdkafka
/// [`OwnedHeaders`].
///
/// [`OwnedHeaders::insert`] consumes and rebuilds the header collection, so
/// [`set`](opentelemetry::propagation::Injector::set) takes the headers out
/// from behind the reference and puts the extended collection back.
pub struct KafkaHeaderInjector<'a>(pub &'a mut OwnedHeaders);

impl Injector for KafkaHeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        *self.0 = std::mem::take(self.0).insert(Header {
            key,
            value: Some(value.as_str()),
        });
    }
}

/// [`Extractor`] adapter that reads trace-context entries out of rdkafka
/// [`Headers`].
///
/// Only the first match of a key is returned, and header values that are
/// null or not UTF-8 are invisible to the propagator.
pub struct KafkaHeaderExtractor<'a, H: Headers>(pub &'a H);

impl<H: Headers> Extractor for KafkaHeaderExtractor<'_, H> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|header| header.key == key)
            .and_then(|Header { value, .. }| value)
            .and_then(|value| std::str::from_utf8(value).ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.iter().map(|Header { key, .. }| key).collect()
    }
}

/// Builds headers carrying the current span's trace context, ready to attach
/// to a produced message.
#[must_use]
pub(crate) fn current_trace_headers() -> OwnedHeaders {
    let mut headers = OwnedHeaders::new();
    inject_current_into(&mut KafkaHeaderInjector(&mut headers));
    headers
}

/// Links `span` to the parent trace context carried in a consumed message's
/// headers, if any.
pub(crate) fn link_parent<H: Headers>(span: &Span, headers: Option<&H>) {
    if let Some(headers) = headers {
        link_parent_from(span, &KafkaHeaderExtractor(headers));
    }
}

#[cfg(test)]
mod tests {
    use opentelemetry::trace::TraceContextExt;
    use opentelemetry::{
        Context, global,
        trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState},
    };
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use rdkafka::message::Header;

    use super::*;
    use telemetry::propagation::inject_context_into;

    fn remote_context(trace_id: TraceId, span_id: SpanId) -> Context {
        Context::new().with_remote_span_context(SpanContext::new(
            trace_id,
            span_id,
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        ))
    }

    #[test]
    fn injector_then_extractor_round_trips_a_value() {
        let mut headers = OwnedHeaders::new();
        KafkaHeaderInjector(&mut headers).set("traceparent", "abc".to_owned());

        let extractor = KafkaHeaderExtractor(&headers);
        assert_eq!(extractor.get("traceparent"), Some("abc"));
        assert_eq!(extractor.keys(), vec!["traceparent"]);
    }

    #[test]
    fn extractor_skips_non_utf8_values() {
        let headers = OwnedHeaders::new().insert(Header {
            key: "bad",
            value: Some(&[0xff_u8][..]),
        });

        assert_eq!(KafkaHeaderExtractor(&headers).get("bad"), None);
    }

    #[test]
    fn propagates_a_remote_span_context() {
        global::set_text_map_propagator(TraceContextPropagator::new());

        let trace_id = TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e4736").unwrap();
        let span_id = SpanId::from_hex("00f067aa0ba902b7").unwrap();
        let cx = remote_context(trace_id, span_id);

        let mut headers = OwnedHeaders::new();
        inject_context_into(&cx, &mut KafkaHeaderInjector(&mut headers));

        assert_eq!(
            KafkaHeaderExtractor(&headers).get("traceparent"),
            Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01")
        );
    }
}
