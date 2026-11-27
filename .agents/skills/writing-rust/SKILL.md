---
name: writing-rust
description: Writing or modifying Rust code in the distributed workspace. Use when creating or editing .rs files, Cargo.toml members, migrations, or tests — covers Clean Architecture layering, error handling patterns, module layout, and the lint/test loop.
---

# Writing Rust in the distributed workspace

Read `AGENTS.md` at the repo root first — it holds the hard rules (commit policy, branch naming, secrets, dependency policy, scratch locations).

Adding a new HTTP endpoint to an API app? Follow the `adding-an-endpoint` skill — it expands the inside-out workflow below into the endpoint-specific playbook (DTO naming, OpenAPI wiring, validation, error mapping, per-handler test layout).

## Style priorities

Two qualities carry extra weight in review, alongside the rules below:

- **Idiomatic over manual:** prefer the construct the language/std offers —
  iterator adapters and combinators over `if`-based `Option`/`Result`
  plumbing, `match` over `if/else` on `bool` predicates, method references over
  trivial closures. When a helper only *reads* a value it doesn't own, take it
  by reference; take it by value only when it consumes it.
- **Typed over stringly-typed:** pass typed values (`Method`, enums, newtypes)
  through APIs instead of string parameters; keep statuses, modes, and
  identifiers as enum variants or named constants and produce their string form
  only at the boundary — serialization (`serde` renames), header construction,
  wire formats. Never compare or branch on bare string literals for state.
- **API design for launch ergonomics:** keep escape hatches public — a
  composite helper (`web::stack::with_api_stack`, `web::launch`) covers the
  common case, but the pieces it wraps (the individual `web::…::with_*` layers,
  `web::server::serve`, `HealthRouter`, `Supervisor`) stay callable. Keep
  launchers shallow: one start step and one shutdown-hook kind per process
  shape (`ApiParts` offers exactly `readiness` checks and `on_shutdown` hooks),
  no hook matrices or extra supervised-task knobs. Never let a process-shape
  lib absorb domain concerns (`web` knows nothing about databases). Inject
  resources explicitly (`ApiContext::meter_provider`, `JobContext`) instead of
  reading globals. Don't merge distinct concepts to save a parameter — one
  config field per concern. Parse configuration into final types at the edge
  (a clap `value_parser` like `AllowedOrigins::parse`) rather than shipping
  raw strings behind pass-through accessors.

## Placement — decide the layer before writing code

Dependency direction is strictly apps → domains → libs. Put new code where its dependencies point outward:

- **HTTP/framework/CLI concerns** (axum handlers, extractors, config parsing, `main`) → `crates/apps/<category>/<app>` (e.g. `api/default-api`, `api/todos-api`, `worker/default-worker`, `worker/greetings-producer`, `worker/greetings-consumer`, `job/default-job`, `cli/ops-cli`)
- **Business rules, ports, use cases** → `crates/domains/<domain>/src/use_cases`
- **Domain models and invariants** (validation like `has_valid_task`) → `crates/domains/<domain>/src/models`
- **Backend adapters** (sqlx/sea-query repositories, Kafka publishers/subscribers) → `crates/domains/<domain>/src/infrastructure/<kind>/<backend>/` (e.g. `repositories/postgres/pg_todos_repository.rs`, `messaging/kafka/kafka_greeting_publisher.rs`)
- **Generic reusable plumbing** (web middleware, error envelopes, telemetry, pagination) → `crates/libs/*`. Code with cross-domain shape (ports, models, adapters a second domain could adopt) is extracted to the appropriate lib — or a new lib crate — per the extraction policy in `AGENTS.md`; don't let it strand in one domain.

Leaks to avoid (found in past reviews):

- No sqlx/postgres types in use_cases — use case code sees only its repository trait.
- No domain-specific types in shared libs — e.g. `CursorError::SortMismatch` existing only for todos in `libs/web` was flagged; add a config knob or keep the concern in the app instead.
- Domain → HTTP status mapping belongs in the app's `app_error.rs`, never in `libs/web` (`From<StorageError>` impls hard-coding 503/409/500 there were flagged).
- No UUID-version or ID-scheme checks in use cases (`get_version_num() != 7` conflated "bad ID" with "not found" — don't repeat it).
- Domain-general machinery stranded in a single domain (e.g. the idempotency machinery initially built in `todos`) gets flagged for extraction.

## Naming and structure — suffixes carry the layer

- **Domain models** (in `crates/domains/<domain>/src/models`) end in `Model`: `TodoModel`, `NewTodoModel`, `TodoUpdateModel`.
- **Port files** in `crates/domains/<domain>/src/use_cases` are entity-qualified: `todo_unit_of_work.rs` — a domain may contain multiple entities, so port files anchor on their entity (singular), not the domain name.
- **Adapter implementation files** inside a backend adapter directory (`infrastructure/repositories/postgres/`) are backend-prefixed: `pg_todos_repository.rs`, `pg_unit_of_work.rs` — this disambiguates them from same-named port files and mirrors the `Pg*` type names. Mods match file names; external public paths are unaffected (re-exports keep the type names at the same paths).
- **Shared-crate consts are concept-prefixed**, not generic: `IDEMPOTENCY_MIN_LENGTH`, not `MIN_LENGTH` — generic names collide at re-export sites (PR 66 review).
- HTTP-side naming and placement (DTOs mirroring `Model` names as `Http`, `Query`/path structs, colocating structs with their handler) is endpoint-specific — see the `adding-an-endpoint` skill.

## Comments — what belongs in code

- **Rustdoc is expected and lint-enforced**: the workspace sets `missing_docs = "warn"` (`[workspace.lints.rust]` in the root `Cargo.toml`, inherited by every crate) and CI fails on warnings, so every public item — including fields, enum variants, trait methods, and binary/test crate roots — needs a doc comment. Never add `#[allow(missing_docs)]`; document the item instead. Keep `# Errors` sections on fallible functions and `# Panics` on panicking ones. Intra-doc links on items visible at a crate root must be crate-qualified (`` [`storage::StorageError`] `` style, or `` [`crate::…`] ``) — docs on re-exported items re-render at crate-root scope, where short links silently break; verify with `cargo doc --no-deps -p <crate>`.
- **No "decision" comments**: never narrate choices, history, or rationale in code ("agreed rule:", "deliberately untouched", "mirrors the existing pattern", "we chose X because Y"). If a decision needs recording, it goes in the plan/notes, not the source.
- **No third-party API documentation comments**: never paraphrase what a framework, crate, or database feature does (e.g. what `AssertSqlSafe` means, what a Postgres function returns, how sea-query renders an expression). The code and upstream docs cover it.
- **Explanatory comments are opt-in**: anything beyond rustdoc must be explicitly planned and approved ahead of time (list the comment in the plan task's `Work:` section). Treat any unplanned explanatory comment as a review finding.

## Patterns

- **Imports**: three groups separated by blank lines, in this order — std library, third-party crates, then one contiguous block of internal workspace crates (`todos`, `greetings`, `database`, `storage`, `messaging`, `kafka`, `telemetry`, `utils`, `web`, `runtime`, `worker`, `jobs`) together with the current crate's paths (`crate::` / `super::`) — project crate imports don't need separation lines (PR 66 review). rustfmt orders `super::`/`crate::` within the merged block, so they sort first — that is expected, not a formatting bug. The same rule applies inside `#[cfg(test)]` modules and to integration-test files under `crates/**/tests`.
- **Error types**: one dedicated file per crate/layer named `<thing>_error.rs`, using `thiserror`. When wrapping a source error, interpolate `{source}` in the `#[error(...)]` message so diagnostics survive `Display` — this binds only for fields *named* `source`; for unnamed `#[source]` tuple fields, positional `{0}` is the correct equivalent (thiserror 2.x does not bind `{source}` for tuple fields; verified in review of issue #70). If a struct-variant split (like `DbError` → `StorageError` classification) is intentional, document it at the top of the file. (The app-layer HTTP error envelope convention is endpoint-specific — see `adding-an-endpoint`.)
- **Metrics instrumentation**:
  - SDK setup lives only in `telemetry` (`telemetry::Telemetry`, `telemetry::provider::*`).
  - Instrumentation lives in the lib that owns the transport or resource (`web::metrics`, `database::metrics`,
    `kafka::metrics`). It depends on the `opentelemetry` API crate only.
  - Take the provider explicitly (`meter_provider: &P` where `P: MeterProvider + ?Sized`) and name the
    instrumentation scope `env!("CARGO_PKG_NAME")`. Never read the global provider inside a lib.
  - Process shapes hand the provider down: APIs pass `telemetry.meter_provider()`; workers and jobs use
    `context.meter_provider()` (`worker::WorkerContext`, `jobs::JobContext`). Tests inject
    `opentelemetry::metrics::NoopMeterProvider` or a `MetricsHarness` provider.
  - Instrumentation that a domain adapter needs is handed over as a prebuilt lib handle
    (`&kafka::metrics::ProducerMetrics`, `&ConsumerMetrics`), so domains never depend on `opentelemetry`. Build
    handles that register observable callbacks once per provider, because callbacks live as long as the provider.
  - Use OTel semantic-convention names as concept-prefixed consts.
  - Never use unbounded values (raw paths, IDs, client-supplied strings) as attributes.
  - Reuse `telemetry::metrics` (`duration_histogram`, `InFlightGuard`) rather than re-deriving buckets or
    in-flight counting.
  - Test through `telemetry::test_support::MetricsHarness` (dev-dependency feature `test-support`), and never
    set the global meter provider in tests.
- **Messaging (Kafka)**:
  - Domain outbound ports return `messaging::MessagingError` (`Unavailable` is retryable, `Rejected` is
    permanent), never rdkafka types — the kafka adapter classifies its source errors into the taxonomy
    (see `greetings::GreetingPublisher`).
  - Kafka adapters keep their serde wire structs private and convert them with `From`, mirroring `TodoRow`
    (`kafka_greeting_message.rs`), so domain models stay serde-free.
  - Topic names are domain-owned constants (`greetings::GREETINGS_TOPIC`), not config fields.
  - The `kafka` lib has no root re-exports; use module paths (`kafka::producer::KafkaProducer`,
    `kafka::consumer::KafkaConsumer`, `kafka::config::KafkaConfig`, `kafka::metrics::ProducerMetrics`), as with
    `runtime`.
  - Message keys are passed by the caller (`send(key: Option<&[u8]>, value)`); the lib never derives them.
- **Broker-backed tests** create a unique topic and group per test (`kafka::admin::create_topic`,
  uuid-suffixed names) since broker auto-create is off, and guard every wait with
  `tokio::time::timeout(..)`; they run plain `#[tokio::test]` with real time.
- **API layer composition**: the required order is written once in
  `web::stack::with_api_stack(router, StackConfig, meter_provider)`, and apps
  compose their router through it. Layers, outermost first:
  `with_http_metrics` → `with_cors` → `with_http_tracing` (request id + span) →
  `web::guardrails::with_guardrails` (catch-panic → timeout → body limit) → the
  API router; merge the health probe router
  (`web::health::HealthRouter::new(..)…into_router()`) **after** the stack so
  probes bypass the layers. Integration-test routers call
  `with_api_stack(.., StackConfig::default(), &NoopMeterProvider::new())`
  directly — never re-derive the order by hand. Layers that build `ApiError`
  bodies sit inside `with_http_tracing` so the bodies carry `requestId`. The
  individual `web::…::with_*` functions stay public for bespoke stacks.
  Layers that return empty or plain-text bodies (tower-http `TimeoutLayer`,
  `RequestBodyLimitLayer`) are not used, because they break the JSON error shape.
- **Modules**: multi-file modules use the file-and-folder pattern — `mod.rs`-style parent file (`repositories.rs`) alongside its directory (`repositories/todos_repository.rs`). New sibling modules should match.
- **Validation**: single source of truth. If a domain model validates (e.g. min length), don't re-encode the rule in the delivery layer (or make them agree); error messages must match the actual rule (`EmptyTask` shouldn't be raised when the real rule is "more than one character").
- **Dependencies**: workspace deps come from the root `Cargo.toml` `[workspace.dependencies]` — reference with `workspace = true`. Before proposing a new third-party dependency, check that no existing workspace dependency already covers the need. Looking up the latest version of a crate (crates.io/docs.rs) is fine, but never add or upgrade a dependency without explicit human permission — propose the crate, version, and reason first, then wait. Never declare a dep a crate doesn't use (unused `uuid`, `thiserror`, `axum` were all flagged).
- **Magic numbers** in shared plumbing (timeouts, limits): accept them as parameters or name them as constants — no bare literals with no configuration path.
- **Time in tests**: time-dependent tests run under `#[tokio::test(start_paused = true)]` and move the clock with auto-advancing sleeps/timeouts or `tokio::time::advance(..)` — never real-time delays (details in "Process shapes").
- **`#[sqlx::test]` migrations**: add `migrator = "<domain>::MIGRATOR"` (plus `fixtures(...)`) only when the test executes SQL against that domain's schema — route handlers that query or write, repository tests, anything touching the tables. Tests that never run SQL — OpenAPI document checks, 404/405 fallback behavior, middleware/request-id plumbing, pool-failure paths — use a plain `#[sqlx::test]` (a fresh, empty database) so migration time isn't paid on every run. When in doubt, trace the route: if the handler's success and failure paths both leave the pool untouched, the schema isn't needed.

## Process shapes (API, worker, job)

- **Lifecycle primitives** live in `runtime` (tokio-only; its only non-tokio dependencies are thiserror, tracing, tokio-util, and clap (for `LifecycleConfig`)): `runtime::shutdown::Shutdown` is the one shutdown handle per process — clone it into every task; `runtime::supervisor::Supervisor` runs named critical tasks — the first to stop (error, panic, or a clean return before shutdown) triggers shutdown and becomes the culprit; `runtime::deadline::within_deadline` bounds a future by the shutdown deadline; `runtime::config::LifecycleConfig` carries the drain/timeout/flush settings (see Configuration); `runtime::lifecycle::finish` ends a run with a bounded telemetry flush and returns the `ExitCode` — it calls `std::process::exit` only when work was abandoned at the deadline or the flush timed out; `runtime::exit_status::ExitStatus` is the exit taxonomy (0 success / 1 retryable / 2 permanent / 143 interrupted); `runtime::run_error::RunError` classifies failures as retryable or permanent; `runtime::health::HealthCheck` is a named, type-erased readiness/liveness check (`new`/`with_resource`, `name()`/`run()`) that probe servers and APIs wrap; `runtime::heartbeat::Heartbeat` carries worker liveness. Resource libs expose checks built from it — e.g. `database::health::check(&pool)` produces the `postgres` readiness check.
- **Lifecycle invariants**:
  `SHUTDOWN_DRAIN_SECONDS + SHUTDOWN_TIMEOUT_SECONDS + SHUTDOWN_FLUSH_TIMEOUT_SECONDS < terminationGracePeriodSeconds` (defaults 5 + 15 + 5 = 25 < 30; the drain is non-zero because endpoint removal races SIGTERM). Every long-running task in a process runs under one `Supervisor`. The deadline (drain + timeout) starts when shutdown is triggered, by a signal or by a failing task. Work a task does during shutdown (e.g. `KAFKA_PRODUCER_FLUSH_TIMEOUT_SECONDS`, default 10) must fit inside `SHUTDOWN_TIMEOUT_SECONDS`. When the deadline passes, the remaining tasks are aborted; telemetry is flushed within `SHUTDOWN_FLUSH_TIMEOUT_SECONDS`, and the process exits 143, or with the culprit's status if a task failed first. A second SIGTERM/SIGINT exits 143 immediately, skipping drain, deadline, and flush. Worker and probe-server panics are caught and exit 1. Env vars (every process shape): the three `SHUTDOWN_*` vars above — `SHUTDOWN_TIMEOUT_SECONDS` and `SHUTDOWN_FLUSH_TIMEOUT_SECONDS` must each be ≥ 1 — and APIs and workers also get `HEALTH_CHECK_TIMEOUT_SECONDS` (default 2).
- **Signal installation**: `web::launch` installs the handlers only after `Api::start` and the bind succeed, so SIGTERM during connect/migrations keeps its default disposition; `worker::launch` and `jobs::launch` install them at startup. A second signal exits 143 immediately via `std::process::exit`, skipping drain, deadline, and flush.
- **APIs**: the app implements `web::Api` in its lib target — `async fn start(self, context: &ApiContext) -> Result<ApiParts, RunError>` connects backends, runs migrations, and builds the router — and `main` is parse → construct → `web::launch(service_name, api, &args.api).await`, returning `ExitCode` (no anyhow). `ApiParts::new(router)` carries optional `.readiness(HealthCheck)` checks reported by `/readyz` and `.on_shutdown(hook)` hooks; hooks run in registration order after the HTTP server stops and are **skipped when the shutdown deadline was exceeded** (aborted tasks may still hold the resources), so resource cleanup like `pool.close()` is an `on_shutdown` hook. Instrumentation takes `context.meter_provider()`. Startup failures return a classified `RunError`; `web::launch` logs them with `tracing::error!` and ends through `runtime::lifecycle::finish`, so the classification becomes the exit status and telemetry is flushed on every path.
- **Workers** implement `worker::Worker` (`#[async_trait]`): they must call `context.heartbeat().beat()` at least every `context.heartbeat().interval()` and return once `context.shutdown()` is triggered — returning earlier exits 1. The app `main` flattens `worker::WorkerConfig` and calls `worker::launch(service_name, worker, &args.worker)`; the worker is taken by value (`'static`) and supervised on a `JoinSet`-backed `Supervisor` alongside the probe server (`worker::supervisor::supervise`), and panics are caught and exit 1. `/livez` is heartbeat-backed and `/readyz` flips to `503 shutting_down` on shutdown. Instrumentation takes `context.meter_provider()`.
- **Kafka workers**: rdkafka clients are built inside `Worker::run`, never in `main`, because `worker::launch` initializes telemetry first and rdkafka captures the log level when `ClientConfig::new()` runs. Wrap any await that can outlast `stale_after` in `context.heartbeat().beat_while(..)`. Consumers run `kafka::consumer::KafkaConsumer::run(&handler, shutdown, heartbeat)`; a handler error stops the consumer without storing/acking the failing message's offset, so it is redelivered. Blocking rdkafka calls (sync commit, consumer drop, producer flush) go through `tokio::task::spawn_blocking`, never `block_in_place`. Build `kafka::metrics::{ProducerMetrics, ConsumerMetrics}` from `context.meter_provider()` inside `Worker::run`, once, and pass them to the client constructors or domain `connect` functions.
- **Jobs** implement `jobs::Job` (`#[async_trait]`) and are idempotent. They receive a `jobs::JobContext` and observe `context.shutdown()`; instrumentation takes `context.meter_provider()`. An error returned after shutdown exits 143, and permanent failures return `RunError::permanent(..)` (exit 2, for `podFailurePolicy` `FailJob`). The app `main` flattens `runtime::config::LifecycleConfig` into its `Args` and calls `jobs::launch(service_name, job, &args.lifecycle)`, taking the job by value like `worker::launch`; a job still running at the deadline is dropped and exits 143. `jobs` never depends on axum or web.
- **Testability split**: the `launch` functions are untested wiring because they call the process-global `Telemetry::init`; behavior lives in testable cores (most take an injected `Shutdown`; `jobs::run` and `jobs::run_within` take an injected `jobs::JobContext`): `runtime::supervisor::Supervisor`, `runtime::deadline::within_deadline`, `runtime::lifecycle::flush_within`, `worker::supervise`, `worker::probes_router`, `jobs::run`, `jobs::run_within`, `web::server::serve`, `web::api::run` (crate-private, tested inside `web`), and `web::ApiParts::router`, which lets app tests serve the same router the process does. Lib tests never call `Telemetry::init` and never send real signals. Blocking work under paused time needs a manual `tokio::time::advance(..)`, because tokio inhibits auto-advance while a `spawn_blocking` task runs.
- **Time in tests**: time-dependent tests use `#[tokio::test(start_paused = true)]` (tokio `test-util`, enabled only in lib `[dev-dependencies]`) and move the clock with auto-advancing sleeps/timeouts or `tokio::time::advance(..)` (it is async and must be awaited); never real-time delays. Time-reading lib code uses `tokio::time::Instant`, so it follows the paused clock.
- **Runtimes**: binaries use the default multi-thread `#[tokio::main]`, never `current_thread` — OTLP shutdown hangs on it.
- **Telemetry flush failures** are logged with `tracing::warn!` and never change the exit status, and the flush is bounded by `SHUTDOWN_FLUSH_TIMEOUT_SECONDS`.
- **Service names**: the compiled-in name passed to `launch`/`Telemetry::init` is a default; `OTEL_SERVICE_NAME` overrides it per deployment.
- **Images**: every app builds from `tools/docker/Dockerfile.rust-app` via `just docker-build <app>`; no per-app Dockerfiles, and the container binary is always `/app`.

## Configuration

- **Layered lib configs**: reusable config structs live in libs (`web::config::ApiConfig`, `runtime::config::LifecycleConfig`, `database::DbConfig`, `kafka::config::{KafkaConfig, KafkaConsumerConfig, KafkaProducerConfig}`) and derive `Debug, clap::Parser` — not bare `clap::Args`, which clap 4.6's derive lacks `try_parse_from` on; the `Parser` derive also generates the `Args` impl, so `#[command(flatten)]` still consumes it. `LifecycleConfig` is flattened inside `web::config::ApiConfig` and `worker::WorkerConfig`, and directly into job apps' `Args`. Every field pairs `#[arg(long, env = "…", …)]` with a doc comment (`missing_docs`) and a declarative default (`default_value` / `default_value_t`); env names are concept-prefixed (`DB_MIN_CONNECTIONS`, not `MIN_CONNECTIONS`). Typed accessors convert to std/composite types (`Duration`, `GuardrailsConfig`, `CorsConfig`, `StackConfig`) instead of leaking raw fields into wiring: `CORS_ALLOWED_ORIGINS` parses at clap time (`value_parser = AllowedOrigins::parse`) into `web::cors::AllowedOrigins` — comma-separated exact origins, the wildcard `*` is rejected with a clear error — and `ApiConfig::cors()` / `ApiConfig::stack()` hand the parsed policy to the wiring.
- **Health check timeout**: both `web::config::ApiConfig` and `worker::WorkerConfig` name the probe timeout `HEALTH_CHECK_TIMEOUT_SECONDS` (`health_check_timeout_seconds`), feeding `web::health::HealthRouter::new(..)`.
- **App composition root**: every app is lib + thin bin — the app value (the `Worker`/`Job`/`Api` impl, e.g. `TodosApi`, `InertWorker`, `NoopJob`, `GreetingsProducerWorker`) lives in the crate's lib target so app tests can exercise it, and `main.rs` is parse → construct → launch. Each binary also has a bin-private `src/config.rs` declared with `mod config;` in `main.rs` — a binary-target module, never exported from the lib target. It defines `pub(crate) struct Args` deriving `clap::Parser` and flattens exactly the lib configs the app uses:

  ```rust
  /// Command-line configuration for the todos API binary.
  #[derive(Debug, clap::Parser)]
  pub(crate) struct Args {
      /// Web/runtime configuration.
      #[command(flatten)]
      pub(crate) api: ApiConfig,
      /// Database configuration.
      #[command(flatten)]
      pub(crate) db: DbConfig,
  }
  ```

  Unparsable config fails fast via clap; no warn-and-default fallbacks.
- **`main.rs` is orchestration-only**: parse once at the top (`Args::parse()`), then only wire — no `std::env::var`, no hand-rolled parsing, no consts standing in for tunable knobs, no config-shaped types defined inline.
- **Clap arg ids are field names**: lib config field names that could recur across flattened configs are concept-prefixed (`kafka_consumer_group_id`, not `group_id`), because duplicate clap ids panic at first parse in debug builds; add a flatten test that combines the configs an app flattens.
- **Required env-backed args in tests**: tests never assert that a missing required env-backed arg fails to parse, because env files and CI set those values (`KAFKA_BOOTSTRAP_SERVERS`, `DATABASE_URL`); assert required-ness through `Command` introspection (`is_required_set`, `get_env`) instead.
- **Composition at the app layer**: the merge point is the app's `Args` because only the app knows which libs it uses; libs must not depend on each other to unify config (no `web → database`), and apps must not advertise flags for database concerns they don't have (`default-api` flattens no database config).

## Before you finish

Run from the repo root (tooling needs direnv active — see `AGENTS.md`, whose
`## Commands` table owns the recipe set). The loop for a finished task:

1. `just cargo-fmt` — zero fmt drift; fix with `just cargo-fmt-fix`.
2. `just cargo-lint` — zero warnings. Mid-task, when the workspace is
   intentionally red (e.g. a trait method awaiting its impl), scope it:
   `just cargo-lint <crate>`; between designed intermediate states, plain
   `just cargo-check <crate>` is the fast loop.
3. `just cargo-test <crate>` for every crate you touched; integration tests
   need Postgres up: `just infrastructure up -d --wait`.
4. Migrations changed: `just migrate <domain> run` (revert syntax lives in
   the `AGENTS.md` commands table).

Prefer the `just` recipes over bare `cargo` commands — they wire in the
per-crate env files (`tools/envs/<name>.env`) and keep one place for flags.

Commits are governed by the `AGENTS.md` hard rules (explicit human permission; the commit is performed by the human or the built-in `build` agent). `.agents/temp/` is your scratch/output area — put notes, plans, and generated artifacts there (forbidden locations: see `AGENTS.md`).
