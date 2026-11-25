use std::sync::OnceLock;

use opentelemetry::trace::{SpanContext, SpanId, TraceContextExt, TraceId};
use tracing::{Dispatch, Event, Subscriber, dispatcher::WeakDispatch};
use tracing_opentelemetry::get_otel_context;
use tracing_subscriber::{
    Layer,
    fmt::{
        self, FmtContext, FormatEvent, FormatFields, MakeWriter,
        format::{JsonFields, Writer},
    },
    layer::Context,
    registry::LookupSpan,
};

const TRACE_ID_FIELD: &str = "trace_id";
const SPAN_ID_FIELD: &str = "span_id";

#[derive(Clone, Copy)]
struct TraceIds {
    trace_id: TraceId,
    span_id: SpanId,
}

impl TraceIds {
    fn from_span_context(span_context: &SpanContext) -> Option<Self> {
        span_context.is_valid().then(|| Self {
            trace_id: span_context.trace_id(),
            span_id: span_context.span_id(),
        })
    }
}

#[derive(Default)]
struct TraceIdsLayer {
    dispatch: OnceLock<WeakDispatch>,
}

impl<S> Layer<S> for TraceIdsLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_register_dispatch(&self, dispatch: &Dispatch) {
        let _ = self.dispatch.set(dispatch.downgrade());
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx
            .event_span(event)
            .filter(|span| span.extensions().get::<TraceIds>().is_none())
        else {
            return;
        };

        let trace_ids = self
            .dispatch
            .get()
            .and_then(WeakDispatch::upgrade)
            .and_then(|dispatch| get_otel_context(&span.id(), &dispatch))
            .and_then(|cx| TraceIds::from_span_context(cx.span().span_context()));

        if let Some(trace_ids) = trace_ids {
            span.extensions_mut().replace(trace_ids);
        }
    }
}

struct TraceIdsFormat<F> {
    inner: F,
}

impl<S, N, F> FormatEvent<S, N> for TraceIdsFormat<F>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
    F: FormatEvent<S, N>,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        let Some(TraceIds { trace_id, span_id }) = ctx
            .parent_span()
            .and_then(|span| span.extensions().get::<TraceIds>().copied())
        else {
            return self.inner.format_event(ctx, writer, event);
        };

        let mut line = String::new();
        self.inner
            .format_event(ctx, Writer::new(&mut line), event)?;

        match line.strip_suffix("}\n") {
            Some(object) => writeln!(
                writer,
                r#"{object},"{TRACE_ID_FIELD}":"{trace_id}","{SPAN_ID_FIELD}":"{span_id}"}}"#
            ),
            None => writer.write_str(&line),
        }
    }
}

/// Renders JSON log lines carrying `trace_id`/`span_id` for events inside a valid OpenTelemetry span.
pub(crate) fn json_log_layer<S, W>(make_writer: W) -> impl Layer<S>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    TraceIdsLayer::default().and_then(
        fmt::layer()
            .fmt_fields(JsonFields::new())
            .event_format(TraceIdsFormat {
                inner: fmt::format()
                    .json()
                    .with_current_span(true)
                    .with_span_list(false)
                    .flatten_event(true),
            })
            .with_writer(make_writer),
    )
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        sync::{Arc, Mutex},
    };

    use opentelemetry::trace::TracerProvider;
    use opentelemetry_sdk::trace::{SdkTracer, SdkTracerProvider};
    use tracing_opentelemetry::OpenTelemetrySpanExt;
    use tracing_subscriber::layer::SubscriberExt;

    use super::*;

    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl Captured {
        fn output(&self) -> String {
            String::from_utf8(self.0.lock().expect("captured output lock").clone())
                .expect("captured output is utf-8")
        }
    }

    impl io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("captured output lock").write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn subscriber(tracer: SdkTracer, captured: Captured) -> impl Subscriber + Send + Sync {
        tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(tracer))
            .with(json_log_layer(move || captured.clone()))
    }

    #[test]
    fn event_inside_a_span_carries_its_trace_and_span_ids() {
        let provider = SdkTracerProvider::builder().build();
        let captured = Captured::default();

        let span_context = tracing::subscriber::with_default(
            subscriber(provider.tracer("test"), captured.clone()),
            || {
                let span = tracing::info_span!("work", request_id = "r-1");
                let _entered = span.enter();
                tracing::info!("hello");
                span.context().span().span_context().clone()
            },
        );

        let line = captured.output();
        let trace_id = span_context.trace_id();
        let span_id = span_context.span_id();
        assert!(span_context.is_valid());
        assert!(
            line.contains(&format!(r#""trace_id":"{trace_id}""#)),
            "{line}"
        );
        assert!(
            line.contains(&format!(r#""span_id":"{span_id}""#)),
            "{line}"
        );
        assert!(line.contains(r#""request_id":"r-1""#), "{line}");
        assert!(line.ends_with("}\n"), "{line}");
    }

    #[test]
    fn event_outside_any_span_has_no_trace_ids() {
        let provider = SdkTracerProvider::builder().build();
        let captured = Captured::default();

        tracing::subscriber::with_default(
            subscriber(provider.tracer("test"), captured.clone()),
            || {
                tracing::info!("hello");
            },
        );

        let line = captured.output();
        assert!(line.contains(r#""message":"hello""#), "{line}");
        assert!(!line.contains(r#""trace_id""#), "{line}");
        assert!(!line.contains(r#""span_id""#), "{line}");
    }
}
