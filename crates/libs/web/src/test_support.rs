//! Helpers for exercising axum routers in tests by sending one request at a
//! time through `tower`'s oneshot service adapter.
//!
//! Compiled only for tests or when the crate's `test-support` feature is
//! enabled; intended for unit and integration tests of HTTP layers built on
//! axum.

use axum::{
    Router,
    body::Body,
    http::{HeaderMap, HeaderName, Method, Request, StatusCode, header},
    response::Response,
};
use http_body_util::BodyExt;
use serde::Serialize;
use serde_json::Value;
use tower::ServiceExt;

/// The interesting parts of an HTTP response: the status code, the
/// `Content-Type` header, the response headers, and the parsed JSON body
/// (`None` when the body is not valid JSON).
pub struct ResponseParts {
    /// The response status code.
    pub status: StatusCode,
    /// The raw `Content-Type` header value.
    pub content_type: String,
    /// The parsed JSON body, or `None` when the body is not valid JSON.
    pub body: Option<Value>,
    headers: HeaderMap,
}

impl ResponseParts {
    /// Returns the value of the response header `name`, or `None` when the
    /// header is absent, `name` is not a valid header name, or the value is
    /// not valid UTF-8.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<String> {
        let name = HeaderName::from_bytes(name.as_bytes()).ok()?;
        self.headers.get(name)?.to_str().ok().map(str::to_owned)
    }
}

/// Collects the status, `Content-Type` header, and parsed JSON body from a
/// response.
///
/// A missing `Content-Type` header maps to `content_type: ""` and
/// `body: None`, so responses that carry no content type — a `204 No
/// Content`, a bare `200 OK` — are collectable too.
///
/// # Panics
///
/// Panics when a `Content-Type` header is present but not UTF-8, or the
/// response body cannot be collected.
pub async fn get_response_parts(res: Response<Body>) -> ResponseParts {
    let headers = res.headers().clone();
    let status = res.status();
    let content_type = res
        .headers()
        .get(header::CONTENT_TYPE)
        .map_or_default(|value| {
            value
                .to_str()
                .expect("content-type header should be UTF-8")
                .to_owned()
        });
    let bytes = res
        .into_body()
        .collect()
        .await
        .expect("response body should be collectable")
        .to_bytes();
    let body = serde_json::from_slice(&bytes).ok();
    ResponseParts {
        status,
        content_type,
        body,
        headers,
    }
}

/// Sends a built `request` through `router` and collects the interesting
/// parts of the response.
///
/// # Panics
///
/// Panics when the router fails to handle the request.
async fn send(router: Router, request: Request<Body>) -> ResponseParts {
    get_response_parts(
        router
            .oneshot(request)
            .await
            .expect("request should be handled"),
    )
    .await
}

/// Sends a `GET` request and collects the interesting parts of the response.
///
/// # Panics
///
/// Panics when the request cannot be built or the router fails to handle it.
pub async fn get(router: Router, uri: &str) -> ResponseParts {
    let request = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("GET request should be buildable");
    send(router, request).await
}

/// Sends a `GET` request with extra request headers and collects the
/// interesting parts of the response.
///
/// # Panics
///
/// Panics when a header name or value is invalid, or the request cannot be
/// built or the router fails to handle it.
pub async fn get_with_headers(
    router: Router,
    uri: &str,
    headers: &[(&str, &str)],
) -> ResponseParts {
    let mut builder = Request::builder().uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = builder
        .body(Body::empty())
        .expect("GET request should be buildable");
    send(router, request).await
}

/// Sends a `POST` request with a JSON-encoded `body` and the
/// `application/json` content type, then collects the interesting parts of the
/// response.
///
/// # Panics
///
/// Panics when `body` cannot be serialized to JSON, or the request cannot be
/// built or the router fails to handle it.
pub async fn post_json(router: Router, uri: &str, body: &impl Serialize) -> ResponseParts {
    let bytes = serde_json::to_vec(body).expect("JSON request body should serialize");
    post_body(router, uri, Some("application/json"), bytes.into()).await
}

/// Sends a `POST` request with extra request headers, a JSON-encoded `body`,
/// and the `application/json` content type, then collects the interesting
/// parts of the response.
///
/// # Panics
///
/// Panics when `body` cannot be serialized to JSON, a header name or value is
/// invalid, or the request cannot be built or the router fails to handle it.
pub async fn post_json_with_headers(
    router: Router,
    uri: &str,
    headers: &[(&str, &str)],
    body: &impl Serialize,
) -> ResponseParts {
    let bytes = serde_json::to_vec(body).expect("JSON request body should serialize");
    post_body_with_headers(router, uri, headers, Some("application/json"), bytes.into()).await
}

/// Sends a `PUT` request with a JSON-encoded `body` and the
/// `application/json` content type, then collects the interesting parts of
/// the response.
///
/// # Panics
///
/// Panics when `body` cannot be serialized to JSON, or the request cannot be
/// built or the router fails to handle it.
pub async fn put_json(router: Router, uri: &str, body: &impl Serialize) -> ResponseParts {
    let bytes = serde_json::to_vec(body).expect("JSON request body should serialize");
    let request = Request::builder()
        .method(Method::PUT)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(bytes))
        .expect("PUT request should be buildable");
    send(router, request).await
}

/// Sends a `POST` request with a raw `body`, optionally setting the
/// `Content-Type` header, then collects the interesting parts of the response.
///
/// This covers the request shapes [`post_json`] cannot express: a missing
/// content type and a body that is not valid JSON.
///
/// # Panics
///
/// Panics when the request cannot be built or the router fails to handle it.
pub async fn post_body(
    router: Router,
    uri: &str,
    content_type: Option<&str>,
    body: Body,
) -> ResponseParts {
    post_body_with_headers(router, uri, &[], content_type, body).await
}

async fn post_body_with_headers(
    router: Router,
    uri: &str,
    headers: &[(&str, &str)],
    content_type: Option<&str>,
    body: Body,
) -> ResponseParts {
    let mut builder = Request::builder().method(Method::POST).uri(uri);
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = builder
        .body(body)
        .expect("POST request should be buildable");
    send(router, request).await
}

/// Sends a `DELETE` request and collects the interesting parts of the
/// response.
///
/// A `204 No Content` response carries no `Content-Type` header; a missing
/// header maps to `content_type: ""` and `body: None`.
///
/// # Panics
///
/// Panics when the request cannot be built or the router fails to handle
/// it, or when a `Content-Type` header is present but not UTF-8.
pub async fn delete(router: Router, uri: &str) -> ResponseParts {
    let request = Request::builder()
        .method(Method::DELETE)
        .uri(uri)
        .body(Body::empty())
        .expect("DELETE request should be buildable");
    send(router, request).await
}

/// Extracts the `next`-page cursor from a list response, if it has one.
#[must_use]
pub fn next_cursor(parts: &ResponseParts) -> Option<String> {
    parts
        .body
        .as_ref()
        .and_then(|body| body["next"].as_str())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{
        ResponseParts, delete, get, get_response_parts, get_with_headers, next_cursor, post_body,
        post_json, post_json_with_headers, put_json,
    };
    use axum::{
        Json, Router,
        body::{Body, Bytes},
        http::{HeaderMap, StatusCode, header},
        routing::{delete as delete_route, get as get_route, post as post_route},
    };
    use serde_json::{Value, json};
    use serde_json_assert::assert_json_include;

    #[allow(clippy::unused_async)]
    async fn echo(headers: HeaderMap, body: Bytes) -> Json<Value> {
        let content_type = headers
            .get(header::CONTENT_TYPE)
            .map(|value| value.to_str().expect("content-type header should be UTF-8"));
        let parsed = serde_json::from_slice::<Value>(&body).ok();
        Json(json!({
            "content_type": content_type,
            "body": parsed,
        }))
    }

    fn router() -> Router {
        Router::new()
            .route(
                "/ping",
                get_route(|| async { Json(json!({ "next": "cursor-1" })) }),
            )
            .route("/text", get_route(|| async { "ok" }))
            .route("/echo", post_route(echo).put(echo))
            .route("/gone", delete_route(|| async { StatusCode::NO_CONTENT }))
    }

    #[tokio::test]
    async fn get_collects_status_content_type_and_json_body() {
        let parts = get(router(), "/ping").await;
        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(parts.content_type, "application/json");
        assert_eq!(next_cursor(&parts), Some("cursor-1".to_owned()));
        let _body = parts.body.expect("ping response should be json");
    }

    #[tokio::test]
    async fn get_parses_a_non_json_body_as_none() {
        let parts = get(router(), "/text").await;
        assert_eq!(parts.status, StatusCode::OK);
        assert!(parts.content_type.starts_with("text/plain"));
        assert_eq!(parts.body, None);
        assert_eq!(next_cursor(&parts), None);
    }

    #[tokio::test]
    async fn get_tolerates_a_missing_content_type() {
        let router: Router = Router::new().route("/empty", get_route(|| async { StatusCode::OK }));
        let parts = get(router, "/empty").await;
        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(parts.content_type, "");
        assert_eq!(parts.body, None);
    }

    #[tokio::test]
    async fn post_json_sends_the_body_as_json() {
        let parts = post_json(router(), "/echo", &json!({ "hello": "world" })).await;
        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(parts.content_type, "application/json");
        let body = parts.body.expect("echo response should be json");
        assert_json_include!(actual: body, expected: json!({
            "content_type": "application/json",
            "body": { "hello": "world" },
        }));
    }

    #[tokio::test]
    async fn post_json_with_headers_sends_the_json_body_and_request_headers() {
        let router: Router = Router::new().route(
            "/echo",
            post_route(|headers: HeaderMap, body: Bytes| async move {
                let idempotency_key = headers
                    .get("idempotency-key")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_owned();
                let parsed = serde_json::from_slice::<Value>(&body).ok();
                Json(json!({ "idempotency_key": idempotency_key, "body": parsed }))
            }),
        );
        let parts = post_json_with_headers(
            router,
            "/echo",
            &[("idempotency-key", "key-1")],
            &json!({ "hello": "world" }),
        )
        .await;
        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(parts.content_type, "application/json");
        let body = parts.body.expect("echo response should be json");
        assert_json_include!(actual: body, expected: json!({
            "idempotency_key": "key-1",
            "body": { "hello": "world" },
        }));
    }

    #[tokio::test]
    async fn put_json_sends_the_body_as_json() {
        let parts = put_json(router(), "/echo", &json!({ "hello": "world" })).await;
        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(parts.content_type, "application/json");
        let body = parts.body.expect("echo response should be json");
        assert_json_include!(actual: body, expected: json!({
            "content_type": "application/json",
            "body": { "hello": "world" },
        }));
    }

    #[tokio::test]
    async fn delete_collects_a_204_with_no_content_type_or_body() {
        let parts = delete(router(), "/gone").await;
        assert_eq!(parts.status, StatusCode::NO_CONTENT);
        assert_eq!(parts.content_type, "");
        assert_eq!(parts.body, None);
    }

    #[tokio::test]
    async fn delete_parses_a_json_error_body() {
        let router: Router = Router::new().route(
            "/gone",
            delete_route(|| async { (StatusCode::NOT_FOUND, Json(json!({ "error": "gone" }))) }),
        );
        let parts = delete(router, "/gone").await;
        assert_eq!(parts.status, StatusCode::NOT_FOUND);
        assert_eq!(parts.content_type, "application/json");
        let body = parts.body.expect("response should be json");
        assert_json_include!(actual: body, expected: json!({ "error": "gone" }));
    }

    #[tokio::test]
    async fn post_body_sets_the_given_content_type() {
        let parts = post_body(
            router(),
            "/echo",
            Some("text/plain"),
            Body::from("not json"),
        )
        .await;
        assert_eq!(parts.status, StatusCode::OK);
        let body = parts.body.expect("echo response should be json");
        assert_json_include!(actual: body, expected: json!({
            "content_type": "text/plain",
            "body": null,
        }));
    }

    #[tokio::test]
    async fn post_body_omits_the_content_type_header() {
        let parts = post_body(
            router(),
            "/echo",
            None,
            Body::from(json!({ "hello": "world" }).to_string()),
        )
        .await;
        assert_eq!(parts.status, StatusCode::OK);
        let body = parts.body.expect("echo response should be json");
        assert_json_include!(actual: body, expected: json!({
            "content_type": null,
            "body": { "hello": "world" },
        }));
    }

    #[tokio::test]
    async fn get_exposes_response_headers() {
        let router: Router = Router::new().route(
            "/hello",
            get_route(|| async { ([("x-custom", "custom-value")], "ok") }),
        );
        let parts = get(router, "/hello").await;
        assert_eq!(parts.header("x-custom"), Some("custom-value".to_owned()));
        assert_eq!(parts.header("x-absent"), None);
    }

    #[tokio::test]
    async fn get_with_headers_sends_the_request_headers() {
        let router: Router = Router::new().route(
            "/echo",
            get_route(|headers: HeaderMap| async move {
                let request_id = headers
                    .get("x-request-id")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_owned();
                Json(json!({ "request_id": request_id }))
            }),
        );
        let parts = get_with_headers(router, "/echo", &[("x-request-id", "id-1")]).await;
        let body = parts.body.expect("echo response should be json");
        assert_json_include!(actual: body, expected: json!({ "request_id": "id-1" }));
    }

    #[tokio::test]
    async fn header_maps_an_invalid_name_to_none() {
        let parts = get(router(), "/ping").await;
        assert_eq!(parts.header("not a header name"), None);
    }

    #[tokio::test]
    async fn delete_exposes_response_headers() {
        let router: Router = Router::new().route(
            "/gone",
            delete_route(|| async { ([("x-request-id", "id-1")], StatusCode::NO_CONTENT) }),
        );
        let parts = delete(router, "/gone").await;
        assert_eq!(parts.header("x-request-id"), Some("id-1".to_owned()));
    }

    #[tokio::test]
    async fn get_response_parts_collects_from_a_raw_response() {
        use axum::response::IntoResponse;

        let res = (StatusCode::CREATED, Json(json!({ "id": 7 }))).into_response();
        let parts: ResponseParts = get_response_parts(res).await;
        assert_eq!(parts.status, StatusCode::CREATED);
        assert_eq!(parts.content_type, "application/json");
        let body = parts.body.expect("response should be json");
        assert_json_include!(actual: body, expected: json!({ "id": 7 }));
    }
}
