//! Request-ID middleware: generation, propagation, sanitization, and
//! task-local access to the current request's ID.

use axum::{
    extract::Request,
    http::{HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};
use tower_http::request_id::{MakeRequestId, RequestId};
use uuid::Uuid;

/// The request-ID header used by the middleware in this module and by the
/// HTTP tracing layer.
pub const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

/// The maximum length in bytes of an acceptable inbound request ID.
pub const MAX_REQUEST_ID_LENGTH: usize = 64;

tokio::task_local! {
    static CURRENT: String;
}

/// The request ID assigned to the current request, when one has been scoped by
/// the [`scope`] middleware.
///
/// Returns [`None`] outside a request scope and for requests that arrived
/// without a valid [`REQUEST_ID_HEADER`].
#[must_use]
pub fn current() -> Option<String> {
    CURRENT.try_with(Clone::clone).ok()
}

/// Generates a `UUIDv7` request ID for every request.
#[derive(Clone, Copy, Default)]
pub struct MakeRequestUuidV7;

impl MakeRequestId for MakeRequestUuidV7 {
    fn make_request_id<B>(&mut self, _request: &Request<B>) -> Option<RequestId> {
        HeaderValue::from_str(&Uuid::now_v7().to_string())
            .ok()
            .map(RequestId::new)
    }
}

/// Removes an untrusted inbound [`REQUEST_ID_HEADER`] that is longer than
/// [`MAX_REQUEST_ID_LENGTH`] bytes or contains bytes outside
/// `[A-Za-z0-9_-]`, so that
/// `SetRequestIdLayer` (which never overrides an existing header) mints a
/// fresh `UUIDv7` instead of echoing attacker-controlled content back to the
/// caller.
pub async fn sanitize(mut req: Request, next: Next) -> Response {
    let untrusted = req
        .headers()
        .get(&REQUEST_ID_HEADER)
        .is_some_and(|value| !acceptable(value));
    if untrusted {
        req.headers_mut().remove(&REQUEST_ID_HEADER);
    }
    next.run(req).await
}

/// Makes the inbound [`REQUEST_ID_HEADER`] value available to the rest of the
/// request (handlers, fallbacks, error responses) via [`current`]. Requests
/// without a valid UTF-8 header run unscoped.
pub async fn scope(req: Request, next: Next) -> Response {
    match req
        .headers()
        .get(&REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
    {
        Some(id) => CURRENT.scope(id.to_owned(), next.run(req)).await,
        None => next.run(req).await,
    }
}

fn acceptable(value: &HeaderValue) -> bool {
    value.len() <= MAX_REQUEST_ID_LENGTH
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-' || *byte == b'_')
}

#[cfg(test)]
mod tests {
    use axum::{Json, Router, body::Body, http::Request, middleware, routing::get};
    use serde_json::Value;
    use tower_http::request_id::MakeRequestId;

    use super::{REQUEST_ID_HEADER, current, sanitize, scope};
    use crate::test_support;

    async fn echo_current_id() -> Json<Value> {
        Json(serde_json::json!({ "id": current() }))
    }

    async fn echo_request_id(req: Request<Body>) -> Json<Value> {
        let id = req.headers().get(&REQUEST_ID_HEADER).map(|value| {
            String::from_utf8(value.as_bytes().to_vec())
                .unwrap_or_else(|_| "<invalid utf-8>".to_owned())
        });
        Json(serde_json::json!({ "id": id }))
    }

    #[tokio::test]
    async fn current_is_none_outside_a_scope() {
        assert!(current().is_none());
    }

    #[tokio::test]
    async fn current_is_none_for_unscoped_requests() {
        let router = Router::new().route("/id", get(echo_current_id));
        let res = test_support::get(router, "/id").await;
        assert_eq!(res.body.expect("the body is JSON")["id"], Value::Null);
    }

    #[tokio::test]
    async fn current_is_some_inside_a_scope() {
        let router = Router::new()
            .route("/id", get(echo_current_id))
            .layer(middleware::from_fn(scope));
        let res = test_support::get_with_headers(
            router,
            "/id",
            &[(REQUEST_ID_HEADER.as_str(), "abc-123")],
        )
        .await;
        assert_eq!(res.body.expect("the body is JSON")["id"], "abc-123");
    }

    #[tokio::test]
    async fn requests_without_a_header_run_outside_the_task_local() {
        let router = Router::new()
            .route("/id", get(echo_current_id))
            .layer(middleware::from_fn(scope));
        let res = test_support::get(router, "/id").await;
        assert_eq!(res.body.expect("the body is JSON")["id"], Value::Null);
    }

    #[tokio::test]
    async fn sanitize_passes_valid_ids_through() {
        let router = Router::new()
            .route("/id", get(echo_request_id))
            .layer(middleware::from_fn(sanitize));
        let res = test_support::get_with_headers(
            router,
            "/id",
            &[(REQUEST_ID_HEADER.as_str(), "gateway-id_XYZ-123")],
        )
        .await;
        assert_eq!(
            res.body.expect("the body is JSON")["id"],
            "gateway-id_XYZ-123"
        );
    }

    #[tokio::test]
    async fn sanitize_drops_overlong_ids() {
        let router = Router::new()
            .route("/id", get(echo_request_id))
            .layer(middleware::from_fn(sanitize));
        let res = test_support::get_with_headers(
            router,
            "/id",
            &[(REQUEST_ID_HEADER.as_str(), "a".repeat(70).as_str())],
        )
        .await;
        assert_eq!(res.body.expect("the body is JSON")["id"], Value::Null);
    }

    #[tokio::test]
    async fn sanitize_drops_ids_with_unacceptable_bytes() {
        let router = Router::new()
            .route("/id", get(echo_request_id))
            .layer(middleware::from_fn(sanitize));
        let res = test_support::get_with_headers(
            router,
            "/id",
            &[(REQUEST_ID_HEADER.as_str(), "id with spaces")],
        )
        .await;
        assert_eq!(res.body.expect("the body is JSON")["id"], Value::Null);
    }

    #[tokio::test]
    async fn sanitize_leaves_absent_headers_absent() {
        let router = Router::new()
            .route("/id", get(echo_request_id))
            .layer(middleware::from_fn(sanitize));
        let res = test_support::get(router, "/id").await;
        assert_eq!(res.body.expect("the body is JSON")["id"], Value::Null);
    }

    #[tokio::test]
    async fn make_request_uuid_v7_makes_a_uuidv7_header() {
        let mut make = super::MakeRequestUuidV7;
        let req = Request::builder()
            .method("GET")
            .uri("/id")
            .body(Body::empty())
            .unwrap();
        let id = make.make_request_id(&req).unwrap();
        let value = id.header_value().to_str().unwrap();
        let uuid = value.parse::<uuid::Uuid>().unwrap();
        assert_eq!(uuid.get_version_num(), 7);
        assert_eq!(value.len(), 36);
    }

    #[tokio::test]
    async fn sanitize_accepts_exactly_64_byte_ids() {
        let max_id = "a".repeat(64);
        let router = Router::new()
            .route("/id", get(echo_request_id))
            .layer(middleware::from_fn(sanitize));
        let res = test_support::get_with_headers(
            router,
            "/id",
            &[(REQUEST_ID_HEADER.as_str(), max_id.as_str())],
        )
        .await;
        assert_eq!(res.body.expect("the body is JSON")["id"], max_id);
    }
}
