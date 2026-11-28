//! JSON fallback handlers and a router helper for axum's empty 404 and 405
//! responses.

use axum::{Router, http::StatusCode};

use crate::api_error::ApiError;

/// The `404 Not Found` response: the standard JSON error body with
/// `"error": "route not found"`.
pub async fn not_found() -> ApiError {
    ApiError::message(StatusCode::NOT_FOUND, "route not found")
}

/// The `405 Method Not Allowed` response: the standard JSON error body with
/// `"error": "method not allowed"`.
pub async fn method_not_allowed() -> ApiError {
    ApiError::message(StatusCode::METHOD_NOT_ALLOWED, "method not allowed")
}

/// Replaces axum's empty 404 and 405 responses with the standard JSON error body.
pub fn with_json_fallbacks<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
}

#[cfg(test)]
mod tests {
    use axum::{Router, http::StatusCode, routing::get};
    use serde_json_assert::assert_json_include;

    use super::with_json_fallbacks;
    use crate::test_support;

    fn router() -> Router {
        with_json_fallbacks(Router::new().route("/ping", get(ping).post(ping)))
    }

    async fn ping() -> StatusCode {
        StatusCode::OK
    }

    #[tokio::test]
    async fn unknown_route_is_json_404() {
        let res = test_support::get(router(), "/nope").await;
        assert_eq!(res.status, StatusCode::NOT_FOUND);
        assert!(res.content_type.starts_with("application/json"));
        let body = res.body.expect("the 404 body is JSON");
        assert_json_include!(
            actual: body,
            expected: serde_json::json!({ "error": "route not found" })
        );
    }

    #[tokio::test]
    async fn wrong_method_is_json_405() {
        let res = test_support::delete(router(), "/ping").await;
        assert_eq!(res.status, StatusCode::METHOD_NOT_ALLOWED);
        assert!(res.content_type.starts_with("application/json"));
        let allow = res.header("allow").unwrap_or_default();
        let body = res.body.expect("the 405 body is JSON");
        assert_json_include!(
            actual: body,
            expected: serde_json::json!({ "error": "method not allowed" })
        );
        assert!(allow.contains("GET"), "Allow: {allow}");
        assert!(allow.contains("POST"), "Allow: {allow}");
    }

    #[tokio::test]
    async fn known_route_is_untouched() {
        let res = test_support::get(router(), "/ping").await;
        assert_eq!(res.status, StatusCode::OK);
    }
}
