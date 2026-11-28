# web

Shared axum web plumbing for HTTP APIs: the standard cross-cutting layer
stack ([`with_api_stack`](src/stack.rs)), the API process contract
([`Api`](src/api.rs)), the shared process runner
([`launch`](src/launch.rs) / [`launch_with`](src/launch.rs)), JSON-validated
extractors, HTTP error bodies, and the CLI/env configuration for API
binaries ([`ApiConfig`](src/config.rs)). The crate depends on `runtime`,
`telemetry`, and the `opentelemetry` API crate, never on a domain or on
`worker`/`database`.

## Contents

- [`Api` / `ApiParts` / `ApiContext`](src/api.rs) — the API process
  contract: `start(context)` connects backends, runs migrations, and builds
  the router, handing back an `ApiParts` — the router plus optional
  readiness and liveness checks, supervised tasks, and shutdown hooks. The
  context carries the meter provider for instrumentation
  (`context.meter_provider()`) and the shutdown handle (`context.shutdown()`)
  that `.task` tasks observe. An API has no name of its own: the HTTP server
  task name is fixed.
- [`launch`](src/launch.rs) — process composition root: initializes
  telemetry, runs `Api::start`, binds the listener, installs shutdown signal
  handlers immediately before supervision (a second signal exits 143
  immediately), supervises the HTTP server and the API's tasks on a
  `runtime::supervisor::Supervisor` under the shutdown deadline, runs the
  shutdown hooks, flushes telemetry, and returns the process exit code.
  [`launch_with`](src/launch.rs) is the same runner taking an already
  composed router, a pre-built `ApiParts`, and a [`Label`](src/launch.rs)
  (`Api` or `Worker(&'static str)`) that names the log messages and the
  server task; `worker::launch` is a thin client of it, so the API and
  worker shapes share one supervision path.
- [`with_api_stack`](src/stack.rs) — the standard layer stack, encoding the
  required order once. Layers, outermost first:
  `with_http_metrics` → `with_cors` → `with_http_tracing` →
  `with_guardrails`. Health probes must be merged after it so they bypass
  the stack.
- [`with_json_fallbacks`](src/fallback.rs) — replaces axum's empty 404 and
  405 responses with the standard JSON error body, for APIs that build
  fallback routes of their own.
- [`ApiParts::probes`](src/api.rs) — the `/livez` and `/readyz` probe
  router carrying the parts' liveness and readiness checks; the checks
  themselves are `runtime::health::HealthCheck` values supplied by the app
  (e.g. `database::health::check(&pool)`).
- [`ApiConfig`](src/config.rs) — CLI/env configuration for an API binary;
  it flattens `runtime::config::LifecycleConfig`, and its fields feed
  [`with_api_stack`](src/stack.rs) directly (the request guardrails and
  CORS policy are derived inside the crate).
- [`AllowedOrigins`](src/cors.rs) — exact origins parsed from a
  comma-separated value at the configuration edge; the wildcard `*` is
  rejected, and empty input means CORS is off.
- [Extractors](src/extractors.rs) — `ValidatedJson`, `ValidatedQuery`,
  `ValidatedPath`, and `OptionalHeader`, validated against JSON schemas.
- [Error bodies](src/api_error.rs) — the `ErrorBody` envelope (with
  `requestId`), `ValidationErrorBody`, and the `ApiError` response wrapper.
- [`test_support`](src/test_support.rs) — oneshot HTTP test helpers
  (`get`, `post_json`, `put_json`, `post_body`), compiled only for tests or
  when the `test-support` feature is enabled.

## Probes

- `GET /livez` — `200` with `{"status":"ok","checks":{...}}` while every
  liveness check passes; `503` with `status: "unavailable"` when one fails
  or exceeds `HEALTH_CHECK_TIMEOUT_SECONDS`. The router carries no checks
  unless the app adds them, and it is unaffected by shutdown.
- `GET /readyz` — `200` while every readiness check passes; `503` with
  `status: "unavailable"` and the failed checks named; `503` with
  `{"status":"shutting_down","checks":{}}` after SIGTERM. While draining,
  the checks are skipped entirely.

Probe routes are merged after the stack, so they bypass metrics, CORS,
tracing, and guardrails.

## Configuration

All of these come from [`ApiConfig`](src/config.rs), which the app's
bin-private `Args` flattens. CLI flags (`--listen-address`,
`--health-check-timeout-seconds`, `--cors-allowed-origins`,
`--request-timeout-seconds`, `--request-body-limit-bytes`, and the shared
`--shutdown-*` flags) override env vars, which override these
defaults.

| Env var | Default | Meaning |
|---|---|---|
| `LISTEN_ADDRESS` | `0.0.0.0:3000` | Address and port the HTTP listener binds to. |
| `HEALTH_CHECK_TIMEOUT_SECONDS` | `2` | Seconds each liveness or readiness check may take before it counts as failed. |
| `CORS_ALLOWED_ORIGINS` | unset | Comma-separated exact origins allowed to make cross-origin requests; entries are trimmed and must not end with a slash; the wildcard `*` is not supported. Unset or empty means CORS is off. |
| `REQUEST_TIMEOUT_SECONDS` | `30` | Seconds a request may take to produce a response before it is answered with `503 Service Unavailable`. |
| `REQUEST_BODY_LIMIT_BYTES` | `1048576` | Maximum request body size in bytes; larger bodies are rejected with `413 Payload Too Large`. |

The shared `SHUTDOWN_*` vars and the shutdown budget are in the
[runtime README](../runtime/README.md#configuration).

## Composing an API

The API value lives in the app's lib target; `main.rs` is parse → construct
→ launch:

```rust
// crates/apps/api/example-api/src/lib.rs
use async_trait::async_trait;
use axum::Router;

use runtime::run_error::RunError;
use web::{Api, ApiContext, ApiParts};

/// The example API process: builds its router behind the standard stack.
#[derive(Debug)]
pub struct ExampleApi;

#[async_trait]
impl Api for ExampleApi {
    async fn start(self, _context: &ApiContext) -> Result<ApiParts, RunError> {
        let router = Router::new().route("/", axum::routing::get(|| async { "hi" }));

        Ok(ApiParts::new(router))
    }
}
```

A backend-backed API follows the `todos-api` shape — connect, observe the
pool, run migrations, and hand back the router with a readiness check and a
shutdown hook:

```rust
#[async_trait]
impl Api for TodosApi {
    async fn start(self, context: &ApiContext) -> Result<ApiParts, RunError> {
        let pool = self.db.connect().await.map_err(RunError::retryable)?;

        database::metrics::observe_pool(&pool, "todos", context.meter_provider());

        MIGRATOR.run(&pool).await.map_err(RunError::retryable)?;

        Ok(ApiParts::new(create_todos_api(AppState::new(pool.clone())))
            .readiness(database::health::check(&pool))
            .on_shutdown(async move { pool.close().await }))
    }
}
```

The layer stack is applied by `ApiParts::router` (via
[`with_api_stack`](src/stack.rs)), so `start` returns the bare API router.

```rust
// crates/apps/api/example-api/src/main.rs
use std::process::ExitCode;

use clap::Parser;

use config::Args;
use example_api::ExampleApi;

mod config;

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    web::launch("example_api", ExampleApi, &args.api).await
}
```

where `src/config.rs` is the bin-private `Args` that flattens
[`ApiConfig`](src/config.rs) plus whatever other lib configs the app uses.

## Startup sequence

[`launch`](src/launch.rs) runs, in order:

1. Initialize telemetry.
2. [`Api::start`](src/api.rs) — connect backends, run migrations, build the
   router. Failures return a classified `runtime::run_error::RunError`.
3. Bind the listener from `LISTEN_ADDRESS`.
4. Install shutdown signal handlers immediately before supervision. This is
   the one signal rule for every process shape: a `SIGTERM` before
   supervision — during `start`, binding, or any other startup — keeps its
   default disposition.
5. Supervise the HTTP server (`http_server`) and every `ApiParts::task` on
   a `runtime::supervisor::Supervisor` under the shutdown deadline; a task
   stopping first (error, panic, or a clean return before shutdown) becomes
   the culprit. Then run the shutdown hooks and flush telemetry.

## Exit statuses

Exit codes follow the [runtime taxonomy](../runtime/README.md#exit-codes).
For an API, exit 1 also covers a startup failure classified retryable, a
bind or signal-install failure, an HTTP server I/O error, or a panic (a
Deployment restarts the pod). Exit 2 covers a startup `RunError` classified
permanent.

Every startup failure is logged with `tracing::error!` and ends through
`runtime::lifecycle::finish`, so telemetry is flushed on every path.

## Kubernetes

Probes are served only after [`Api::start`](src/api.rs) finishes, because
the probe router is merged into the server's router once startup is
complete. A `startupProbe` must therefore cover the whole startup:
connecting to Postgres plus running migrations. Set its `failureThreshold`
× `periodSeconds` above the worst-case migration time.

Migrations run at API startup. sqlx's `Migrator` serializes concurrent
replicas with a Postgres advisory lock, so two replicas cannot apply the
same migration at once, but every replica must be able to acquire it: the
app's database role needs DDL privileges, and rolling updates need
expand/contract migrations so both the old and the new pod can serve against
either schema revision.
