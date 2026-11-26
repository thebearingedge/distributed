//! Typed JSON Kafka producers and consumers on rdkafka over PLAINTEXT. It
//! covers CLI/env configuration, at-least-once consumption with heartbeat
//! and shutdown integration, and W3C trace propagation through message
//! headers. Its errors are classified into
//! [`messaging::MessagingError`] and
//! [`runtime::run_error::RunError`].

/// Topic administration helpers for creating and deleting topics.
pub mod admin;
/// Client configuration from CLI args and env vars.
pub mod config;
/// At-least-once JSON consumption with heartbeat and shutdown integration.
pub mod consumer;
/// Message-header adapters for W3C trace propagation.
pub mod headers;
/// Error types and their classification.
pub mod kafka_error;
/// Typed JSON producer with idempotent delivery.
pub mod producer;
