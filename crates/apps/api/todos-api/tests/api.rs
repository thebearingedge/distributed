//! HTTP integration tests for the todos-api endpoints, exercised through
//! `create_todos_api` against a test database.

mod common;

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::json;
    use serde_json_assert::assert_json_include;
    use sqlx::PgPool;

    use super::common::todos_api;
    use database::{
        IDEMPOTENCY_MAX_LENGTH, IDEMPOTENCY_MIN_LENGTH, IDEMPOTENCY_VISIBLE_ASCII_PATTERN,
    };
    use todos::TodosInteractor;
    use web::test_support::{delete, get, post_json};

    #[sqlx::test]
    async fn serves_openapi_document(postgres: PgPool) {
        let parts = get(todos_api(postgres), "/openapi.json").await;

        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(parts.content_type, "application/json");
        assert_json_include!(actual: parts.body, expected: json!({
            "openapi": "3.1.0",
            "info": { "title": "Todos API" },
            "paths": {
                "/v1/todos": {
                    "get": { "operationId": "listTodos" },
                    "post": { "operationId": "addTodo" }
                },
                "/v1/todos/{todo_id}": {
                    "get": { "operationId": "getTodo" },
                    "put": { "operationId": "updateTodo" }
                }
            }
        }));

        let body = parts
            .body
            .as_ref()
            .expect("should have returned the document");

        let description = body["info"]["description"]
            .as_str()
            .expect("info should have a description");
        assert!(
            !description.is_empty(),
            "info.description should not be empty"
        );
    }

    #[sqlx::test]
    async fn documents_the_update_todo_responses(postgres: PgPool) {
        let parts = get(todos_api(postgres), "/openapi.json").await;

        assert_eq!(parts.status, StatusCode::OK);

        let put = &parts.body.expect("should have returned the document")["paths"]["/v1/todos/{todo_id}"]
            ["put"];

        assert_json_include!(actual: put.clone(), expected: json!({
            "operationId": "updateTodo",
            "summary": "Update a todo"
        }));

        let responses = put["responses"]
            .as_object()
            .expect("the operation should document its responses");

        assert!(
            responses.contains_key("200"),
            "updateTodo should document a 200 response"
        );
        assert!(
            responses.contains_key("404"),
            "updateTodo should document a 404 response"
        );

        let found = responses
            .get("200")
            .expect("updateTodo should document a 200 response");

        let description = found["description"]
            .as_str()
            .expect("the 200 response should have a description");
        assert!(
            !description.is_empty(),
            "the 200 response description should not be empty"
        );

        let parameters = put["parameters"]
            .as_array()
            .expect("updateTodo should document its parameters");

        let todo_id = parameters
            .iter()
            .find(|parameter| parameter["in"] == "path" && parameter["name"] == "todo_id")
            .expect("updateTodo should document the todo_id path parameter");

        let description = todo_id["description"]
            .as_str()
            .expect("the todo_id path parameter should have a description");
        assert!(
            !description.is_empty(),
            "the todo_id path parameter description should not be empty"
        );
    }

    #[sqlx::test]
    async fn documents_413_on_body_accepting_operations(postgres: PgPool) {
        let parts = get(todos_api(postgres), "/openapi.json").await;

        assert_eq!(parts.status, StatusCode::OK);

        let body = parts.body.expect("should have returned the document");

        let post_responses = body["paths"]["/v1/todos"]["post"]["responses"]
            .as_object()
            .expect("addTodo should document its responses");
        assert!(
            post_responses.contains_key("413"),
            "addTodo should document a 413 response"
        );

        let put_responses = body["paths"]["/v1/todos/{todo_id}"]["put"]["responses"]
            .as_object()
            .expect("updateTodo should document its responses");
        assert!(
            put_responses.contains_key("413"),
            "updateTodo should document a 413 response"
        );

        let get_responses = body["paths"]["/v1/todos"]["get"]["responses"]
            .as_object()
            .expect("listTodos should document its responses");
        assert!(
            !get_responses.contains_key("413"),
            "listTodos should not document a 413 response"
        );
    }

    #[sqlx::test]
    async fn documents_descriptions_across_the_spec(postgres: PgPool) {
        let parts = get(todos_api(postgres), "/openapi.json").await;

        assert_eq!(parts.status, StatusCode::OK);

        let body = parts
            .body
            .as_ref()
            .expect("should have returned the document");

        for (path, method, operation_id) in [
            ("/v1/todos/{todo_id}", "get", "getTodo"),
            ("/v1/todos/{todo_id}", "put", "updateTodo"),
            ("/v1/todos", "get", "listTodos"),
            ("/v1/todos", "post", "addTodo"),
        ] {
            let operation = &body["paths"][path][method];

            assert_json_include!(actual: operation.clone(), expected: json!({
                "operationId": operation_id
            }));

            let description = operation["description"].as_str().unwrap_or_default();
            assert!(
                !description.is_empty(),
                "{operation_id} should have a description"
            );
        }

        let responses = body["paths"]["/v1/todos"]["post"]["responses"]
            .as_object()
            .expect("addTodo should document its responses");

        let created = responses
            .get("201")
            .expect("addTodo should document a 201 response");

        let description = created["description"].as_str().unwrap_or_default();
        assert!(
            !description.is_empty(),
            "the addTodo 201 response should have a description"
        );

        let responses = body["paths"]["/v1/todos"]["get"]["responses"]
            .as_object()
            .expect("listTodos should document its responses");

        let ok = responses
            .get("200")
            .expect("listTodos should document a 200 response");

        let description = ok["description"].as_str().unwrap_or_default();
        assert!(
            !description.is_empty(),
            "the listTodos 200 response should have a description"
        );

        let list_todos_query = &body["components"]["schemas"]["ListTodosQuery"]["properties"];

        for field in ["todoId", "isCompleted", "sort", "order", "limit", "cursor"] {
            let description = list_todos_query[field]["description"]
                .as_str()
                .unwrap_or_default();
            assert!(
                !description.is_empty(),
                "ListTodosQuery.{field} should have a description"
            );
        }

        for schema in ["NewTodoHttp", "TodoUpdateHttp"] {
            let description = body["components"]["schemas"][schema]["description"]
                .as_str()
                .unwrap_or_default();
            assert!(
                !description.is_empty(),
                "{schema} should have a type description"
            );
        }
    }

    #[sqlx::test]
    async fn documents_the_created_todo_location_header(postgres: PgPool) {
        let parts = get(todos_api(postgres), "/openapi.json").await;

        let body = parts
            .body
            .as_ref()
            .expect("should have returned the document");

        let created = &body["paths"]["/v1/todos"]["post"]["responses"]["201"];

        let location = &created["headers"]["Location"];

        assert_json_include!(actual: location.clone(), expected: json!({
            "required": true,
            "description": "The path of the new todo, relative to the service root.",
            "schema": { "type": "string" }
        }));

        let description = &body["paths"]["/v1/todos"]["post"]["description"];
        assert!(
            description
                .as_str()
                .unwrap_or_default()
                .contains("Location"),
            "the addTodo description should mention the Location header"
        );
    }

    #[sqlx::test]
    async fn documents_the_idempotency_key_header_schema(postgres: PgPool) {
        let parts = get(todos_api(postgres), "/openapi.json").await;

        let parameters = &parts.body.expect("should have returned the document")["paths"]["/v1/todos"]
            ["post"]["parameters"];
        let parameters = parameters
            .as_array()
            .expect("addTodo should document its parameters");

        let header = parameters
            .iter()
            .find(|parameter| parameter["in"] == "header" && parameter["name"] == "idempotency-key")
            .expect("addTodo should document the idempotency-key header");

        let schema = &header["schema"];
        assert_json_include!(actual: schema.clone(), expected: json!({
            "type": "string",
            "minLength": IDEMPOTENCY_MIN_LENGTH,
            "maxLength": IDEMPOTENCY_MAX_LENGTH,
            "pattern": IDEMPOTENCY_VISIBLE_ASCII_PATTERN
        }));
    }

    #[sqlx::test]
    async fn documents_the_list_limit_maximum(postgres: PgPool) {
        let parts = get(todos_api(postgres), "/openapi.json").await;

        let limit = &parts.body.expect("should have returned the document")["components"]["schemas"]
            ["ListTodosQuery"]["properties"]["limit"];

        assert_json_include!(actual: limit.clone(), expected: json!({
            "minimum": 1,
            "maximum": TodosInteractor::MAX_PAGE_SIZE
        }));
    }

    #[sqlx::test]
    async fn serves_json_404_for_unknown_routes(postgres: PgPool) {
        let parts = get(todos_api(postgres), "/nope").await;

        assert_eq!(parts.status, StatusCode::NOT_FOUND);
        assert_eq!(parts.content_type, "application/json");
        assert_json_include!(actual: parts.body, expected: json!({
            "error": "route not found"
        }));
    }

    #[sqlx::test]
    async fn serves_json_405_for_wrong_methods(postgres: PgPool) {
        let parts = delete(todos_api(postgres), "/v1/todos").await;

        assert_eq!(parts.status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(parts.content_type, "application/json");
        assert_json_include!(actual: parts.body, expected: json!({
            "error": "method not allowed"
        }));
    }

    #[sqlx::test]
    async fn responds_503_when_the_database_is_unavailable(postgres: PgPool) {
        let router = todos_api(postgres.clone());

        postgres.close().await;

        let parts = post_json(router, "/v1/todos", &json!({ "task": "write some rust!" })).await;

        assert_eq!(parts.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(parts.content_type, "application/json");
        assert_json_include!(actual: parts.body, expected: json!({
            "error": "service unavailable"
        }));
    }
}
