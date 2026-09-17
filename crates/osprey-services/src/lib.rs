//! Osprey application services.
//!
//! The [`api::EngineApi`] trait is the one command surface consumed by the macOS app (through
//! `osprey-ffi`), the REST/WebSocket server, and the CLI. [`Engine`] is its production
//! implementation, wiring the persistence layer, the transfer engines and the services.

pub mod api;
pub mod bootstrap;

pub use api::*;
