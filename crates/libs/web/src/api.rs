//! The API process contract ([`crate::Api`], [`crate::ApiParts`]).

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use opentelemetry::metrics::MeterProvider;
use tokio::net::TcpListener;

use runtime::health::HealthCheck;
use runtime::lifecycle::RunOutcome;
use runtime::run_error::RunError;
use runtime::shutdown::Shutdown;
use runtime::supervisor::Supervisor;

use crate::config::ApiConfig;
use crate::health::HealthRouter;
use crate::server::{HTTP_SERVER_TASK, serve};
use crate::stack::with_api_stack;

/// The meter provider, shutdown handle, and configuration handed to an
/// [`Api::start`].
///
/// Clones share the underlying meter provider.
#[derive(Clone)]
pub struct ApiContext {
    meter_provider: Arc<dyn MeterProvider + Send + Sync>,
}

impl ApiContext {
    /// Creates a context from the meter provider for the API's
    /// instrumentation.
    #[must_use]
    pub fn new(meter_provider: Arc<dyn MeterProvider + Send + Sync>) -> Self {
        Self { meter_provider }
    }

    /// The meter provider to pass to instrumentation such as
    /// [`crate::metrics::with_http_metrics`].
    #[must_use]
    pub fn meter_provider(&self) -> &(dyn MeterProvider + Send + Sync) {
        self.meter_provider.as_ref()
    }
}

impl fmt::Debug for ApiContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiContext").finish_non_exhaustive()
    }
}

/// An API process hosted by the [`crate::launch()`] composition root:
/// [`Api::start`] connects
/// backends and builds the router, and the launch sequence binds, installs
/// signal handlers, and supervises the HTTP server.
///
/// Unlike a [`runtime::run_error::RunError`]-returning worker, an API has no
/// name of its own: the HTTP server task name is fixed.
#[async_trait]
pub trait Api: Send + 'static {
    /// Connects backends, runs migrations, and builds the router. Runs before
    /// shutdown signal handlers are installed, so a signal during startup
    /// keeps its default disposition.
    ///
    /// # Errors
    ///
    /// Returns a [`runtime::run_error::RunError`] classifying why startup
    /// failed; its classification becomes the process exit status.
    async fn start(self, context: &ApiContext) -> Result<ApiParts, RunError>;
}

/// What an [`Api::start`] hands to the launch sequence: the API router plus
/// the optional readiness checks and shutdown hooks.
pub struct ApiParts {
    router: Router,
    readiness: Vec<HealthCheck>,
    on_shutdown: Vec<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl ApiParts {
    /// Creates parts from the API's router.
    #[must_use]
    pub fn new(router: Router) -> Self {
        Self {
            router,
            readiness: Vec::new(),
            on_shutdown: Vec::new(),
        }
    }

    /// Adds a readiness check reported by `/readyz`.
    #[must_use]
    pub fn readiness(mut self, check: HealthCheck) -> Self {
        self.readiness.push(check);
        self
    }

    /// Adds a shutdown hook. Hooks run in registration order after the HTTP
    /// server stops, and are skipped when the shutdown deadline was exceeded
    /// (aborted tasks may still hold the resources).
    #[must_use]
    pub fn on_shutdown<F>(mut self, hook: F) -> Self
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.on_shutdown.push(Box::pin(hook));
        self
    }

    /// The production router: the API router behind the standard layer stack
    /// ([`crate::stack::with_api_stack`]), merged with the health probe
    /// router carrying this API's readiness checks, so probes bypass the
    /// stack. Usable by app tests to serve the same router the process does.
    pub fn router<P>(&self, config: &ApiConfig, shutdown: &Shutdown, meter_provider: &P) -> Router
    where
        P: MeterProvider + ?Sized,
    {
        let health = self
            .readiness
            .iter()
            .fold(
                HealthRouter::new(config.health_check_timeout(), shutdown.clone()),
                |router, check| router.readiness(check.clone()),
            )
            .into_router();
        with_api_stack(self.router.clone(), config.stack(), meter_provider).merge(health)
    }
}

impl fmt::Debug for ApiParts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let readiness = self
            .readiness
            .iter()
            .map(HealthCheck::name)
            .collect::<Vec<_>>();
        f.debug_struct("ApiParts")
            .field("readiness", &readiness)
            .field("on_shutdown", &self.on_shutdown.len())
            .finish_non_exhaustive()
    }
}

/// Serves the parts on `listener` under a [`runtime::supervisor::Supervisor`]
/// with `config.lifecycle`'s deadline, logs how the run stopped, runs the
/// shutdown hooks unless the deadline was exceeded, and returns the outcome.
pub(crate) async fn run<P>(
    parts: ApiParts,
    listener: TcpListener,
    config: &ApiConfig,
    meter_provider: &P,
    shutdown: Shutdown,
) -> RunOutcome
where
    P: MeterProvider + ?Sized,
{
    let lifecycle = &config.lifecycle;
    let router = parts.router(config, &shutdown, meter_provider);

    let mut supervisor = Supervisor::new(shutdown.clone());
    supervisor.spawn(
        HTTP_SERVER_TASK,
        serve(listener, router, shutdown, lifecycle.drain_delay()),
    );
    let supervised = supervisor.run(lifecycle.deadline()).await;

    let code = supervised.outcome.status.code();
    tracing::info!(
        status = ?supervised.outcome.status,
        code,
        culprit = ?supervised.culprit,
        deadline_exceeded = supervised.outcome.deadline_exceeded,
        "api stopped"
    );

    if !supervised.outcome.deadline_exceeded {
        for hook in parts.on_shutdown {
            hook.await;
        }
    }

    supervised.outcome
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::Duration;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use clap::Parser;
    use http_body_util::BodyExt;
    use opentelemetry::metrics::NoopMeterProvider;
    use runtime::exit_status::ExitStatus;
    use runtime::health::HealthCheck;
    use runtime::lifecycle::RunOutcome;
    use runtime::shutdown::Shutdown;
    use serde_json::Value;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpStream;
    use tokio::sync::Notify;
    use tokio::time::timeout;
    use tower::ServiceExt;

    use super::ApiContext;
    use super::ApiParts;
    use super::run;
    use crate::config::ApiConfig;
    use crate::request_id::REQUEST_ID_HEADER;

    fn request_get(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    async fn body_json(body: Body) -> Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn config(lifecycle_args: &[&str]) -> ApiConfig {
        let mut argv = vec!["test"];
        argv.extend_from_slice(lifecycle_args);
        ApiConfig::try_parse_from(argv).expect("test config parses")
    }

    fn lifecycle_args() -> Vec<&'static str> {
        vec![
            "--shutdown-drain-seconds",
            "0",
            "--shutdown-timeout-seconds",
            "1",
            "--shutdown-flush-timeout-seconds",
            "1",
        ]
    }

    #[tokio::test(start_paused = true)]
    async fn router_serves_the_api_behind_the_stack() {
        let parts = ApiParts::new(Router::new().route("/hello", get(|| async { "hi" })));
        let router = parts.router(&config(&[]), &Shutdown::new(), &NoopMeterProvider::new());

        let res = router.oneshot(request_get("/hello")).await.unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert!(
            res.headers().get(REQUEST_ID_HEADER).is_some(),
            "stacked response should carry {REQUEST_ID_HEADER}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn probes_bypass_the_stack() {
        let parts = ApiParts::new(Router::new());
        let router = parts.router(&config(&[]), &Shutdown::new(), &NoopMeterProvider::new());

        let res = router.oneshot(request_get("/readyz")).await.unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert!(
            res.headers().get(REQUEST_ID_HEADER).is_none(),
            "probes must bypass the stack"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_reports_the_parts_checks() {
        let parts = ApiParts::new(Router::new())
            .readiness(HealthCheck::new("down", || async { Err::<(), _>("boom") }));
        let router = parts.router(&config(&[]), &Shutdown::new(), &NoopMeterProvider::new());

        let res = router.oneshot(request_get("/readyz")).await.unwrap();

        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = body_json(res.into_body()).await;
        assert_eq!(body["status"], "unavailable");
        assert_eq!(body["checks"]["down"], "failed");
    }

    #[tokio::test(start_paused = true)]
    async fn run_succeeds_and_runs_hooks_after_shutdown() {
        let args = lifecycle_args();
        let config = config(&args);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("ephemeral listener binds");
        let shutdown = Shutdown::new();
        let order: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
        let parts = ApiParts::new(Router::new())
            .on_shutdown({
                let order = Arc::clone(&order);
                async move { order.lock().expect("order lock").push("first") }
            })
            .on_shutdown({
                let order = Arc::clone(&order);
                async move { order.lock().expect("order lock").push("second") }
            });

        let trigger = shutdown.clone();
        let trigger_task = tokio::spawn(async move {
            trigger.trigger();
        });

        let outcome = timeout(
            Duration::from_secs(60),
            run(
                parts,
                listener,
                &config,
                &NoopMeterProvider::new(),
                shutdown,
            ),
        )
        .await
        .expect("run completes under the test timeout");
        trigger_task.await.expect("trigger task does not panic");

        assert_eq!(outcome, RunOutcome::completed(ExitStatus::Success));
        assert_eq!(
            *order.lock().expect("order lock"),
            vec!["first", "second"],
            "hooks run in registration order"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn run_skips_hooks_when_the_deadline_passes() {
        let args = lifecycle_args();
        let config = config(&args);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("ephemeral listener binds");
        let address = listener.local_addr().expect("local address");
        let shutdown = Shutdown::new();
        let hook_ran = Arc::new(AtomicBool::new(false));
        let dispatched = Arc::new(Notify::new());
        let handler_dispatched = Arc::clone(&dispatched);
        let parts = ApiParts::new(Router::new().route(
            "/hang",
            get(move || {
                let dispatched = Arc::clone(&handler_dispatched);
                async move {
                    dispatched.notify_one();
                    std::future::pending::<()>().await;
                }
            }),
        ))
        .on_shutdown({
            let hook_ran = Arc::clone(&hook_ran);
            async move { hook_ran.store(true, Ordering::SeqCst) }
        });

        let trigger = shutdown.clone();
        let client = tokio::spawn(async move {
            let mut stream = TcpStream::connect(address)
                .await
                .expect("raw client connects");
            stream
                .write_all(b"GET /hang HTTP/1.1\r\nHost: test\r\n\r\n")
                .await
                .expect("raw client sends its request");
            stream.flush().await.expect("raw client flushes");
            dispatched.notified().await;
            trigger.trigger();
            std::future::pending::<()>().await;
        });

        let outcome = timeout(
            Duration::from_secs(60),
            run(
                parts,
                listener,
                &config,
                &NoopMeterProvider::new(),
                shutdown,
            ),
        )
        .await
        .expect("run completes under the test timeout");

        assert_eq!(outcome, RunOutcome::abandoned(ExitStatus::Interrupted));
        assert!(
            !hook_ran.load(Ordering::SeqCst),
            "shutdown hooks must be skipped when the deadline passes"
        );
        client.abort();
    }

    #[test]
    fn context_debug_omits_the_provider() {
        let context = ApiContext::new(Arc::new(NoopMeterProvider::new()));

        let debug = format!("{context:?}");

        assert!(debug.starts_with("ApiContext {"), "{debug}");
        assert!(debug.contains(".."), "{debug}");
    }
}
