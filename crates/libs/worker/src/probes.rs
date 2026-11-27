use std::time::Duration;

use runtime::health::HealthCheck;
use runtime::heartbeat::Heartbeat;
use runtime::shutdown::Shutdown;
use web::health::HealthRouter;

/// Name of the heartbeat liveness check in `/livez` responses.
pub const WORKER_HEARTBEAT_CHECK: &str = "heartbeat";

/// The worker's probe router: `/livez` fails once the heartbeat goes stale;
/// `/readyz` fails once shutdown is triggered.
///
/// There are no readiness checks: readiness is "ok until shutdown is
/// triggered", and the router is only served after initialization.
pub fn probes_router(
    heartbeat: &Heartbeat,
    health_check_timeout: Duration,
    shutdown: Shutdown,
) -> axum::Router {
    HealthRouter::new(health_check_timeout, shutdown)
        .liveness(HealthCheck::with_resource(
            WORKER_HEARTBEAT_CHECK,
            heartbeat.clone(),
            |heartbeat| async move { heartbeat.check() },
        ))
        .into_router()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use runtime::heartbeat::Heartbeat;
    use runtime::shutdown::Shutdown;
    use web::test_support::get;

    use super::probes_router;

    fn router(heartbeat: &Heartbeat, shutdown: Shutdown) -> axum::Router {
        probes_router(heartbeat, Duration::from_secs(2), shutdown)
    }

    #[tokio::test(start_paused = true)]
    async fn livez_ok_while_heartbeat_fresh() {
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        let parts = get(router(&heartbeat, Shutdown::new()), "/livez").await;
        assert_eq!(parts.status, axum::http::StatusCode::OK);
        assert_eq!(
            parts.body,
            Some(json!({"status": "ok", "checks": {"heartbeat": "ok"}}))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn livez_unavailable_when_heartbeat_stale() {
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        tokio::time::advance(Duration::from_secs(31)).await;
        let parts = get(router(&heartbeat, Shutdown::new()), "/livez").await;
        assert_eq!(parts.status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            parts.body,
            Some(json!({"status": "unavailable", "checks": {"heartbeat": "failed"}}))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn readyz_ok_before_shutdown() {
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        let parts = get(router(&heartbeat, Shutdown::new()), "/readyz").await;
        assert_eq!(parts.status, axum::http::StatusCode::OK);
        assert_eq!(parts.body, Some(json!({"status": "ok", "checks": {}})));
    }

    #[tokio::test(start_paused = true)]
    async fn readyz_shutting_down_after_trigger() {
        let heartbeat = Heartbeat::new(Duration::from_secs(30));
        let shutdown = Shutdown::new();
        shutdown.trigger();
        let parts = get(router(&heartbeat, shutdown), "/readyz").await;
        assert_eq!(parts.status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            parts.body,
            Some(json!({"status": "shutting_down", "checks": {}}))
        );
    }
}
