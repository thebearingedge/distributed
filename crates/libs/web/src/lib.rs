//! Shared axum web plumbing: HTTP error bodies, schema-validated extractors,
//! health-probe routing, tracing and metrics middleware, request guardrails
//! (timeout, body limit, catch-panic), `OpenAPI` helpers, the runtime
//! configuration for axum API binaries, and — under the `test-support`
//! feature — oneshot HTTP test helpers.

/// HTTP error-response body types ([`crate::api_error::ErrorBody`],
/// [`crate::api_error::ValidationErrorBody`]) and the
/// [`crate::api_error::ApiError`] response wrapper.
pub mod api_error;

/// Request body-size limiting ([`crate::body_limit::with_body_limit`]).
pub mod body_limit;

/// Panic-to-500 middleware ([`crate::catch_panic::with_catch_panic`]).
pub mod catch_panic;

/// Runtime configuration for axum API binaries
/// ([`crate::config::ApiConfig`]).
pub mod config;

/// CORS layer composition for browser clients
/// ([`crate::cors::with_cors`]).
pub mod cors;

/// Encoding and decoding of opaque, base64url pagination cursor tokens.
pub mod cursor_token;

/// JSON fallback handlers and the [`crate::fallback::with_json_fallbacks`]
/// router helper for axum's empty 404 and 405 responses.
pub mod fallback;

/// JSON-Schema-validated extractors: [`crate::extractors::ValidatedJson`],
/// [`crate::extractors::ValidatedQuery`],
/// [`crate::extractors::ValidatedPath`], and the configurable optional
/// request-header extractor [`crate::extractors::OptionalHeader`].
pub mod extractors;

/// The [`crate::graceful_shutdown::graceful_shutdown`] drain future for axum
/// servers, driven by [`runtime::shutdown::Shutdown`].
pub mod graceful_shutdown;

/// Request guardrails composite ([`crate::guardrails::with_guardrails`]).
pub mod guardrails;

/// Liveness (`/livez`) and readiness (`/readyz`) probe router
/// ([`crate::health::HealthRouter`]).
pub mod health;

/// OpenTelemetry-convention request tracing middleware.
pub mod middleware;

/// OpenTelemetry-convention HTTP server metrics middleware
/// ([`crate::metrics::with_http_metrics`]).
pub mod metrics;

/// `OpenAPI` document handler and standard error-response transforms.
pub mod openapi;

/// Request-ID middleware: generation, propagation, sanitization, and
/// task-local access to the current request's ID.
pub mod request_id;

/// Request timeout middleware ([`crate::timeout::with_request_timeout`]).
pub mod timeout;

/// Oneshot HTTP test helpers for exercising routers and handlers
/// ([`crate::test_support::get`], [`crate::test_support::post_json`],
/// [`crate::test_support::put_json`], [`crate::test_support::post_body`]) —
/// compiled only when the `test-support` feature is enabled.
#[cfg(feature = "test-support")]
pub mod test_support;
