//! swoop-ffi — the UniFFI (proc-macro) bridge that embeds the Swoop engine in the macOS app.
//! See `docs/architecture/003-ffi-surface.md`.
//!
//! * [`engine::SwoopEngine`] — the object the app holds; owns the Tokio runtime, the engine, the
//!   event forwarders and the local/remote API servers.
//! * [`events`] — `FfiEvent`, the `EventListener` callback interface and the batching forwarder.
//! * [`types`] — flat records/enums mirroring the domain.
//! * [`error::FfiError`] — typed errors (`throws` in Swift).

uniffi::setup_scaffolding!("swoop_ffi");

pub mod engine;
pub mod error;
pub mod events;
mod server;
pub mod types;

pub use engine::SwoopEngine;
pub use error::FfiError;
pub use events::{EventListener, FfiEvent, FfiNotification};
pub use types::*;

#[cfg(test)]
mod tests;
