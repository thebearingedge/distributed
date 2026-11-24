//! HTTP integration tests for the `POST /v1/todos` (`add_todo`) endpoint,
//! exercised through `create_todos_api` against a migrated test database.

mod common;

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::StatusCode};
    use claims::assert_ok;
    use jiff::Timestamp;
    use serde_json::json;
    use serde_json_assert::assert_json_include;
    use sqlx::PgPool;
    use uuid::Uuid;

    use super::common::todos_api;
    use web::test_support::{get, post_body, post_json};

    #[sqlx::test(migrator = "todos::MIGRATOR")]
    async fn requires_a_json_body(postgres: PgPool) {
        let parts = post_body(todos_api(postgres), "/v1/todos", None, Body::empty()).await;

        assert_eq!(parts.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(parts.content_type, "application/json");
        assert_json_include!(actual: parts.body, expected: json!({
            "error": "Expected request with `Content-Type: application/json`"
        }));
    }

    #[sqlx::test(migrator = "todos::MIGRATOR")]
    async fn requires_a_valid_json_body(postgres: PgPool) {
        let parts = post_body(
            todos_api(postgres),
            "/v1/todos",
            Some("application/json"),
            Body::from("{"),
        )
        .await;

        assert_eq!(parts.status, StatusCode::BAD_REQUEST);
        assert_eq!(parts.content_type, "application/json");
        assert_json_include!(actual: parts.body, expected: json!({
            "error": "Failed to parse the request body as JSON: EOF while parsing an object at line 1 column 1"
        }));
    }

    #[sqlx::test(migrator = "todos::MIGRATOR")]
    async fn rejects_a_missing_task(postgres: PgPool) {
        let parts = post_json(todos_api(postgres), "/v1/todos", &json!({ "task": "" })).await;

        assert_eq!(parts.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(parts.content_type, "application/json");
        assert_json_include!(actual: parts.body, expected: json!({
            "details": [
                {
                    "message": "\"\" is shorter than 1 character",
                    "path": "/task"
                }
            ],
            "error": "schema violation"
        }));
    }

    #[sqlx::test(migrator = "todos::MIGRATOR")]
    async fn accepts_a_single_character_task(postgres: PgPool) {
        let parts = post_json(todos_api(postgres), "/v1/todos", &json!({ "task": "a" })).await;

        assert_eq!(parts.status, StatusCode::CREATED);
        assert_json_include!(actual: parts.body.expect("should have returned the added todo"), expected: json!({
            "task": "a",
            "completedAt": null
        }));
    }

    #[sqlx::test(migrator = "todos::MIGRATOR")]
    async fn rejects_a_whitespace_only_task(postgres: PgPool) {
        let parts = post_json(todos_api(postgres), "/v1/todos", &json!({ "task": "   " })).await;

        assert_eq!(parts.status, StatusCode::BAD_REQUEST);
        assert_json_include!(actual: parts.body.expect("should have returned an error body"), expected: json!({
            "error": "task must not be empty"
        }));
    }

    #[sqlx::test(migrator = "todos::MIGRATOR")]
    async fn responds_with_the_added_todo(postgres: PgPool) {
        let router = todos_api(postgres);
        let parts = post_json(
            router.clone(),
            "/v1/todos",
            &json!({ "task": "write some rust!" }),
        )
        .await;

        assert_eq!(parts.status, StatusCode::CREATED);
        assert_eq!(parts.content_type, "application/json");

        let data = parts
            .body
            .as_ref()
            .expect("should have returned the added todo");

        let todo_id = data["todoId"].as_str().expect("todoId should be a string");
        let todo_id = Uuid::parse_str(todo_id).expect("todoId should be a valid uuid");

        assert_eq!(todo_id.get_version_num(), 7);

        let updated_at = data["updatedAt"]
            .as_str()
            .expect("updatedAt should be a string");

        assert_ok!(updated_at.parse::<Timestamp>());
        assert_json_include!(actual: parts.body, expected: json!({
            "task": "write some rust!",
            "completedAt": null
        }));

        let location = parts
            .header("location")
            .expect("should have returned a Location header");
        assert_eq!(location, format!("/v1/todos/{todo_id}"));

        let fetched = get(router, &location).await;

        assert_eq!(fetched.status, StatusCode::OK);
        assert_eq!(fetched.content_type, "application/json");
        assert_json_include!(actual: fetched.body, expected: json!({
            "todoId": todo_id.to_string(),
            "task": "write some rust!",
            "completedAt": null
        }));
    }
}
