//! Request guardrails composite: [`crate::guardrails::with_guardrails`].

use std::time::Duration;

use axum::Router;

use crate::{
    body_limit::with_body_limit,
    catch_panic::with_catch_panic,
    config::{REQUEST_BODY_LIMIT_DEFAULT_BYTES, REQUEST_TIMEOUT_DEFAULT_SECONDS},
    timeout::with_request_timeout,
};

/// Limits applied by [`crate::guardrails::with_guardrails`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuardrailsConfig {
    /// Time a request may take to produce a response before it is answered
    /// with `503 Service Unavailable`.
    pub request_timeout: Duration,
    /// Maximum request body size in bytes; larger bodies are rejected with
    /// `413 Payload Too Large`.
    pub body_limit_bytes: usize,
}

/// Defaults match the [`crate::config::ApiConfig`] defaults.
impl Default for GuardrailsConfig {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(REQUEST_TIMEOUT_DEFAULT_SECONDS),
            body_limit_bytes: REQUEST_BODY_LIMIT_DEFAULT_BYTES,
        }
    }
}

/// Applies the request guardrails to `router` in a fixed order: from the
/// outside in, [`crate::catch_panic::with_catch_panic`],
/// [`crate::timeout::with_request_timeout`], and
/// [`crate::body_limit::with_body_limit`] wrap every route and fallback
/// `router` has at call time.
///
/// Apply it to the API router *before* passing the router to
/// [`crate::middleware::with_http_tracing`], so error bodies carry the request
/// id and the trace span records the status. Merge health probes afterwards so
/// they are not affected.
pub fn with_guardrails(router: Router, config: GuardrailsConfig) -> Router {
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
        http::{Request, StatusCode},
        middleware,
        response::Response,
        routing::{get, post},
    };
    use http_body_util::BodyExt;
    use serde_json::Value;
    use serde_json_assert::assert_json_include;
    use tower::ServiceExt;

    use super::{GuardrailsConfig, with_guardrails};
    use crate::request_id;

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

    fn get_request(uri: &'static str, id: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().method("GET").uri(uri);
        if let Some(id) = id {
            builder = builder.header(&request_id::REQUEST_ID_HEADER, id);
        }
        builder.body(Body::empty()).unwrap()
    }

    fn post_request(uri: &'static str, body: Body) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .body(body)
            .unwrap()
    }

    async fn body_json(res: Response) -> Value {
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn panics_are_answered_with_a_json_500() {
        let res = router()
            .oneshot(get_request("/panic", Some("req-1")))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_json(res).await;
        assert_json_include!(
            actual: body,
            expected: serde_json::json!({ "error": "internal server error", "requestId": "req-1" })
        );
    }

    #[tokio::test]
    async fn slow_requests_are_answered_with_a_json_503() {
        let res = router()
            .oneshot(get_request("/slow", Some("req-1")))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = body_json(res).await;
        assert_json_include!(
            actual: body,
            expected: serde_json::json!({ "error": "request timed out", "requestId": "req-1" })
        );
    }

    #[tokio::test]
    async fn oversized_bodies_are_rejected_with_413() {
        let res = router()
            .oneshot(post_request("/upload", Body::from(b"a".repeat(64))))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn small_bodies_pass_through() {
        let res = router()
            .oneshot(post_request("/upload", Body::from(b"12345678".as_slice())))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(bytes.as_ref(), b"8");
    }
}
