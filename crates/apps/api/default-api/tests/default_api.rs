//! Integration tests for the default-api application library.

use std::sync::Arc;

use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use clap::Parser;
use opentelemetry::metrics::NoopMeterProvider;

use runtime::shutdown::Shutdown;
use web::config::ApiConfig;
use web::test_support::get;
use web::{Api, ApiContext};

use default_api::{DefaultApi, create_default_api};

/// Serves a router through `web::test_support::get`, which requires a
/// `Content-Type` response header to parse; supplies `application/json` when
/// a handler (like the empty-body `200 OK` from [`create_default_api`]) sends
/// none.
async fn get_with_fallback_content_type(
    router: axum::Router,
    uri: &str,
) -> web::test_support::ResponseParts {
    let router = router.layer(axum::middleware::from_fn(
        async |req: Request, next: Next| -> Response {
            let mut res = next.run(req).await;
            res.headers_mut()
                .entry(header::CONTENT_TYPE)
                .or_insert_with(|| header::HeaderValue::from_static("application/json"));
            res
        },
    ));
    get(router, uri).await
}

#[tokio::test]
async fn root_is_ok() {
    let res = get_with_fallback_content_type(create_default_api(), "/").await;

    assert_eq!(res.status, StatusCode::OK);
}

#[tokio::test]
async fn unknown_paths_are_json_404() {
    let res = get(create_default_api(), "/nope").await;

    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert!(
        res.content_type.starts_with("application/json"),
        "content type is JSON, got {}",
        res.content_type
    );
    let body = res.body.expect("the 404 body is JSON");
    assert_eq!(body["error"], "route not found");
}

#[tokio::test]
async fn started_api_is_ready() {
    let context = ApiContext::new(Arc::new(NoopMeterProvider::new()));
    let parts = DefaultApi.start(&context).await.expect("DefaultApi starts");
    let config = ApiConfig::try_parse_from(["test"]).expect("test config parses");
    let router = parts.router(&config, &Shutdown::new(), &NoopMeterProvider::new());

    let res = get(router, "/readyz").await;

    assert_eq!(res.status, StatusCode::OK);
    let body = res.body.expect("the readyz body is JSON");
    let checks = body["checks"]
        .as_object()
        .expect("readyz reports a checks object");
    assert!(checks.is_empty(), "readyz reports no checks, got {body}");
}
