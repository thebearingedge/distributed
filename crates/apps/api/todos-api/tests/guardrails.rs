//! HTTP integration tests for the request guardrails, exercised through
//! `create_todos_api` against a test database.

mod common;

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::json;
    use serde_json_assert::assert_json_include;
    use sqlx::PgPool;

    use super::common::todos_api;
    use web::{config::REQUEST_BODY_LIMIT_DEFAULT_BYTES, test_support::post_json};

    #[sqlx::test]
    async fn rejects_an_oversized_body_with_a_json_413(postgres: PgPool) {
        let task = "a".repeat(REQUEST_BODY_LIMIT_DEFAULT_BYTES);
        let parts = post_json(todos_api(postgres), "/v1/todos", &json!({ "task": task })).await;

        assert_eq!(parts.status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(parts.content_type.starts_with("application/json"));
        let request_id = parts
            .header("x-request-id")
            .expect("response should carry an x-request-id");
        assert_json_include!(
            actual: parts.body,
            expected: json!({ "error": "request body too large", "requestId": request_id })
        );
    }
}
