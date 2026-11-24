//! HTTP integration tests for the `PUT /v1/todos/{todo_id}` (`update_todo`)
//! endpoint, exercised through `create_todos_api` against a migrated test
//! database.

mod common;

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use claims::assert_ok;
    use jiff::Timestamp;
    use serde_json::json;
    use serde_json_assert::assert_json_include;
    use sqlx::PgPool;
    use uuid::Uuid;

    use super::common::todos_api;
    use web::test_support::put_json;

    #[sqlx::test(migrator = "todos::MIGRATOR", fixtures("update_todo"))]
    async fn responds_with_the_updated_todo(postgres: PgPool) {
        let parts = put_json(
            todos_api(postgres),
            "/v1/todos/00000000-0000-7000-8000-000000000001",
            &json!({ "task": "write more rust!", "isCompleted": true }),
        )
        .await;

        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(parts.content_type, "application/json");

        let data = parts
            .body
            .as_ref()
            .expect("should have returned the updated todo");

        let updated_at = data["updatedAt"]
            .as_str()
            .expect("updatedAt should be a string");
        let updated_at = assert_ok!(updated_at.parse::<Timestamp>());
        let original_updated_at = assert_ok!("2026-01-05T00:00:00Z".parse::<Timestamp>());
        assert!(
            updated_at > original_updated_at,
            "updatedAt should strictly increase ({original_updated_at} -> {updated_at})"
        );

        let completed_at = data["completedAt"]
            .as_str()
            .expect("completedAt should be set for a completed todo");
        assert_ok!(completed_at.parse::<Timestamp>());

        assert_json_include!(actual: parts.body, expected: json!({
            "task": "write more rust!"
        }));
    }

    #[sqlx::test(migrator = "todos::MIGRATOR", fixtures("update_todo"))]
    async fn clears_completed_at_when_an_updated_todo_is_uncompleted(postgres: PgPool) {
        let parts = put_json(
            todos_api(postgres),
            "/v1/todos/00000000-0000-7000-8000-000000000002",
            &json!({ "task": "write less rust!", "isCompleted": false }),
        )
        .await;

        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(parts.content_type, "application/json");
        assert_json_include!(actual: parts.body, expected: json!({
            "task": "write less rust!",
            "completedAt": null
        }));
    }

    #[sqlx::test(migrator = "todos::MIGRATOR", fixtures("update_todo"))]
    async fn refreshes_completed_at_when_an_updated_todo_stays_completed(postgres: PgPool) {
        let parts = put_json(
            todos_api(postgres),
            "/v1/todos/00000000-0000-7000-8000-000000000002",
            &json!({ "task": "write more rust!", "isCompleted": true }),
        )
        .await;

        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(parts.content_type, "application/json");

        let data = parts
            .body
            .as_ref()
            .expect("should have returned the updated todo");

        let completed_at = data["completedAt"]
            .as_str()
            .expect("completedAt should stay set for a completed todo");
        let completed_at = assert_ok!(completed_at.parse::<Timestamp>());
        let original_completed_at = assert_ok!("2026-01-06T00:00:00Z".parse::<Timestamp>());
        assert!(
            completed_at > original_completed_at,
            "completedAt should refresh ({original_completed_at} -> {completed_at})"
        );
    }

    #[sqlx::test(migrator = "todos::MIGRATOR", fixtures("update_todo"))]
    async fn returns_404_when_updating_a_missing_todo(postgres: PgPool) {
        let parts = put_json(
            todos_api(postgres),
            &format!("/v1/todos/{}", Uuid::now_v7()),
            &json!({ "task": "write some rust!", "isCompleted": false }),
        )
        .await;

        assert_eq!(parts.status, StatusCode::NOT_FOUND);
        assert_eq!(parts.content_type, "application/json");
        assert_json_include!(actual: parts.body, expected: json!({
            "error": "todo not found"
        }));
    }

    #[sqlx::test(migrator = "todos::MIGRATOR", fixtures("update_todo"))]
    async fn rejects_a_whitespace_only_task_when_updating(postgres: PgPool) {
        let parts = put_json(
            todos_api(postgres),
            "/v1/todos/00000000-0000-7000-8000-000000000001",
            &json!({ "task": "   ", "isCompleted": false }),
        )
        .await;

        assert_eq!(parts.status, StatusCode::BAD_REQUEST);
        assert_json_include!(actual: parts.body.expect("should have returned an error body"), expected: json!({
            "error": "task must not be empty"
        }));
    }
}
