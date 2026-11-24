//! Request timeout middleware: [`crate::timeout::with_request_timeout`].

use std::time::Duration;

use axum::{
    Router,
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
};

use crate::api_error::ApiError;

const REQUEST_TIMEOUT_MESSAGE: &str = "request timed out";

/// Applies a request timeout to `router`.
///
/// Every route and fallback `router` has at call time is wrapped so that a
/// request which does not produce a response within `timeout` is answered
/// with `503 Service Unavailable` and the standard JSON error body
/// (`{"error": "request timed out"}`, plus `requestId` under a request-id
/// scope). The unfinished handler future is dropped when the timeout fires.
///
/// Apply this layer inside [`crate::middleware::with_http_tracing`] so the
/// body carries the request id.
pub fn with_request_timeout(router: Router, timeout: Duration) -> Router {
    router.layer(middleware::from_fn_with_state(timeout, enforce_timeout))
}

async fn enforce_timeout(State(timeout): State<Duration>, req: Request, next: Next) -> Response {
    tokio::time::timeout(timeout, next.run(req))
        .await
        .unwrap_or_else(|_elapsed| {
            ApiError::message(StatusCode::SERVICE_UNAVAILABLE, REQUEST_TIMEOUT_MESSAGE)
                .into_response()
        })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode, header},
        middleware,
        response::Response,
        routing::get,
    };
    use http_body_util::BodyExt;
    use serde_json::Value;
    use serde_json_assert::assert_json_include;
    use tower::ServiceExt;

    use super::with_request_timeout;
    use crate::request_id;

    fn router() -> Router {
        let inner = Router::new()
            .route("/slow", get(std::future::pending::<&'static str>))
            .route("/fast", get(|| async { "ok" }));
        with_request_timeout(inner, Duration::from_millis(10))
            .layer(middleware::from_fn(request_id::scope))
    }

    fn request(uri: &'static str, id: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().method("GET").uri(uri);
        if let Some(id) = id {
            builder = builder.header(&request_id::REQUEST_ID_HEADER, id);
        }
        builder.body(Body::empty()).unwrap()
    }

    async fn body_json(res: Response) -> Value {
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn content_type(res: &Response) -> String {
        res.headers()
            .get(header::CONTENT_TYPE)
            .map_or_default(|v| v.to_str().unwrap().to_owned())
    }

    #[tokio::test]
    async fn slow_requests_get_a_json_503() {
        let res = router()
            .oneshot(request("/slow", Some("req-1")))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(content_type(&res).starts_with("application/json"));
        let body = body_json(res).await;
        assert_json_include!(
            actual: body,
            expected: serde_json::json!({ "error": "request timed out", "requestId": "req-1" })
        );
    }

    #[tokio::test]
    async fn fast_requests_are_untouched() {
        let res = router().oneshot(request("/fast", None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body.as_ref(), b"ok");
    }
}
