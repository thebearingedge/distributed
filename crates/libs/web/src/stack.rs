//! The standard cross-cutting layer stack for API routers:
//! [`crate::stack::with_api_stack`].

use axum::Router;
use opentelemetry::metrics::MeterProvider;

use crate::{
    config::ApiConfig, cors::with_cors, guardrails::with_guardrails, metrics::with_http_metrics,
    middleware::with_http_tracing,
};

/// Applies the standard API layer stack to `router`: `with_http_metrics`,
/// then `with_cors`, then `with_http_tracing`, then `with_guardrails`,
/// outermost first, using the request guardrails and CORS policy configured
/// on `config`. Health probes must be merged **after** this call, so they
/// bypass the stack.
pub fn with_api_stack<P>(router: Router, config: &ApiConfig, meter_provider: &P) -> Router
where
    P: MeterProvider + ?Sized,
{
    with_http_metrics(
        with_cors(
            with_http_tracing(with_guardrails(router, config.guardrails())),
            config.cors(),
        ),
        meter_provider,
    )
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        http::StatusCode,
        routing::{get, post},
    };
    use clap::Parser;
    use jsonschema::Validator;
    use opentelemetry::{Value, metrics::NoopMeterProvider};
    use schemars::JsonSchema;
    use serde::Deserialize;

    use super::with_api_stack;
    use crate::{
        config::ApiConfig,
        extractors::{HasSchema, ValidatedJson},
        request_id::REQUEST_ID_HEADER,
        test_support,
    };
    use telemetry::test_support::MetricsHarness;

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

    fn stack_router(args: &[&str]) -> Router {
        let schema = serde_json::to_value(schemars::schema_for!(Payload)).unwrap();
        let state = TestState {
            payload: jsonschema::validator_for(&schema).unwrap(),
        };
        let mut command = vec!["test"];
        command.extend_from_slice(args);
        let config = ApiConfig::try_parse_from(command).unwrap();
        with_api_stack(
            Router::new()
                .route("/things", post(accept))
                .with_state(state),
            &config,
            &NoopMeterProvider::new(),
        )
    }

    #[tokio::test(start_paused = true)]
    async fn guardrail_errors_carry_the_request_id() {
        let router = stack_router(&[
            "--request-body-limit-bytes",
            "16",
            "--cors-allowed-origins=",
        ]);
        let res = test_support::post_json(
            router,
            "/things",
            &serde_json::json!({ "name": "a".repeat(32) }),
        )
        .await;

        assert_eq!(res.status, StatusCode::PAYLOAD_TOO_LARGE);
        let request_id = res
            .header(REQUEST_ID_HEADER.as_str())
            .expect("response should carry an x-request-id");
        let body = res.body.expect("the error body is JSON");
        assert_eq!(body["error"], "request body too large");
        assert_eq!(body["requestId"], serde_json::Value::from(request_id));
    }

    #[tokio::test(start_paused = true)]
    async fn requests_are_measured_with_their_route() {
        let harness = MetricsHarness::default();
        let config = ApiConfig::try_parse_from(["test", "--cors-allowed-origins="]).unwrap();
        let router = with_api_stack(
            Router::new().route("/things/{id}", get(|| async { "ok" })),
            &config,
            harness.provider(),
        );
        test_support::get(router, "/things/1").await;

        let points = harness.f64_histogram("http.server.request.duration");
        assert_eq!(points.len(), 1);
        assert_eq!(
            points[0].attribute("http.route"),
            Some(&Value::from("/things/{id}"))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cors_headers_cover_guardrail_errors() {
        const ALLOWED_ORIGIN: &str = "http://web.example";
        let router = stack_router(&[
            "--request-body-limit-bytes",
            "16",
            "--cors-allowed-origins",
            ALLOWED_ORIGIN,
        ]);
        let res = test_support::post_json_with_headers(
            router,
            "/things",
            &[("origin", ALLOWED_ORIGIN)],
            &serde_json::json!({ "name": "a".repeat(32) }),
        )
        .await;

        assert_eq!(res.status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            res.header("access-control-allow-origin").as_deref(),
            Some(ALLOWED_ORIGIN)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cors_is_off_by_default() {
        let router = stack_router(&["--cors-allowed-origins="]);
        let res = test_support::post_json_with_headers(
            router,
            "/things",
            &[("origin", "http://web.example")],
            &serde_json::json!({ "name": "a" }),
        )
        .await;

        assert_eq!(res.status, StatusCode::OK);
        assert!(res.header("access-control-allow-origin").is_none());
    }
}
