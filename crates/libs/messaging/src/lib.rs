//! A broker-neutral messaging error taxonomy shared by message-transport
//! backends, which classify their source errors into its variants via
//! `From` impls.

/// The messaging error taxonomy.
pub mod messaging_error;

/// The broker-neutral messaging error taxonomy.
pub use messaging_error::MessagingError;
