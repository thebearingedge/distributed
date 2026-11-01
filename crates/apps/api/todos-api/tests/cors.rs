//! HTTP integration tests for CORS behavior, exercised through the app
//! router composed the way `main.rs` composes it: the CORS layer applied
//! outermost, wrapping the traced todos API router.

mod common;

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::Body,
        http::{self, HeaderMap, HeaderValue, Request, StatusCode, header},
    };
    use serde_json::json;
    use serde_json_assert::assert_json_include;
    use sqlx::PgPool;
    use tower::ServiceExt;

    use super::common::todos_api;
    use todos_api::{api::create_todos_api, app_state::AppState};
    use web::cors::{CorsConfig, with_cors};
    use web::middleware::with_http_tracing;
    use web::test_support::{get_with_headers, post_json_with_headers};

    const ALLOWED_ORIGIN: &str = "http://localhost:5173";
    const UNLISTED_ORIGIN: &str = "http://evil.example";
    const PREFLIGHT_REQUEST_HEADERS: &str = "content-type, idempotency-key";

    /// Builds the todos API router with the CORS layer outermost, exactly as
    /// `main.rs` composes it.
    fn cors_api(postgres: PgPool, allowed_origins: Vec<HeaderValue>) -> Router {
        with_cors(
            with_http_tracing(create_todos_api(AppState::new(postgres))),
            CorsConfig {
                allowed_origins,
                ..CorsConfig::default()
            },
        )
    }

    fn allowed_origins() -> Vec<HeaderValue> {
        vec![HeaderValue::from_static(ALLOWED_ORIGIN)]
    }

    fn preflight_for(origin: &str, request_method: &str) -> Request<Body> {
        Request::builder()
            .method(http::Method::OPTIONS)
            .uri("/v1/todos")
            .header(header::ORIGIN, origin)
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, request_method)
            .header(
                header::ACCESS_CONTROL_REQUEST_HEADERS,
                PREFLIGHT_REQUEST_HEADERS,
            )
            .body(Body::empty())
            .expect("OPTIONS request should be buildable")
    }

    async fn preflight_parts(
        router: Router,
        origin: &str,
        request_method: &str,
    ) -> (StatusCode, HeaderMap) {
        let response = router
            .oneshot(preflight_for(origin, request_method))
            .await
            .expect("OPTIONS request should be handled");
        let status = response.status();
        let headers = response.headers().clone();
        (status, headers)
    }

    fn has_cors_headers(headers: &HeaderMap) -> bool {
        headers
            .keys()
            .any(|name| name.as_str().starts_with("access-control-"))
    }

    #[sqlx::test]
    async fn preflight_from_an_allowed_origin_is_answered(postgres: PgPool) {
        let (status, headers) = preflight_parts(
            cors_api(postgres, allowed_origins()),
            ALLOWED_ORIGIN,
            "POST",
        )
        .await;

        assert!(status.is_success());
        assert_eq!(
            headers.get("access-control-allow-origin"),
            Some(&HeaderValue::from_static(ALLOWED_ORIGIN))
        );
    }

    #[sqlx::test]
    async fn preflight_for_a_method_beyond_the_default_set_is_answered(postgres: PgPool) {
        let (status, headers) =
            preflight_parts(cors_api(postgres, allowed_origins()), ALLOWED_ORIGIN, "PUT").await;

        assert!(status.is_success());
        assert_eq!(
            headers.get("access-control-allow-methods"),
            Some(&HeaderValue::from_static("GET,POST,PUT,DELETE"))
        );
    }

    #[sqlx::test]
    async fn preflight_from_an_unlisted_origin_has_no_allow_origin_header(postgres: PgPool) {
        let (status, headers) = preflight_parts(
            cors_api(postgres, allowed_origins()),
            UNLISTED_ORIGIN,
            "POST",
        )
        .await;

        assert!(status.is_success());
        assert!(headers.get("access-control-allow-origin").is_none());
    }

    #[sqlx::test(migrator = "todos::MIGRATOR")]
    async fn post_from_an_allowed_origin_exposes_the_location_header(postgres: PgPool) {
        let parts = post_json_with_headers(
            cors_api(postgres, allowed_origins()),
            "/v1/todos",
            &[("origin", ALLOWED_ORIGIN)],
            &json!({ "task": "write some rust!" }),
        )
        .await;

        assert_eq!(parts.status, StatusCode::CREATED);
        assert_eq!(
            parts.header("access-control-allow-origin"),
            Some(ALLOWED_ORIGIN.to_owned())
        );
        let exposed = parts
            .header("access-control-expose-headers")
            .expect("response should expose CORS response headers");
        assert!(exposed.contains("location"));
        assert!(parts.header("location").is_some());
    }

    #[sqlx::test]
    async fn validation_errors_carry_the_allow_origin_header(postgres: PgPool) {
        let parts = post_json_with_headers(
            cors_api(postgres, allowed_origins()),
            "/v1/todos",
            &[("origin", ALLOWED_ORIGIN)],
            &json!({ "task": "" }),
        )
        .await;

        assert_eq!(parts.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(parts.content_type, "application/json");
        assert_eq!(
            parts.header("access-control-allow-origin"),
            Some(ALLOWED_ORIGIN.to_owned())
        );
        assert_json_include!(actual: parts.body, expected: json!({
            "error": "schema violation"
        }));
    }

    #[sqlx::test]
    async fn unknown_routes_carry_the_allow_origin_header(postgres: PgPool) {
        let parts = get_with_headers(
            cors_api(postgres, allowed_origins()),
            "/nope",
            &[("origin", ALLOWED_ORIGIN)],
        )
        .await;

        assert_eq!(parts.status, StatusCode::NOT_FOUND);
        assert_eq!(parts.content_type, "application/json");
        assert_eq!(
            parts.header("access-control-allow-origin"),
            Some(ALLOWED_ORIGIN.to_owned())
        );
        assert_json_include!(actual: parts.body, expected: json!({
            "error": "route not found"
        }));
    }

    #[sqlx::test]
    async fn cors_is_off_without_configured_origins(postgres: PgPool) {
        let (status, headers) = preflight_parts(todos_api(postgres), ALLOWED_ORIGIN, "POST").await;

        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert!(!has_cors_headers(&headers));
    }
}
