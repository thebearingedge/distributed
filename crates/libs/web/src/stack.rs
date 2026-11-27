//! The standard cross-cutting layer stack for API routers:
//! [`crate::stack::with_api_stack`].

use axum::Router;
use opentelemetry::metrics::MeterProvider;

use crate::{
    cors::{CorsConfig, with_cors},
    guardrails::{GuardrailsConfig, with_guardrails},
    metrics::with_http_metrics,
    middleware::with_http_tracing,
};

/// Configuration for [`crate::stack::with_api_stack`].
#[derive(Debug, Clone, Default)]
pub struct StackConfig {
    /// Request guardrails applied inside the request-id tracing layer.
    pub guardrails: GuardrailsConfig,
    /// CORS policy applied over the traced router; empty origins mean CORS
    /// is off.
    pub cors: CorsConfig,
}

/// Applies the standard API layer stack to `router`. The layers, outermost
/// first:
///
/// 1. [`crate::metrics::with_http_metrics`]
/// 2. [`crate::cors::with_cors`]
/// 3. [`crate::middleware::with_http_tracing`]
/// 4. [`crate::guardrails::with_guardrails`]
///
/// Layers that build [`crate::api_error::ApiError`] bodies sit inside
/// [`crate::middleware::with_http_tracing`], so the bodies carry the request
/// id. Health probes must be merged **after** this call, so they bypass the
/// stack.
pub fn with_api_stack<P>(router: Router, config: StackConfig, meter_provider: &P) -> Router
where
    P: MeterProvider + ?Sized,
{
    with_http_metrics(
        with_cors(
            with_http_tracing(with_guardrails(router, config.guardrails)),
            config.cors,
        ),
        meter_provider,
    )
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::Body,
        http::{HeaderValue, Request, Response, StatusCode, header},
        routing::{get, post},
    };
    use http_body_util::BodyExt;
    use jsonschema::Validator;
    use opentelemetry::{Value, metrics::NoopMeterProvider};
    use schemars::JsonSchema;
    use serde::Deserialize;
    use tower::ServiceExt;

    use super::{StackConfig, with_api_stack};
    use crate::{
        cors::CorsConfig,
        extractors::{HasSchema, ValidatedJson},
        guardrails::GuardrailsConfig,
        request_id::REQUEST_ID_HEADER,
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

    fn stack_router(config: StackConfig) -> Router {
        let schema = serde_json::to_value(schemars::schema_for!(Payload)).unwrap();
        let state = TestState {
            payload: jsonschema::validator_for(&schema).unwrap(),
        };
        with_api_stack(
            Router::new()
                .route("/things", post(accept))
                .with_state(state),
            config,
            &NoopMeterProvider::new(),
        )
    }

    fn post_request(
        uri: &'static str,
        origin: Option<&'static str>,
        body: &'static str,
    ) -> Request<Body> {
        let mut builder = Request::builder().method("POST").uri(uri);
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap()
    }

    async fn body_json(res: Response<Body>) -> serde_json::Value {
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn guardrail_errors_carry_the_request_id() {
        let router = stack_router(StackConfig {
            guardrails: GuardrailsConfig {
                body_limit_bytes: 16,
                ..GuardrailsConfig::default()
            },
            ..StackConfig::default()
        });
        let res = router
            .oneshot(post_request(
                "/things",
                None,
                r#"{"name":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
            ))
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let request_id = res
            .headers()
            .get(&REQUEST_ID_HEADER)
            .and_then(|value| value.to_str().ok())
            .expect("response should carry an x-request-id")
            .to_owned();
        let body = body_json(res).await;
        assert_eq!(body["error"], "request body too large");
        assert_eq!(body["requestId"], serde_json::Value::from(request_id));
    }

    #[tokio::test(start_paused = true)]
    async fn requests_are_measured_with_their_route() {
        let harness = MetricsHarness::default();
        let router = with_api_stack(
            Router::new().route("/things/{id}", get(|| async { "ok" })),
            StackConfig::default(),
            harness.provider(),
        );
        router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/things/1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

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
        let router = stack_router(StackConfig {
            guardrails: GuardrailsConfig {
                body_limit_bytes: 16,
                ..GuardrailsConfig::default()
            },
            cors: CorsConfig {
                allowed_origins: vec![HeaderValue::from_static(ALLOWED_ORIGIN)],
                ..CorsConfig::default()
            },
        });
        let res = router
            .oneshot(post_request(
                "/things",
                Some(ALLOWED_ORIGIN),
                r#"{"name":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
            ))
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            res.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_static(ALLOWED_ORIGIN))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cors_is_off_by_default() {
        let router = stack_router(StackConfig::default());
        let res = router
            .oneshot(post_request(
                "/things",
                Some("http://web.example"),
                r#"{"name":"a"}"#,
            ))
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert!(
            res.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .is_none()
        );
    }
}
