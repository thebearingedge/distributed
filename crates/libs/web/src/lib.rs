//! Shared axum web plumbing: HTTP error bodies, schema-validated extractors,
//! health-probe routing, tracing and metrics middleware, request guardrails
//! (timeout, body limit, catch-panic), `OpenAPI` helpers, the runtime
//! configuration for axum API binaries, the API process contract
//! ([`crate::Api`], [`crate::ApiParts`]) with its [`crate::launch()`] and
//! [`crate::launch_with()`] composition roots (labeled by [`crate::Label`]),
//! and — compiled only for tests or when the `test-support` feature is
//! enabled — oneshot HTTP test helpers.

/// The API process contract ([`crate::Api`], [`crate::ApiParts`]).
pub mod api;

/// HTTP error-response body types ([`crate::api_error::ErrorBody`],
/// [`crate::api_error::ValidationErrorBody`]) and the
/// [`crate::api_error::ApiError`] response wrapper.
pub mod api_error;

mod body_limit;

mod catch_panic;

/// Runtime configuration for axum API binaries
/// ([`crate::config::ApiConfig`]).
pub mod config;

/// CORS configuration for browser clients
/// ([`crate::cors::AllowedOrigins`]).
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

mod guardrails;

mod health;

/// The API process composition root ([`crate::launch()`]).
pub mod launch;

mod middleware;

mod metrics;

/// `OpenAPI` document handler and standard error-response transforms.
pub mod openapi;

/// Request-ID middleware: generation, propagation, sanitization, and
/// task-local access to the current request's ID.
pub mod request_id;

mod server;

/// The standard API layer stack ([`crate::stack::with_api_stack`]).
pub mod stack;

mod timeout;

/// Oneshot HTTP test helpers for exercising routers and handlers
/// ([`crate::test_support::get`], [`crate::test_support::post_json`],
/// [`crate::test_support::put_json`], [`crate::test_support::post_body`]) —
/// compiled only for tests or when the `test-support` feature is enabled.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use api::{Api, ApiContext, ApiParts};
pub use launch::{Label, launch, launch_with};
