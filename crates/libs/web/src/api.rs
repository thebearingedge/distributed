//! The API process contract ([`crate::Api`], [`crate::ApiParts`]).

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use opentelemetry::metrics::MeterProvider;
use tokio::net::TcpListener;

use runtime::config::LifecycleConfig;
use runtime::health::HealthCheck;
use runtime::run_error::RunError;
use runtime::shutdown::Shutdown;
use runtime::supervisor::{Supervised, Supervisor};

use crate::config::ApiConfig;
use crate::health::HealthRouter;
use crate::launch::Label;
use crate::server::serve;
use crate::stack::with_api_stack;

/// The shutdown handle and meter provider handed to an [`Api::start`].
///
/// Clones share the underlying meter provider.
#[derive(Clone)]
pub struct ApiContext {
    shutdown: Shutdown,
    meter_provider: Arc<dyn MeterProvider + Send + Sync>,
}

impl ApiContext {
    /// Creates a context from the process shutdown and the meter provider for
    /// the API's instrumentation.
    #[must_use]
    pub fn new(shutdown: Shutdown, meter_provider: Arc<dyn MeterProvider + Send + Sync>) -> Self {
        Self {
            shutdown,
            meter_provider,
        }
    }

    /// The process shutdown, triggered by an OS signal or by a supervised
    /// task stopping; tasks added with [`ApiParts::task`] observe it.
    #[must_use]
    pub fn shutdown(&self) -> &Shutdown {
        &self.shutdown
    }

    /// The meter provider to pass to instrumentation such as HTTP or pool
    /// metrics.
    #[must_use]
    pub fn meter_provider(&self) -> &(dyn MeterProvider + Send + Sync) {
        self.meter_provider.as_ref()
    }
}

impl fmt::Debug for ApiContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiContext")
            .field("shutdown", &self.shutdown)
            .finish_non_exhaustive()
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

/// A supervised task future boxed for storage in [`ApiParts`].
type SupervisedTask = Pin<Box<dyn Future<Output = Result<(), RunError>> + Send>>;

/// What an [`Api::start`] hands to the launch sequence: the API router plus
/// the optional readiness checks, liveness checks, supervised tasks, and
/// shutdown hooks.
pub struct ApiParts {
    router: Router,
    readiness: Vec<HealthCheck>,
    liveness: Vec<HealthCheck>,
    tasks: Vec<(&'static str, SupervisedTask)>,
    on_shutdown: Vec<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl ApiParts {
    /// Creates parts from the API's router.
    #[must_use]
    pub fn new(router: Router) -> Self {
        Self {
            router,
            readiness: Vec::new(),
            liveness: Vec::new(),
            tasks: Vec::new(),
            on_shutdown: Vec::new(),
        }
    }

    /// Adds a readiness check reported by `/readyz`.
    #[must_use]
    pub fn readiness(mut self, check: HealthCheck) -> Self {
        self.readiness.push(check);
        self
    }

    /// Adds a liveness check reported by `/livez`.
    #[must_use]
    pub fn liveness(mut self, check: HealthCheck) -> Self {
        self.liveness.push(check);
        self
    }

    /// Adds a task supervised alongside the HTTP server under `name`. It must
    /// run until the process shutdown (see [`ApiContext::shutdown`]) is
    /// triggered; stopping earlier — with an error, a panic, or a clean
    /// return — triggers the shutdown and makes `name` the culprit.
    #[must_use]
    pub fn task<F>(mut self, name: &'static str, task: F) -> Self
    where
        F: Future<Output = Result<(), RunError>> + Send + 'static,
    {
        self.tasks.push((name, Box::pin(task)));
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
    /// router carrying this API's readiness and liveness checks, so probes
    /// bypass the stack. Usable by app tests to serve the same router the
    /// process does.
    pub fn router<P>(&self, config: &ApiConfig, shutdown: &Shutdown, meter_provider: &P) -> Router
    where
        P: MeterProvider + ?Sized,
    {
        with_api_stack(self.router.clone(), config, meter_provider)
            .merge(self.probes(config.health_check_timeout(), shutdown))
    }

    /// The `/livez` + `/readyz` probe router carrying this part's liveness
    /// and readiness checks, without the API router or the layer stack.
    pub fn probes(&self, health_check_timeout: Duration, shutdown: &Shutdown) -> Router {
        let health = self.readiness.iter().fold(
            HealthRouter::new(health_check_timeout, shutdown.clone()),
            |router, check| router.readiness(check.clone()),
        );
        self.liveness
            .iter()
            .fold(health, |router, check| router.liveness(check.clone()))
            .into_router()
    }
}

impl fmt::Debug for ApiParts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let readiness = self
            .readiness
            .iter()
            .map(HealthCheck::name)
            .collect::<Vec<_>>();
        let liveness = self
            .liveness
            .iter()
            .map(HealthCheck::name)
            .collect::<Vec<_>>();
        let tasks = self.tasks.iter().map(|(name, _)| *name).collect::<Vec<_>>();
        f.debug_struct("ApiParts")
            .field("readiness", &readiness)
            .field("liveness", &liveness)
            .field("tasks", &tasks)
            .field("on_shutdown", &self.on_shutdown.len())
            .finish_non_exhaustive()
    }
}

/// Serves `router` on `listener` and supervises `parts`' tasks under a
/// [`runtime::supervisor::Supervisor`] with `lifecycle`'s deadline, logs how
/// the run stopped under `label`, runs the shutdown hooks unless the
/// deadline was exceeded, and returns how the run ended.
pub(crate) async fn run(
    router: Router,
    parts: ApiParts,
    listener: TcpListener,
    shutdown: Shutdown,
    lifecycle: &LifecycleConfig,
    label: Label,
) -> Supervised {
    let mut supervisor = Supervisor::new(shutdown.clone());
    supervisor.spawn(
        label.server_task(),
        serve(listener, router, shutdown, lifecycle.drain_delay()),
    );
    for (name, task) in parts.tasks {
        supervisor.spawn(name, task);
    }

    let supervised = supervisor.run(lifecycle.deadline()).await;

    let noun = label.noun();
    tracing::info!(
        worker = label.worker(),
        status = ?supervised.outcome.status,
        code = supervised.outcome.status.code(),
        culprit = ?supervised.culprit,
        deadline_exceeded = supervised.outcome.deadline_exceeded,
        "{noun} stopped"
    );

    if !supervised.outcome.deadline_exceeded {
        for hook in parts.on_shutdown {
            hook.await;
        }
    }

    supervised
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::Duration;

    use axum::Router;
    use axum::http::StatusCode;
    use axum::routing::get;
    use clap::Parser;
    use opentelemetry::metrics::NoopMeterProvider;
    use runtime::config::LifecycleConfig;
    use runtime::exit_status::ExitStatus;
    use runtime::health::HealthCheck;
    use runtime::lifecycle::RunOutcome;
    use runtime::run_error::RunError;
    use runtime::shutdown::Shutdown;
    use runtime::supervisor::Supervised;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpStream;
    use tokio::sync::Notify;
    use tokio::time::timeout;

    use super::{ApiContext, ApiParts, run};
    use crate::config::ApiConfig;
    use crate::launch::Label;
    use crate::request_id::REQUEST_ID_HEADER;
    use crate::test_support;

    fn config() -> ApiConfig {
        ApiConfig::try_parse_from(["test"]).expect("test config parses")
    }

    fn lifecycle() -> LifecycleConfig {
        LifecycleConfig::try_parse_from([
            "test",
            "--shutdown-drain-seconds",
            "0",
            "--shutdown-timeout-seconds",
            "1",
            "--shutdown-flush-timeout-seconds",
            "1",
        ])
        .expect("test lifecycle parses")
    }

    async fn listener() -> tokio::net::TcpListener {
        tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("ephemeral listener binds")
    }

    #[tokio::test(start_paused = true)]
    async fn router_serves_the_api_behind_the_stack() {
        let parts = ApiParts::new(Router::new().route("/hello", get(|| async { "hi" })));
        let router = parts.router(&config(), &Shutdown::new(), &NoopMeterProvider::new());

        let res = test_support::get(router, "/hello").await;

        assert_eq!(res.status, StatusCode::OK);
        assert!(
            res.header(REQUEST_ID_HEADER.as_str()).is_some(),
            "stacked response should carry {REQUEST_ID_HEADER}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn probes_bypass_the_stack() {
        let parts = ApiParts::new(Router::new());
        let router = parts.router(&config(), &Shutdown::new(), &NoopMeterProvider::new());

        let res = test_support::get(router, "/readyz").await;

        assert_eq!(res.status, StatusCode::OK);
        assert!(
            res.header(REQUEST_ID_HEADER.as_str()).is_none(),
            "probes must bypass the stack"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_reports_the_parts_checks() {
        let parts = ApiParts::new(Router::new()).readiness(HealthCheck::with_resource(
            "down",
            (),
            |()| async { Err::<(), _>("boom") },
        ));
        let router = parts.router(&config(), &Shutdown::new(), &NoopMeterProvider::new());

        let res = test_support::get(router, "/readyz").await;

        assert_eq!(res.status, StatusCode::SERVICE_UNAVAILABLE);
        let body = res.body.expect("the readyz body is JSON");
        assert_eq!(body["status"], "unavailable");
        assert_eq!(body["checks"]["down"], "failed");
    }

    #[tokio::test(start_paused = true)]
    async fn run_succeeds_and_runs_hooks_after_shutdown() {
        let listener = listener().await;
        let shutdown = Shutdown::new();
        let context = ApiContext::new(shutdown.clone(), Arc::new(NoopMeterProvider::new()));
        let order: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
        let parts = ApiParts::new(Router::new())
            .task("steady", {
                let shutdown = context.shutdown().clone();
                async move {
                    shutdown.triggered().await;
                    Ok::<(), RunError>(())
                }
            })
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

        let supervised = timeout(
            Duration::from_secs(60),
            run(
                Router::new(),
                parts,
                listener,
                shutdown,
                &lifecycle(),
                Label::Api,
            ),
        )
        .await
        .expect("run completes under the test timeout");
        trigger_task.await.expect("trigger task does not panic");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::completed(ExitStatus::Success),
                culprit: None,
            }
        );
        assert_eq!(
            *order.lock().expect("order lock"),
            vec!["first", "second"],
            "hooks run in registration order"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn run_skips_hooks_when_the_deadline_passes() {
        let listener = listener().await;
        let address = listener.local_addr().expect("local address");
        let shutdown = Shutdown::new();
        let hook_ran = Arc::new(AtomicBool::new(false));
        let dispatched = Arc::new(Notify::new());
        let handler_dispatched = Arc::clone(&dispatched);
        let router = Router::new().route(
            "/hang",
            get(move || {
                let dispatched = Arc::clone(&handler_dispatched);
                async move {
                    dispatched.notify_one();
                    std::future::pending::<()>().await;
                }
            }),
        );
        let parts = ApiParts::new(Router::new()).on_shutdown({
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

        let supervised = run(router, parts, listener, shutdown, &lifecycle(), Label::Api).await;

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::abandoned(ExitStatus::Interrupted),
                culprit: None,
            }
        );
        assert!(
            !hook_ran.load(Ordering::SeqCst),
            "shutdown hooks must be skipped when the deadline passes"
        );
        client.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn run_reports_a_task_returning_early_as_the_retryable_culprit() {
        let listener = listener().await;
        let shutdown = Shutdown::new();
        let parts = ApiParts::new(Router::new()).task("early", async { Ok::<(), RunError>(()) });

        let supervised = timeout(
            Duration::from_secs(60),
            run(
                Router::new(),
                parts,
                listener,
                shutdown.clone(),
                &lifecycle(),
                Label::Worker("w"),
            ),
        )
        .await
        .expect("run completes under the test timeout");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::completed(ExitStatus::Retryable),
                culprit: Some("early"),
            }
        );
        assert!(
            shutdown.is_triggered(),
            "a task stopping early must trigger the shutdown"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn run_reports_a_task_panic_as_the_retryable_culprit() {
        let listener = listener().await;
        let shutdown = Shutdown::new();
        let parts = ApiParts::new(Router::new()).task("exploding", async { panic!("boom") });

        let supervised = timeout(
            Duration::from_secs(60),
            run(
                Router::new(),
                parts,
                listener,
                shutdown,
                &lifecycle(),
                Label::Worker("w"),
            ),
        )
        .await
        .expect("run completes under the test timeout");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::completed(ExitStatus::Retryable),
                culprit: Some("exploding"),
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn run_maps_a_task_error_and_still_runs_hooks() {
        let listener = listener().await;
        let shutdown = Shutdown::new();
        let hook_ran = Arc::new(AtomicBool::new(false));
        let parts = ApiParts::new(Router::new())
            .task("failing", async { Err(RunError::permanent("bad config")) })
            .on_shutdown({
                let hook_ran = Arc::clone(&hook_ran);
                async move { hook_ran.store(true, Ordering::SeqCst) }
            });

        let supervised = timeout(
            Duration::from_secs(60),
            run(
                Router::new(),
                parts,
                listener,
                shutdown,
                &lifecycle(),
                Label::Worker("w"),
            ),
        )
        .await
        .expect("run completes under the test timeout");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::completed(ExitStatus::Permanent),
                culprit: Some("failing"),
            }
        );
        assert!(
            hook_ran.load(Ordering::SeqCst),
            "a task failure is not a deadline overrun, so hooks still run"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn liveness_reports_the_parts_checks() {
        let parts = ApiParts::new(Router::new()).liveness(HealthCheck::with_resource(
            "down",
            (),
            |()| async { Err::<(), _>("boom") },
        ));
        let router = parts.router(&config(), &Shutdown::new(), &NoopMeterProvider::new());

        let livez = test_support::get(router.clone(), "/livez").await;
        assert_eq!(livez.status, StatusCode::SERVICE_UNAVAILABLE);
        let body = livez.body.expect("the livez body is JSON");
        assert_eq!(body["status"], "unavailable");
        assert_eq!(body["checks"]["down"], "failed");

        let readyz = test_support::get(router, "/readyz").await;
        assert_eq!(readyz.status, StatusCode::OK);
    }

    #[tokio::test(start_paused = true)]
    async fn probes_serve_the_checks_without_the_api_router() {
        let shutdown = Shutdown::new();
        let parts = ApiParts::new(Router::new().route("/hello", get(|| async { "hi" }))).liveness(
            HealthCheck::with_resource("heartbeat", (), |()| async { Ok::<(), &str>(()) }),
        );
        let router = parts.probes(Duration::from_secs(2), &shutdown);

        let livez = test_support::get(router.clone(), "/livez").await;
        assert_eq!(livez.status, StatusCode::OK);
        let body = livez.body.expect("the livez body is JSON");
        assert_eq!(body["checks"]["heartbeat"], "ok");

        let hello = test_support::get(router, "/hello").await;
        assert_eq!(hello.status, StatusCode::NOT_FOUND);
        assert!(
            hello.header(REQUEST_ID_HEADER.as_str()).is_none(),
            "probes must not carry {REQUEST_ID_HEADER}"
        );
    }

    #[test]
    fn context_debug_omits_the_provider() {
        let context = ApiContext::new(Shutdown::new(), Arc::new(NoopMeterProvider::new()));

        let debug = format!("{context:?}");

        assert!(debug.starts_with("ApiContext {"), "{debug}");
        assert!(debug.contains("shutdown"), "{debug}");
        assert!(debug.contains(".."), "{debug}");
    }
}
