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
pub(crate) fn with_request_timeout(router: Router, timeout: Duration) -> Router {
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

    use axum::{Json, Router, http::StatusCode, middleware, routing::get};
    use serde_json_assert::assert_json_include;

    use super::with_request_timeout;
    use crate::{request_id, test_support};

    fn router() -> Router {
        let inner = Router::new()
            .route("/slow", get(std::future::pending::<&'static str>))
            .route("/fast", get(|| async { Json("ok") }));
        with_request_timeout(inner, Duration::from_millis(10))
            .layer(middleware::from_fn(request_id::scope))
    }

    #[tokio::test]
    async fn slow_requests_get_a_json_503() {
        let res = test_support::get_with_headers(
            router(),
            "/slow",
            &[(request_id::REQUEST_ID_HEADER.as_str(), "req-1")],
        )
        .await;
        assert_eq!(res.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(res.content_type.starts_with("application/json"));
        let body = res.body.expect("the 503 body is JSON");
        assert_json_include!(
            actual: body,
            expected: serde_json::json!({ "error": "request timed out", "requestId": "req-1" })
        );
    }

    #[tokio::test]
    async fn fast_requests_are_untouched() {
        let res = test_support::get(router(), "/fast").await;
        assert_eq!(res.status, StatusCode::OK);
        assert_eq!(res.body, Some(serde_json::json!("ok")));
    }
}
