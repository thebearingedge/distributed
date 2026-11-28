use axum::{Router, extract::DefaultBodyLimit};

/// Caps request bodies read by body extractors on every route `router` has
/// at call time at `limit_bytes` bytes; larger bodies read through
/// [`crate::extractors::ValidatedJson`] are rejected with
/// `413 Payload Too Large` and the standard JSON error body
/// (`{"error": "request body too large"}`).
pub(crate) fn with_body_limit(router: Router, limit_bytes: usize) -> Router {
    router.layer(DefaultBodyLimit::max(limit_bytes))
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode, header},
        routing::post,
    };
    use http_body_util::BodyExt;
    use jsonschema::Validator;
    use schemars::JsonSchema;
    use serde::Deserialize;
    use serde_json::Value;
    use serde_json_assert::assert_json_include;
    use tower::ServiceExt;

    use super::with_body_limit;
    use crate::extractors::{HasSchema, ValidatedJson};

    #[derive(Deserialize, JsonSchema)]
    struct Payload {
        name: String,
    }

    #[derive(Clone)]
    struct TestState {
        payload: Validator,
    }

    impl HasSchema<Payload> for TestState {
        fn validator_for(&self) -> &Validator {
            &self.payload
        }
    }

    async fn accept(ValidatedJson(payload): ValidatedJson<Payload>) -> String {
        payload.name
    }

    fn router() -> Router {
        let schema = serde_json::to_value(schemars::schema_for!(Payload)).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        with_body_limit(
            Router::new()
                .route("/accept", post(accept))
                .with_state(TestState { payload: validator }),
            32,
        )
    }

    #[tokio::test]
    async fn oversized_bodies_get_a_json_413() {
        let body = serde_json::to_vec(&serde_json::json!({ "name": "a".repeat(64) })).unwrap();
        let req = Request::builder()
            .method("POST")
            .uri("/accept")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap();
        let res = router().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let content_type = res
            .headers()
            .get(header::CONTENT_TYPE)
            .map_or_default(|v| v.to_str().unwrap().to_owned());
        assert!(content_type.starts_with("application/json"));
        let data: Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_json_include!(actual: data, expected: serde_json::json!({
            "error": "request body too large"
        }));
    }

    #[tokio::test]
    async fn bodies_within_the_limit_are_accepted() {
        let req = Request::builder()
            .method("POST")
            .uri("/accept")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::json!({ "name": "a" }).to_string()))
            .unwrap();
        let res = router().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&bytes[..], b"a");
    }
}
