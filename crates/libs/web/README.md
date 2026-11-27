# web

Shared axum web plumbing for HTTP APIs: the standard cross-cutting layer
stack ([`with_api_stack`](src/stack.rs)), the API process contract
([`Api`](src/api.rs)) with its [`launch`](src/launch.rs) composition root,
the `/livez` and `/readyz` probe router ([`HealthRouter`](src/health.rs)),
the draining HTTP server ([`serve`](src/server.rs)), JSON-validated
extractors, HTTP error bodies, and the CLI/env configuration for API
binaries ([`ApiConfig`](src/config.rs)). The crate depends on `runtime`,
`telemetry`, and the `opentelemetry` API crate, never on a domain or on
`worker`/`database`.

## Contents

- [`Api` / `ApiParts` / `ApiContext`](src/api.rs) — the API process
  contract: `start(context)` connects backends, runs migrations, and builds
  the router, handing back an `ApiParts` — the router plus optional
  readiness checks and shutdown hooks. The context carries the meter
  provider for instrumentation (`context.meter_provider()`). An API has no
  name of its own: the HTTP server task name is fixed.
- [`launch`](src/launch.rs) — process composition root: initializes
  telemetry, runs `Api::start`, binds the listener, installs shutdown signal
  handlers (a second signal exits 143 immediately), supervises the HTTP
  server on a `runtime::supervisor::Supervisor` under the shutdown deadline,
  runs the shutdown hooks, flushes telemetry, and returns the process exit
  code.
- [`with_api_stack`](src/stack.rs) — the standard layer stack, encoding the
  required order once. Layers, outermost first:
  `with_http_metrics` → `with_cors` → `with_http_tracing` →
  `with_guardrails`. Health probes must be merged after it so they bypass
  the stack.
- The individual layer functions —
  [`with_http_metrics`](src/metrics.rs), [`with_cors`](src/cors.rs),
  [`with_http_tracing`](src/middleware.rs),
  [`with_guardrails`](src/guardrails.rs),
  [`with_json_fallbacks`](src/fallback.rs) — stay public escape hatches for
  bespoke stacks.
- [`HealthRouter`](src/health.rs) — the `/livez` and `/readyz` probe
  router; the checks themselves are `runtime::health::HealthCheck` values
  supplied by the app (e.g. `database::health::check(&pool)`).
- [`serve`](src/server.rs) — the HTTP server future: after shutdown is
  triggered it drains for `SHUTDOWN_DRAIN_SECONDS` (readiness already
  reports `shutting_down`), stops accepting connections, and waits for
  in-flight requests.
- [`ApiConfig`](src/config.rs) — CLI/env configuration for an API binary;
  it flattens `runtime::config::LifecycleConfig` and derives the guardrails
  and CORS policy (`guardrails()`, `cors()`, `stack()`) the stack consumes.
- [`AllowedOrigins`](src/cors.rs) — exact origins parsed from a
  comma-separated value at the configuration edge; the wildcard `*` is
  rejected, and empty input means CORS is off.
- [Extractors](src/extractors.rs) — `ValidatedJson`, `ValidatedQuery`,
  `ValidatedPath`, and `OptionalHeader`, validated against JSON schemas.
- [Error bodies](src/api_error.rs) — the `ErrorBody` envelope (with
  `requestId`), `ValidationErrorBody`, and the `ApiError` response wrapper.
- [`test_support`](src/test_support.rs) — oneshot HTTP test helpers
  (`get`, `post_json`, `put_json`, `post_body`), compiled only under the
  `test-support` feature.

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
`--request-timeout-seconds`, `--request-body-limit-bytes`, and the
`--shutdown-*` flags below) override env vars, which override these
defaults.

| Env var | Default | Meaning |
|---|---|---|
| `LISTEN_ADDRESS` | `0.0.0.0:3000` | Address and port the HTTP listener binds to. |
| `HEALTH_CHECK_TIMEOUT_SECONDS` | `2` | Seconds each liveness or readiness check may take before it counts as failed. |
| `CORS_ALLOWED_ORIGINS` | unset | Comma-separated exact origins allowed to make cross-origin requests; entries are trimmed and must not end with a slash; the wildcard `*` is not supported. Unset or empty means CORS is off. |
| `REQUEST_TIMEOUT_SECONDS` | `30` | Seconds a request may take to produce a response before it is answered with `503 Service Unavailable`. |
| `REQUEST_BODY_LIMIT_BYTES` | `1048576` | Maximum request body size in bytes; larger bodies are rejected with `413 Payload Too Large`. |
| `SHUTDOWN_DRAIN_SECONDS` | `5` | Seconds to keep serving after the shutdown signal, with readiness reporting shutting down, before the server stops accepting connections. |
| `SHUTDOWN_TIMEOUT_SECONDS` | `15` | Seconds after the drain that the HTTP server may take to finish before it is aborted; must be at least 1. |
| `SHUTDOWN_FLUSH_TIMEOUT_SECONDS` | `5` | Seconds the final telemetry flush may take; must be at least 1. |

The shutdown deadline (`SHUTDOWN_DRAIN_SECONDS +
SHUTDOWN_TIMEOUT_SECONDS`) plus `SHUTDOWN_FLUSH_TIMEOUT_SECONDS` must fit
inside the pod's `terminationGracePeriodSeconds`: the defaults give
5 + 15 + 5 = 25 < 30.

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
4. Install shutdown signal handlers — only after `start` and the bind
   succeed, so a `SIGTERM` during startup keeps its default disposition
   (workers and jobs install theirs at startup instead).
5. Supervise [`serve`](src/server.rs) on a
   `runtime::supervisor::Supervisor` under the shutdown deadline, run the
   shutdown hooks, and flush telemetry.

## Exit statuses

| Exit code | Meaning |
|---|---|
| 0 | Success: the HTTP server stopped cleanly after a shutdown signal. |
| 1 | Retryable: a startup failure classified retryable, bind or signal-install failure, HTTP server I/O error, panic; a Deployment restarts the pod. |
| 2 | Permanent: a startup `RunError` classified as permanent (bad input, bad configuration). |
| 143 | Interrupted: the run was cut short by a shutdown signal (128 + SIGTERM), the shutdown deadline passed with the server still running, or a second signal arrived. |

Every startup failure is logged with `tracing::error!` and ends through
`runtime::lifecycle::finish`, so telemetry is flushed on every path. A
telemetry flush failure is logged and never changes the exit status.

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
