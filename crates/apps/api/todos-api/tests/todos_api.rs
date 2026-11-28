//! Integration tests for the todos API process: [`TodosApi::parts`] runs
//! the migrations itself and produces the router the process serves.

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::http::StatusCode;
    use clap::Parser;
    use opentelemetry::metrics::NoopMeterProvider;
    use serde_json::json;
    use serde_json_assert::assert_json_include;
    use sqlx::PgPool;

    use runtime::shutdown::Shutdown;
    use todos_api::TodosApi;
    use web::ApiContext;
    use web::config::ApiConfig;
    use web::test_support::{get, post_json};

    #[sqlx::test]
    async fn parts_migrate_and_report_postgres_ready(postgres: PgPool) {
        let context = ApiContext::new(Shutdown::new(), Arc::new(NoopMeterProvider::new()));
        let parts = TodosApi::parts(postgres, &context)
            .await
            .expect("parts builds against the test database");
        let config = ApiConfig::try_parse_from(["test"]).expect("test config parses");
        let router = parts.router(&config, context.shutdown(), &NoopMeterProvider::new());

        let ready = get(router.clone(), "/readyz").await;
        assert_eq!(ready.status, StatusCode::OK);
        assert_json_include!(
            actual: ready.body,
            expected: json!({ "status": "ok", "checks": { "postgres": "ok" } })
        );

        let created = post_json(router, "/v1/todos", &json!({ "task": "x" })).await;
        assert_eq!(created.status, StatusCode::CREATED);
    }
}
