use std::time::Duration;

use axum::Router;

use crate::{
    body_limit::with_body_limit, catch_panic::with_catch_panic, timeout::with_request_timeout,
};

/// Limits applied by the `with_guardrails` layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GuardrailsConfig {
    /// Time a request may take to produce a response before it is answered
    /// with `503 Service Unavailable`.
    pub request_timeout: Duration,
    /// Maximum request body size in bytes; larger bodies are rejected with
    /// `413 Payload Too Large`.
    pub body_limit_bytes: usize,
}

/// Applies the request guardrails to `router`, from the outside in:
/// `with_catch_panic`, `with_request_timeout`, and `with_body_limit` wrap
/// every route and fallback `router` has at call time.
pub(crate) fn with_guardrails(router: Router, config: GuardrailsConfig) -> Router {
    with_catch_panic(with_request_timeout(
        with_body_limit(router, config.body_limit_bytes),
        config.request_timeout,
    ))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::{
        Router,
        body::{Body, Bytes},
        http::StatusCode,
        middleware,
        routing::{get, post},
    };
    use serde_json_assert::assert_json_include;

    use super::{GuardrailsConfig, with_guardrails};
    use crate::{request_id, test_support};

    async fn panics() -> &'static str {
        panic!("secret detail")
    }

    fn router() -> Router {
        let inner = Router::new()
            .route("/panic", get(panics))
            .route("/slow", get(std::future::pending::<&'static str>))
            .route(
                "/upload",
                post(|body: Bytes| async move { body.len().to_string() }),
            );
        with_guardrails(
            inner,
            GuardrailsConfig {
                request_timeout: Duration::from_millis(50),
                body_limit_bytes: 16,
            },
        )
        .layer(middleware::from_fn(request_id::scope))
    }

    #[tokio::test]
    async fn panics_are_answered_with_a_json_500() {
        let res = test_support::get_with_headers(
            router(),
            "/panic",
            &[(request_id::REQUEST_ID_HEADER.as_str(), "req-1")],
        )
        .await;
        assert_eq!(res.status, StatusCode::INTERNAL_SERVER_ERROR);
        let body = res.body.expect("the 500 body is JSON");
        assert_json_include!(
            actual: body,
            expected: serde_json::json!({ "error": "internal server error", "requestId": "req-1" })
        );
    }

    #[tokio::test]
    async fn slow_requests_are_answered_with_a_json_503() {
        let res = test_support::get_with_headers(
            router(),
            "/slow",
            &[(request_id::REQUEST_ID_HEADER.as_str(), "req-1")],
        )
        .await;
        assert_eq!(res.status, StatusCode::SERVICE_UNAVAILABLE);
        let body = res.body.expect("the 503 body is JSON");
        assert_json_include!(
            actual: body,
            expected: serde_json::json!({ "error": "request timed out", "requestId": "req-1" })
        );
    }

    #[tokio::test]
    async fn oversized_bodies_are_rejected_with_413() {
        let res =
            test_support::post_body(router(), "/upload", None, Body::from(b"a".repeat(64))).await;
        assert_eq!(res.status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn small_bodies_pass_through() {
        let res = test_support::post_body(
            router(),
            "/upload",
            None,
            Body::from(b"12345678".as_slice()),
        )
        .await;
        assert_eq!(res.status, StatusCode::OK);
        assert_eq!(res.body, Some(serde_json::json!(8)));
    }
}
