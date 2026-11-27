//! The todos HTTP API, packaged as a lib with a thin `main` bin on top.
//!
//! - [`api`] builds the axum [`Router`](axum::Router) and the aide-generated
//!   `OpenAPI` document served at `/openapi.json`
//! - [`controller`] defines the HTTP DTOs and the request handlers
//! - [`app_state`] bundles the domain interactors and the request-validation
//!   schema validators the handlers share
//! - [`TodosApi`] implements the [`web::Api`] process contract
//!   [`web::launch()`] drives
//!
//! Domain errors are mapped to HTTP responses at this boundary in
//! `app_error.rs` (private) rather than inside the `web` lib.

/// Builds the axum router and the aide-generated `OpenAPI` document.
pub mod api;
mod app_error;
/// The two-part shared request state.
pub mod app_state;
/// HTTP DTOs and request handlers.
pub mod controller;
/// The route paths shared by the router and the create response.
pub mod paths;
mod todos_api;

pub use todos_api::TodosApi;
