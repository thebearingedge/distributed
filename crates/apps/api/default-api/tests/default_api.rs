//! Integration tests for the default-api application library.

use std::sync::Arc;

use axum::http::StatusCode;
use clap::Parser;
use opentelemetry::metrics::NoopMeterProvider;

use runtime::shutdown::Shutdown;
use web::config::ApiConfig;
use web::test_support::get;
use web::{Api, ApiContext};

use default_api::{DefaultApi, create_default_api};

#[tokio::test]
async fn root_is_ok() {
    let res = get(create_default_api(), "/").await;

    assert_eq!(res.status, StatusCode::OK);
}

#[tokio::test]
async fn unknown_paths_are_json_404() {
    let res = get(create_default_api(), "/nope").await;

    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let body = res.body.expect("the 404 body is JSON");
    assert_eq!(body["error"], "route not found");
}

#[tokio::test]
async fn started_api_is_ready() {
    let context = ApiContext::new(Shutdown::new(), Arc::new(NoopMeterProvider::new()));
    let parts = DefaultApi.start(&context).await.expect("DefaultApi starts");
    let config = ApiConfig::try_parse_from(["test"]).expect("test config parses");
    let router = parts.router(&config, context.shutdown(), &NoopMeterProvider::new());

    let res = get(router, "/readyz").await;

    assert_eq!(res.status, StatusCode::OK);
    let body = res.body.expect("the readyz body is JSON");
    let checks = body["checks"]
        .as_object()
        .expect("readyz reports a checks object");
    assert!(checks.is_empty(), "readyz reports no checks, got {body}");
}
