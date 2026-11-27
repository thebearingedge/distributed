use std::{collections::BTreeMap, sync::Arc, time::Duration};

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use futures_util::future::join_all;
use runtime::health::HealthCheck;
use runtime::shutdown::Shutdown;
use serde::Serialize;

#[derive(Clone)]
struct HealthState {
    readiness: Arc<[HealthCheck]>,
    liveness: Arc<[HealthCheck]>,
    timeout: Duration,
    shutdown: Shutdown,
}

/// Which probe route a check run belongs to, for log attribution.
#[derive(Clone, Copy)]
enum Probe {
    Liveness,
    Readiness,
}

impl Probe {
    fn as_str(self) -> &'static str {
        match self {
            Probe::Liveness => "liveness",
            Probe::Readiness => "readiness",
        }
    }
}

/// Overall readiness of the service, reported as the `status` field of the
/// `/readyz` response body.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum ReadinessStatus {
    /// Every check passed.
    Ok,
    /// At least one check failed.
    Unavailable,
    /// A shutdown signal has been received; checks are skipped.
    ShuttingDown,
}

/// Overall liveness of the service, reported as the `status` field of the
/// `/livez` response body.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum LivenessStatus {
    /// Every check passed.
    Ok,
    /// At least one check failed.
    Unavailable,
}

/// Outcome of a single named health check, reported in the `checks` map.
#[derive(Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum CheckStatus {
    /// The check completed successfully within the timeout.
    Ok,
    /// The check failed or timed out.
    Failed,
}

#[derive(Serialize)]
struct Readiness {
    status: ReadinessStatus,
    checks: BTreeMap<&'static str, CheckStatus>,
}

#[derive(Serialize)]
struct Liveness {
    status: LivenessStatus,
    checks: BTreeMap<&'static str, CheckStatus>,
}

/// Builder for the health-probe router.
///
/// Exposes two routes:
///
/// - `/livez` — liveness; runs every liveness
///   [`runtime::health::HealthCheck`] concurrently, each bounded by
///   `timeout`, and ignores the shutdown state because a draining process is
///   still alive. Returns `200 OK` with
///   `{"status": "ok", "checks": {...}}` when every check passes (including
///   when there are none), or `503` with `{"status": "unavailable", ...}`
///   and a per-check `ok`/`failed` map (timed-out checks report `"failed"`)
///   when any check fails.
/// - `/readyz` — readiness; once the shutdown is triggered, returns `503`
///   with `{"status": "shutting_down", "checks": {}}` and skips the checks
///   entirely. Otherwise it runs every readiness
///   [`runtime::health::HealthCheck`] as above.
#[derive(Clone)]
pub struct HealthRouter {
    readiness: Vec<HealthCheck>,
    liveness: Vec<HealthCheck>,
    timeout: Duration,
    shutdown: Shutdown,
}

impl HealthRouter {
    /// A router builder with no checks, probing against `shutdown`.
    #[must_use]
    pub fn new(timeout: Duration, shutdown: Shutdown) -> Self {
        Self {
            readiness: Vec::new(),
            liveness: Vec::new(),
            timeout,
            shutdown,
        }
    }

    /// Registers a readiness check for `/readyz`.
    #[must_use]
    pub fn readiness(mut self, check: HealthCheck) -> Self {
        self.readiness.push(check);
        self
    }

    /// Registers a liveness check for `/livez`.
    #[must_use]
    pub fn liveness(mut self, check: HealthCheck) -> Self {
        self.liveness.push(check);
        self
    }

    /// Builds the axum router with both probe routes.
    pub fn into_router(self) -> Router {
        let state = HealthState {
            readiness: self.readiness.into(),
            liveness: self.liveness.into(),
            timeout: self.timeout,
            shutdown: self.shutdown,
        };
        Router::new()
            .route("/livez", get(livez))
            .route("/readyz", get(readyz))
            .with_state(state)
    }
}

async fn readyz(State(state): State<HealthState>) -> (StatusCode, Json<Readiness>) {
    if state.shutdown.is_triggered() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(Readiness {
                status: ReadinessStatus::ShuttingDown,
                checks: BTreeMap::new(),
            }),
        );
    }

    let checks = run_checks(&state.readiness, state.timeout, Probe::Readiness).await;
    let ready = checks.values().all(|status| *status == CheckStatus::Ok);
    let (code, status) = if ready {
        (StatusCode::OK, ReadinessStatus::Ok)
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            ReadinessStatus::Unavailable,
        )
    };
    (code, Json(Readiness { status, checks }))
}

async fn livez(State(state): State<HealthState>) -> (StatusCode, Json<Liveness>) {
    let checks = run_checks(&state.liveness, state.timeout, Probe::Liveness).await;
    let alive = checks.values().all(|status| *status == CheckStatus::Ok);
    let (code, status) = if alive {
        (StatusCode::OK, LivenessStatus::Ok)
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, LivenessStatus::Unavailable)
    };
    (code, Json(Liveness { status, checks }))
}

async fn run_checks(
    checks: &[HealthCheck],
    timeout: Duration,
    probe: Probe,
) -> BTreeMap<&'static str, CheckStatus> {
    let outcomes = join_all(checks.iter().map(|check| async move {
        let result = match tokio::time::timeout(timeout, check.run()).await {
            Ok(result) => result,
            Err(_) => Err("timed out".to_owned()),
        };
        (check.name(), result)
    }))
    .await;

    let mut statuses = BTreeMap::new();
    for (name, result) in outcomes {
        if let Err(error) = &result {
            tracing::warn!(
                check = name,
                probe = probe.as_str(),
                %error,
                "health check failed"
            );
        }
        statuses.insert(
            name,
            match result {
                Ok(()) => CheckStatus::Ok,
                Err(_) => CheckStatus::Failed,
            },
        );
    }
    statuses
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use runtime::shutdown::Shutdown;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::{self as health};

    fn get(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    async fn body_json(body: Body) -> Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn readyz_is_ok_when_every_check_passes() {
        let router = health::HealthRouter::new(Duration::from_secs(2), Shutdown::new())
            .readiness(health::HealthCheck::new("up", || async {
                Ok::<(), String>(())
            }))
            .into_router();
        let res = router.oneshot(get("/readyz")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test(start_paused = true)]
    async fn readyz_is_503_when_a_check_fails_but_livez_stays_ok() {
        let router = health::HealthRouter::new(Duration::from_secs(2), Shutdown::new())
            .readiness(health::HealthCheck::new("down", || async {
                Err::<(), _>("boom")
            }))
            .into_router();
        let res = router.clone().oneshot(get("/readyz")).await.unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let res = router.oneshot(get("/livez")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test(start_paused = true)]
    async fn readyz_is_503_shutting_down_while_draining() {
        let shutdown = Shutdown::new();
        let router =
            health::HealthRouter::new(Duration::from_secs(2), shutdown.clone()).into_router();
        shutdown.trigger();
        let res = router.oneshot(get("/readyz")).await.unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = body_json(res.into_body()).await;
        assert_eq!(body["status"], "shutting_down");
        assert_eq!(body["checks"], serde_json::json!({}));
    }

    #[tokio::test(start_paused = true)]
    async fn livez_stays_ok_while_draining() {
        let shutdown = Shutdown::new();
        let router =
            health::HealthRouter::new(Duration::from_secs(2), shutdown.clone()).into_router();
        shutdown.trigger();
        let res = router.oneshot(get("/livez")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test(start_paused = true)]
    async fn draining_skips_the_checks_entirely() {
        let ran = Arc::new(AtomicBool::default());
        let ran_in_check = Arc::clone(&ran);
        let shutdown = Shutdown::new();
        let router = health::HealthRouter::new(Duration::from_secs(2), shutdown.clone())
            .readiness(health::HealthCheck::new("down", move || {
                ran_in_check.store(true, Ordering::Relaxed);
                async { Err::<(), _>("boom") }
            }))
            .into_router();
        shutdown.trigger();
        let res = router.oneshot(get("/readyz")).await.unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = body_json(res.into_body()).await;
        assert_eq!(body["status"], "shutting_down");
        assert!(!ran.load(Ordering::Relaxed));
    }

    #[tokio::test(start_paused = true)]
    async fn readyz_reports_a_timed_out_check_as_failed() {
        let router = health::HealthRouter::new(Duration::from_secs(2), Shutdown::new())
            .readiness(health::HealthCheck::new("slow", || async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                Ok::<(), String>(())
            }))
            .into_router();
        let res = router.oneshot(get("/readyz")).await.unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = body_json(res.into_body()).await;
        assert_eq!(body["status"], "unavailable");
        assert_eq!(body["checks"]["slow"], "failed");
    }

    #[tokio::test(start_paused = true)]
    async fn livez_is_ok_with_no_checks() {
        let router =
            health::HealthRouter::new(Duration::from_secs(2), Shutdown::new()).into_router();
        let res = router.oneshot(get("/livez")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = body_json(res.into_body()).await;
        assert_eq!(body["status"], "ok");
        assert_eq!(body["checks"], serde_json::json!({}));
    }

    #[tokio::test(start_paused = true)]
    async fn livez_is_unavailable_when_a_liveness_check_fails() {
        let router = health::HealthRouter::new(Duration::from_secs(2), Shutdown::new())
            .liveness(health::HealthCheck::new("down", || async {
                Err::<(), _>("boom")
            }))
            .into_router();
        let res = router.oneshot(get("/livez")).await.unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = body_json(res.into_body()).await;
        assert_eq!(body["status"], "unavailable");
        assert_eq!(body["checks"]["down"], "failed");
    }

    #[tokio::test(start_paused = true)]
    async fn livez_reports_a_timed_out_check_as_failed() {
        let router = health::HealthRouter::new(Duration::from_secs(2), Shutdown::new())
            .liveness(health::HealthCheck::new("slow", || async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                Ok::<(), String>(())
            }))
            .into_router();
        let res = router.oneshot(get("/livez")).await.unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = body_json(res.into_body()).await;
        assert_eq!(body["status"], "unavailable");
        assert_eq!(body["checks"]["slow"], "failed");
    }

    #[tokio::test(start_paused = true)]
    async fn livez_stays_ok_while_shutting_down() {
        let shutdown = Shutdown::new();
        let router = health::HealthRouter::new(Duration::from_secs(2), shutdown.clone())
            .liveness(health::HealthCheck::new("up", || async {
                Ok::<(), String>(())
            }))
            .into_router();
        shutdown.trigger();
        let res = router.oneshot(get("/livez")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = body_json(res.into_body()).await;
        assert_eq!(body["status"], "ok");
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_failures_do_not_affect_livez() {
        let router = health::HealthRouter::new(Duration::from_secs(2), Shutdown::new())
            .readiness(health::HealthCheck::new("down", || async {
                Err::<(), _>("boom")
            }))
            .into_router();
        let res = router.clone().oneshot(get("/readyz")).await.unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let res = router.oneshot(get("/livez")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = body_json(res.into_body()).await;
        assert_eq!(body["status"], "ok");
    }

    #[tokio::test(start_paused = true)]
    async fn liveness_failures_do_not_affect_readyz() {
        let router = health::HealthRouter::new(Duration::from_secs(2), Shutdown::new())
            .liveness(health::HealthCheck::new("down", || async {
                Err::<(), _>("boom")
            }))
            .into_router();
        let res = router.clone().oneshot(get("/livez")).await.unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let res = router.oneshot(get("/readyz")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = body_json(res.into_body()).await;
        assert_eq!(body["status"], "ok");
    }
}
