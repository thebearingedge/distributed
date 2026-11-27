use std::time::Duration;

use axum::{
    Router,
    extract::MatchedPath,
    http::{Request, Response},
    middleware,
};
use tower::ServiceBuilder;
use tower_http::{
    request_id::{PropagateRequestIdLayer, SetRequestIdLayer},
    trace::{DefaultOnFailure, MakeSpan, OnResponse, TraceLayer},
};
use tracing::{Level, Span};

use crate::request_id::{MakeRequestUuidV7, REQUEST_ID_HEADER, sanitize, scope};

/// `MakeSpan` implementation following the OpenTelemetry HTTP server
/// semantic conventions: `otel.name` is `"{method} {route}"` (method alone
/// when no route matched), `otel.kind` is `"server"`, and the standard
/// fields (`http.request.method`, `http.route`, `url.path`,
/// `http.response.status_code`) are recorded. The request ID from
/// [`crate::request_id::REQUEST_ID_HEADER`] is recorded as `request_id`
/// (empty when absent), and the incoming parent context is linked from the
/// request headers via `telemetry::propagation::link_parent`.
#[derive(Clone, Copy)]
pub(crate) struct HttpMakeSpan;

impl<B> MakeSpan<B> for HttpMakeSpan {
    fn make_span(&mut self, req: &Request<B>) -> Span {
        let method = req.method().as_str();
        let route = req
            .extensions()
            .get::<MatchedPath>()
            .map(MatchedPath::as_str);
        let name = match route {
            Some(route) => format!("{method} {route}"),
            None => method.to_owned(),
        };
        let request_id = req
            .headers()
            .get(&REQUEST_ID_HEADER)
            .and_then(|value| value.to_str().ok());
        let span = tracing::info_span!(
            "http.request",
            otel.name = %name,
            otel.kind = "server",
            http.request.method = %method,
            http.route = route,
            request_id = request_id,
            url.path = %req.uri().path(),
            http.response.status_code = tracing::field::Empty,
        );

        telemetry::propagation::link_parent(&span, req.headers());
        span
    }
}

/// `OnResponse` implementation that records the response status into the
/// span's `http.response.status_code` field.
#[derive(Clone, Copy)]
pub(crate) struct HttpOnResponse;

impl<B> OnResponse<B> for HttpOnResponse {
    fn on_response(self, response: &Response<B>, _latency: Duration, span: &Span) {
        span.record("http.response.status_code", response.status().as_u16());
    }
}

/// Wraps `router` with the standard HTTP middleware chain, applied outermost
/// first: an untrusted inbound [`crate::request_id::REQUEST_ID_HEADER`] is
/// sanitized ([`crate::request_id::sanitize`]), a missing header is assigned a
/// `UUIDv7` request ID ([`crate::request_id::MakeRequestUuidV7`]), the request
/// ID is propagated onto the response, and it is made available to handlers
/// and error responses via [`crate::request_id::current`]
/// ([`crate::request_id::scope`]). Finally a [`TraceLayer`] is applied using
/// the OTel-convention span from [`HttpMakeSpan`], recording the status code
/// on response via [`HttpOnResponse`] and logging failures at ERROR level.
/// Requests and responses are not logged.
pub(crate) fn with_http_tracing(router: Router) -> Router {
    router.layer(
        ServiceBuilder::new()
            .layer(middleware::from_fn(sanitize))
            .layer(SetRequestIdLayer::new(REQUEST_ID_HEADER, MakeRequestUuidV7))
            .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER))
            .layer(middleware::from_fn(scope))
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(HttpMakeSpan)
                    .on_request(())
                    .on_response(HttpOnResponse)
                    .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
            ),
    )
}

#[cfg(test)]
mod tests {
    use axum::{Router, routing::get};

    use super::with_http_tracing;
    use crate::{fallback::with_json_fallbacks, request_id::REQUEST_ID_HEADER, test_support};

    fn router() -> Router {
        with_http_tracing(with_json_fallbacks(
            Router::new().route("/things", get(|| async { "ok" })),
        ))
    }

    #[tokio::test]
    async fn requests_without_a_header_get_a_fresh_uuid() {
        let res = test_support::get(router(), "/things").await;
        let id = res
            .header(REQUEST_ID_HEADER.as_str())
            .expect("x-request-id");
        assert!(id.parse::<uuid::Uuid>().is_ok(), "not a UUID: {id}");
    }

    #[tokio::test]
    async fn valid_inbound_ids_are_echoed_unchanged() {
        let inbound = "gateway-id_XYZ-123";
        let res = test_support::get_with_headers(
            router(),
            "/things",
            &[(REQUEST_ID_HEADER.as_str(), inbound)],
        )
        .await;
        assert_eq!(
            res.header(REQUEST_ID_HEADER.as_str()).as_deref(),
            Some(inbound)
        );
    }

    #[tokio::test]
    async fn overlong_inbound_ids_are_replaced_with_a_fresh_uuid() {
        let res = test_support::get_with_headers(
            router(),
            "/things",
            &[(REQUEST_ID_HEADER.as_str(), "a".repeat(70).as_str())],
        )
        .await;
        let id = res
            .header(REQUEST_ID_HEADER.as_str())
            .expect("x-request-id");
        assert!(id.parse::<uuid::Uuid>().is_ok(), "not a UUID: {id}");
    }

    #[tokio::test]
    async fn inbound_ids_with_unacceptable_bytes_are_replaced_with_a_fresh_uuid() {
        let res = test_support::get_with_headers(
            router(),
            "/things",
            &[(REQUEST_ID_HEADER.as_str(), "id with spaces")],
        )
        .await;
        let id = res
            .header(REQUEST_ID_HEADER.as_str())
            .expect("x-request-id");
        assert!(id.parse::<uuid::Uuid>().is_ok(), "not a UUID: {id}");
    }

    #[tokio::test]
    async fn not_found_bodies_carry_the_response_request_id() {
        let res = test_support::get(router(), "/nope").await;
        let id = res
            .header(REQUEST_ID_HEADER.as_str())
            .expect("x-request-id");
        let body = res.body.expect("the 404 body is JSON");
        assert_eq!(body["requestId"], id);
    }
}
