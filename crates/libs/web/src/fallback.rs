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
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode, header},
        response::Response,
        routing::get,
    };
    use http_body_util::BodyExt;
    use serde_json::Value;
    use serde_json_assert::assert_json_include;
    use tower::ServiceExt;

    use super::with_json_fallbacks;

    fn router() -> Router {
        with_json_fallbacks(Router::new().route("/ping", get(ping).post(ping)))
    }

    async fn ping() -> StatusCode {
        StatusCode::OK
    }

    fn request(method: &'static str, uri: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap()
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
    async fn unknown_route_is_json_404() {
        let res = router().oneshot(request("GET", "/nope")).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert!(content_type(&res).starts_with("application/json"));
        let body = body_json(res).await;
        assert_json_include!(
            actual: body,
            expected: serde_json::json!({ "error": "route not found" })
        );
    }

    #[tokio::test]
    async fn wrong_method_is_json_405() {
        let res = router().oneshot(request("DELETE", "/ping")).await.unwrap();
        assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert!(content_type(&res).starts_with("application/json"));
        let allow = res
            .headers()
            .get(header::ALLOW)
            .map_or_default(|v| v.to_str().unwrap().to_owned());
        let body = body_json(res).await;
        assert_json_include!(
            actual: body,
            expected: serde_json::json!({ "error": "method not allowed" })
        );
        assert!(allow.contains("GET"), "Allow: {allow}");
        assert!(allow.contains("POST"), "Allow: {allow}");
    }

    #[tokio::test]
    async fn known_route_is_untouched() {
        let res = router().oneshot(request("GET", "/ping")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}
