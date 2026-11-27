use std::any::Any;

use axum::{
    Router,
    body::Body,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use tower_http::catch_panic::{CatchPanicLayer, ResponseForPanic};

use crate::api_error::ApiError;

const PANIC_RESPONSE_MESSAGE: &str = "internal server error";

#[derive(Clone, Copy)]
struct JsonPanicResponse;

impl ResponseForPanic for JsonPanicResponse {
    type ResponseBody = Body;

    fn response_for_panic(&mut self, panic: Box<dyn Any + Send + 'static>) -> Response<Body> {
        let message = panic
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("non-string panic payload");
        tracing::error!(panic.message = message, "request handler panicked");
        ApiError::message(StatusCode::INTERNAL_SERVER_ERROR, PANIC_RESPONSE_MESSAGE).into_response()
    }
}

/// Wraps `router` with [`CatchPanicLayer`]: a panic in any route or fallback
/// `router` has at call time is caught and answered with `500 Internal Server
/// Error` and the standard JSON error body (`{"error": "internal server
/// error"}`, plus `requestId` under a
/// [`crate::request_id::scope`] request-id scope). The panic message is logged
/// at `ERROR` level and never included in the response.
pub(crate) fn with_catch_panic(router: Router) -> Router {
    router.layer(CatchPanicLayer::custom(JsonPanicResponse))
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode, header},
        middleware,
        response::Response,
        routing::get,
    };
    use http_body_util::BodyExt;
    use serde_json_assert::assert_json_include;
    use tower::ServiceExt;

    use super::with_catch_panic;
    use crate::request_id;

    async fn panics() -> &'static str {
        panic!("secret detail")
    }

    fn router() -> Router {
        with_catch_panic(
            Router::new()
                .route("/panic", get(panics))
                .route("/ok", get(|| async { "ok" })),
        )
        .layer(middleware::from_fn(request_id::scope))
    }

    fn request(uri: &str, id: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri(uri);
        if let Some(id) = id {
            builder = builder.header(&request_id::REQUEST_ID_HEADER, id);
        }
        builder.body(Body::empty()).unwrap()
    }

    async fn body(res: Response) -> Vec<u8> {
        res.into_body().collect().await.unwrap().to_bytes().to_vec()
    }

    #[tokio::test]
    async fn panics_become_a_json_500() {
        let res = router()
            .oneshot(request("/panic", Some("req-1")))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            res.headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("application/json"))
        );
        let body: serde_json::Value = serde_json::from_slice(&body(res).await).unwrap();
        assert_json_include!(
            actual: body,
            expected: serde_json::json!({ "error": "internal server error", "requestId": "req-1" })
        );
    }

    #[tokio::test]
    async fn panic_messages_are_not_leaked() {
        let res = router()
            .oneshot(request("/panic", Some("req-1")))
            .await
            .unwrap();
        let body = String::from_utf8(body(res).await).unwrap();
        assert!(!body.contains("secret detail"));
    }

    #[tokio::test]
    async fn non_panicking_routes_are_untouched() {
        let res = router()
            .oneshot(request("/ok", Some("req-1")))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(body(res).await, b"ok".to_vec());
    }
}
