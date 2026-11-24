//! HTTP integration tests for the `GET /v1/todos` (`list_todos`) endpoint,
//! exercised through `create_todos_api` against a migrated test database.

mod common;

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use jiff::Timestamp;
    use serde_json::{Value, json};
    use serde_json_assert::assert_json_include;
    use sqlx::PgPool;
    use uuid::Uuid;

    use super::common::todos_api;
    use todos_api::controller::TodoPageHttp;
    use web::test_support::{ResponseParts, get, next_cursor};

    /// Base for the deterministic fixture ids: a UUID-v7 layout (version nibble 7,
    /// zero timestamp) whose ordering is decided entirely by the low `n` bits.
    /// The values match the ids seeded by the `list_todos` fixture.
    const UUID_BASE: u128 = 0x0000_0000_0000_7000_8000_0000_0000_0000;

    #[must_use]
    fn create_todo_id(n: u32) -> Uuid {
        Uuid::from_u128(UUID_BASE + u128::from(n))
    }

    /// Collects the `todoId` of every item in a list response as parsed UUIDs.
    #[must_use]
    fn get_todo_ids(data: &Value) -> Vec<Uuid> {
        data["items"]
            .as_array()
            .expect("list response should contain an items array")
            .iter()
            .map(|item| {
                item["todoId"]
                    .as_str()
                    .and_then(|id| Uuid::parse_str(id).ok())
                    .expect("todoId should be a valid uuid")
            })
            .collect()
    }

    fn read_page(parts: ResponseParts) -> TodoPageHttp {
        serde_json::from_value(parts.body.expect("should have returned a page"))
            .expect("the page should deserialize")
    }

    #[sqlx::test(migrator = "todos::MIGRATOR", fixtures("list_todos"))]
    async fn returns_todos_ordered_by_id(postgres: PgPool) {
        let parts = get(todos_api(postgres), "/v1/todos").await;

        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(parts.content_type, "application/json");

        let ids = get_todo_ids(
            parts
                .body
                .as_ref()
                .expect("should have returned seeded todos"),
        );

        assert_eq!(
            ids,
            vec![
                create_todo_id(1),
                create_todo_id(2),
                create_todo_id(3),
                create_todo_id(4),
                create_todo_id(5),
            ]
        );
    }

    #[sqlx::test(migrator = "todos::MIGRATOR", fixtures("list_todos"))]
    async fn returns_pages_of_todos(postgres: PgPool) {
        let router = todos_api(postgres);

        let parts = get(router.clone(), "/v1/todos?limit=2").await;

        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(parts.content_type, "application/json");
        let ids = get_todo_ids(
            parts
                .body
                .as_ref()
                .expect("should have returned the first page"),
        );
        assert_eq!(ids.len(), 2);
        let cursor = next_cursor(&parts).expect("the first page should have a next cursor");

        let parts = get(
            router.clone(),
            &format!("/v1/todos?limit=2&cursor={cursor}"),
        )
        .await;

        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(parts.content_type, "application/json");
        let ids = get_todo_ids(
            parts
                .body
                .as_ref()
                .expect("should have returned the second page"),
        );
        assert_eq!(ids.len(), 2);
        let cursor = next_cursor(&parts).expect("the second page should have a next cursor");

        let parts = get(router, &format!("/v1/todos?limit=2&cursor={cursor}")).await;

        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(parts.content_type, "application/json");
        let ids = get_todo_ids(
            parts
                .body
                .as_ref()
                .expect("should have returned the last page"),
        );
        assert_eq!(ids.len(), 1);
        assert_eq!(
            next_cursor(&parts),
            None,
            "the last page should not have a next cursor"
        );
    }

    #[sqlx::test(migrator = "todos::MIGRATOR", fixtures("list_todos"))]
    async fn returns_completed_todos(postgres: PgPool) {
        let parts = get(todos_api(postgres), "/v1/todos?isCompleted=true").await;

        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(parts.content_type, "application/json");

        let todo_page = read_page(parts);

        assert_eq!(todo_page.items.len(), 2);
        assert_eq!(todo_page.next, None);

        todo_page
            .items
            .iter()
            .for_each(|todo| assert_ne!(todo.completed_at, None));
    }

    #[sqlx::test(migrator = "todos::MIGRATOR", fixtures("list_todos"))]
    async fn returns_incomplete_todos(postgres: PgPool) {
        let parts = get(todos_api(postgres), "/v1/todos?isCompleted=false").await;

        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(parts.content_type, "application/json");

        let todo_page = read_page(parts);

        assert_eq!(todo_page.items.len(), 3);
        assert_eq!(todo_page.next, None);

        todo_page
            .items
            .iter()
            .for_each(|todo| assert_eq!(todo.completed_at, None));
    }

    #[sqlx::test(migrator = "todos::MIGRATOR", fixtures("list_todos"))]
    async fn sorts_todos_by_updated_at(postgres: PgPool) {
        let parts = get(todos_api(postgres), "/v1/todos?sort=updatedAt").await;

        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(parts.content_type, "application/json");

        let todo_page = read_page(parts);

        assert_eq!(todo_page.items.len(), 5);
        assert_eq!(todo_page.next, None);

        let updated_dates = todo_page
            .items
            .iter()
            .map(|todo| todo.updated_at)
            .collect::<Vec<Timestamp>>();

        assert!(updated_dates.is_sorted());
    }

    #[sqlx::test(migrator = "todos::MIGRATOR", fixtures("list_todos"))]
    async fn sorts_todos_by_todo_id_descending(postgres: PgPool) {
        let parts = get(todos_api(postgres), "/v1/todos?sort=todoId&order=desc").await;

        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(parts.content_type, "application/json");

        let todo_page = read_page(parts);

        assert_eq!(todo_page.items.len(), 5);
        assert_eq!(todo_page.next, None);

        let todo_ids = todo_page
            .items
            .iter()
            .map(|todo| todo.todo_id)
            .collect::<Vec<Uuid>>();

        assert_eq!(
            todo_ids,
            vec![
                create_todo_id(5),
                create_todo_id(4),
                create_todo_id(3),
                create_todo_id(2),
                create_todo_id(1),
            ]
        );
    }

    #[sqlx::test(migrator = "todos::MIGRATOR")]
    async fn responds_400_for_a_malformed_cursor(postgres: PgPool) {
        let parts = get(todos_api(postgres), "/v1/todos?cursor=not-a-cursor").await;

        assert_eq!(parts.status, StatusCode::BAD_REQUEST);
        assert_eq!(parts.content_type, "application/json");
        assert_json_include!(actual: parts.body, expected: json!({
            "error": "cursor is malformed"
        }));
    }

    #[sqlx::test(migrator = "todos::MIGRATOR", fixtures("list_todos"))]
    async fn responds_422_when_the_cursor_sort_does_not_match(postgres: PgPool) {
        let router = todos_api(postgres);

        let parts = get(router.clone(), "/v1/todos?limit=2").await;

        assert_eq!(parts.status, StatusCode::OK);
        let cursor = next_cursor(&parts).expect("the first page should have a next cursor");

        let parts = get(
            router,
            &format!("/v1/todos?limit=2&sort=updatedAt&cursor={cursor}"),
        )
        .await;

        assert_eq!(parts.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(parts.content_type, "application/json");
        assert_json_include!(actual: parts.body, expected: json!({
            "error": "cursor does not match the requested sort"
        }));
    }
}
