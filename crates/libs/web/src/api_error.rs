//! HTTP error-response types for the web layer.
//!
//! [`crate::api_error::ApiError`] is an HTTP response-*body* type,
//! deliberately not `std::error::Error`: it pairs a status code with a
//! serializable body and implements
//! [`IntoResponse`](axum::response::IntoResponse) only. Apps own domain→HTTP
//! mapping via `From` impls in their `app_error.rs` (newtyping `ApiError`);
//! the only mapping that lives here is for errors the web layer itself
//! produces (`From<CursorError>`).

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use schemars::JsonSchema;
use serde::Serialize;

use crate::cursor_token::CursorError;
use crate::request_id;

/// The standard error body for plain, non-validation error responses.
#[derive(Serialize, JsonSchema)]
pub struct ErrorBody {
    /// A generic, client-safe description of the failure.
    pub error: String,
    /// The request id of the request that produced this error, filled in at
    /// response time from [`crate::request_id::current`] and omitted when the
    /// response is produced outside a request-id scope.
    #[serde(rename = "requestId", skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

/// One invalid input, part of a [`ValidationErrorBody`].
#[derive(Serialize, JsonSchema)]
pub struct ValidationDetail {
    /// Where the problem is. A JSON Pointer such as `/task` for request bodies,
    /// `?name=` for a query parameter (`?` alone for the query string as a whole),
    /// and `{name}` for a path parameter.
    /// A repeated query parameter adds the zero-based position of the bad value,
    /// such as `?id[1]=`.
    pub path: String,
    /// A client-safe description of what failed at `path`.
    pub message: String,
}

/// The error body for validation failures, optionally carrying per-field details.
#[derive(Serialize, JsonSchema)]
pub struct ValidationErrorBody {
    /// A generic, client-safe description of the validation failure.
    pub error: String,
    /// The request id of the request that produced this error, filled in at
    /// response time from [`crate::request_id::current`] and omitted when the
    /// response is produced outside a request-id scope.
    #[serde(rename = "requestId", skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Details for each invalid input, present only when validation produced
    /// per-field failures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Vec<ValidationDetail>>,
}

/// An error response: an HTTP status code paired with a serialized JSON
/// body, rendered via [`IntoResponse`]. When rendering an object-shaped body
/// under a request-id scope ([`crate::request_id::current`]), the current
/// request id is added as the body's `requestId` field.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    body: serde_json::Value,
}

impl ApiError {
    /// Creates an error response from a status code and a serializable body.
    ///
    /// # Panics
    ///
    /// Will panic when `body` cannot be serialized to JSON.
    pub fn new(status: StatusCode, body: &impl Serialize) -> Self {
        let body = serde_json::to_value(body)
            .expect("error bodies are plain structs and always serialize");
        Self { status, body }
    }

    /// Creates an error response whose body is an [`ErrorBody`] carrying
    /// `message` as its `error` field.
    #[must_use]
    pub fn message(status: StatusCode, message: &str) -> Self {
        Self::new(
            status,
            &ErrorBody {
                error: message.to_owned(),
                request_id: None,
            },
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = self.body;
        if let (Some(id), Some(object)) = (request_id::current(), body.as_object_mut()) {
            object.insert("requestId".to_owned(), serde_json::Value::String(id));
        }
        (self.status, Json(body)).into_response()
    }
}

/// Two-tier HTTP policy for cursor errors: a *malformed* cursor (base64/JSON
/// decode failure) is a client request error → `400 Bad Request`; an *invalid*
/// cursor (decodes but fails validation, e.g. sort-key mismatch) is a semantic
/// violation of the API schema → `422 Unprocessable Entity`. Both variants are
/// field-less, so `Display` yields only fixed, client-safe messages; internal
/// decode details are logged at debug level in
/// [`crate::cursor_token::decode`], never surfaced here.
impl From<CursorError> for ApiError {
    fn from(e: CursorError) -> Self {
        let status = match e {
            CursorError::Malformed => StatusCode::BAD_REQUEST,
            CursorError::SortMismatch => StatusCode::UNPROCESSABLE_ENTITY,
        };
        ApiError::message(status, &e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use axum::{Router, http::StatusCode, middleware, routing::get};
    use serde_json::Value;
    use serde_json_assert::assert_json_include;

    use super::ApiError;
    use crate::{request_id, test_support};

    async fn message_error() -> ApiError {
        ApiError::message(StatusCode::NOT_FOUND, "route not found")
    }

    fn scoped_router() -> Router {
        Router::new()
            .route("/boom", get(message_error))
            .route("/plain", get(plain_error))
            .layer(middleware::from_fn(request_id::scope))
    }

    async fn plain_error() -> ApiError {
        ApiError::new(StatusCode::OK, &"plain")
    }

    #[tokio::test]
    async fn error_body_inside_scope_carries_request_id() {
        let res = test_support::get_with_headers(
            scoped_router(),
            "/boom",
            &[(request_id::REQUEST_ID_HEADER.as_str(), "req-1")],
        )
        .await;
        let body = res.body.expect("the error body is JSON");
        assert_json_include!(
            actual: body,
            expected: serde_json::json!({ "error": "route not found", "requestId": "req-1" })
        );
    }

    #[tokio::test]
    async fn error_body_outside_scope_has_no_request_id() {
        let router = Router::new().route("/boom", get(message_error));
        let res = test_support::get_with_headers(
            router,
            "/boom",
            &[(request_id::REQUEST_ID_HEADER.as_str(), "req-1")],
        )
        .await;
        let body = res.body.expect("the error body is JSON");
        assert!(body.get("requestId").is_none());
        assert_json_include!(
            actual: body,
            expected: serde_json::json!({ "error": "route not found" })
        );
    }

    #[tokio::test]
    async fn error_body_without_header_has_no_request_id() {
        let res = test_support::get(scoped_router(), "/boom").await;
        let body = res.body.expect("the error body is JSON");
        assert!(body.get("requestId").is_none());
    }

    #[tokio::test]
    async fn non_object_body_is_not_mutated() {
        let res = test_support::get_with_headers(
            scoped_router(),
            "/plain",
            &[(request_id::REQUEST_ID_HEADER.as_str(), "req-1")],
        )
        .await;
        assert_eq!(res.body, Some(Value::String("plain".to_owned())));
    }

    #[test]
    fn error_body_schema_documents_optional_request_id() {
        let schema = serde_json::to_value(schemars::schema_for!(super::ErrorBody)).unwrap();
        let type_of = schema.pointer("/properties/requestId/type").unwrap();
        assert!(
            type_of
                .as_array()
                .unwrap()
                .contains(&Value::String("string".to_owned()))
        );
        let required = schema["required"].as_array().unwrap();
        assert!(!required.contains(&Value::String("requestId".to_owned())));
    }

    #[test]
    fn validation_error_body_schema_documents_optional_request_id() {
        let schema =
            serde_json::to_value(schemars::schema_for!(super::ValidationErrorBody)).unwrap();
        let type_of = schema.pointer("/properties/requestId/type").unwrap();
        assert!(
            type_of
                .as_array()
                .unwrap()
                .contains(&Value::String("string".to_owned()))
        );
        let required = schema["required"].as_array().unwrap();
        assert!(!required.contains(&Value::String("requestId".to_owned())));
    }
}
