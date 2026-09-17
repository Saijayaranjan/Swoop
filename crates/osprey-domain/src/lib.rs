//! Osprey domain model.
//!
//! This crate is deliberately free of I/O: it defines the vocabulary shared by the engines,
//! the services layer, the persistence layer, the HTTP API, the CLI and the FFI surface.
//! Everything here is `Serialize`/`Deserialize` so a single definition feeds SQLite, JSON
//! import/export, the REST API and the UI DTOs.

pub mod ids;
pub mod time;
pub mod task;
pub mod state;
pub mod error;
pub mod queue;
pub mod category;
pub mod rules;
pub mod schedule;
pub mod automation;
pub mod events;
pub mod engine;
pub mod settings;
pub mod history;
pub mod device;
pub mod health;
pub mod media;
pub mod torrent;

pub use ids::*;
pub use time::Millis;
pub use task::*;
pub use state::*;
pub use error::*;
pub use events::*;
