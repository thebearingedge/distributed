use runtime::health::HealthCheck;
use runtime::heartbeat::Heartbeat;

/// Name of the heartbeat liveness check in `/livez` responses.
pub(crate) const WORKER_HEARTBEAT_CHECK: &str = "heartbeat";

/// The worker's heartbeat liveness check: `/livez` fails once the heartbeat
/// goes stale.
pub(crate) fn heartbeat_check(heartbeat: &Heartbeat) -> HealthCheck {
    HealthCheck::with_resource(
        WORKER_HEARTBEAT_CHECK,
        heartbeat.clone(),
        |heartbeat| async move { heartbeat.check() },
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::Router;
    use serde_json::json;
    use tokio::time::advance;

    use runtime::heartbeat::Heartbeat;
    use runtime::shutdown::Shutdown;
    use web::ApiParts;
    use web::test_support::get;

    use super::heartbeat_check;

    fn router(heartbeat: &Heartbeat, shutdown: &Shutdown) -> Router {
        ApiParts::new(Router::new())
            .liveness(heartbeat_check(heartbeat))
            .probes(Duration::from_secs(2), shutdown)
    }

    #[tokio::test(start_paused = true)]
    async fn livez_ok_while_heartbeat_fresh() {
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        let shutdown = Shutdown::new();
        let parts = get(router(&heartbeat, &shutdown), "/livez").await;
        assert_eq!(parts.status, axum::http::StatusCode::OK);
        assert_eq!(
            parts.body,
            Some(json!({"status": "ok", "checks": {"heartbeat": "ok"}}))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn livez_unavailable_when_heartbeat_stale() {
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        let shutdown = Shutdown::new();
        advance(Duration::from_secs(31)).await;
        let parts = get(router(&heartbeat, &shutdown), "/livez").await;
        assert_eq!(parts.status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            parts.body,
            Some(json!({"status": "unavailable", "checks": {"heartbeat": "failed"}}))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn readyz_ok_before_shutdown() {
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        let shutdown = Shutdown::new();
        let parts = get(router(&heartbeat, &shutdown), "/readyz").await;
        assert_eq!(parts.status, axum::http::StatusCode::OK);
        assert_eq!(parts.body, Some(json!({"status": "ok", "checks": {}})));
    }

    #[tokio::test(start_paused = true)]
    async fn readyz_shutting_down_after_trigger() {
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        let shutdown = Shutdown::new();
        shutdown.trigger();
        let parts = get(router(&heartbeat, &shutdown), "/readyz").await;
        assert_eq!(parts.status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            parts.body,
            Some(json!({"status": "shutting_down", "checks": {}}))
        );
    }
}
