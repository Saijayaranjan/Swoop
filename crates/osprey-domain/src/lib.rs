//! Osprey domain model.
//!
//! This crate is deliberately free of I/O: it defines the vocabulary shared by the engines,
//! the services layer, the persistence layer, the HTTP API, the CLI and the FFI surface.
//! Everything here is `Serialize`/`Deserialize` so a single definition feeds SQLite, JSON
//! import/export, the REST API and the UI DTOs.

pub mod automation;
pub mod category;
pub mod checkpoint;
pub mod device;
pub mod error;
pub mod events;
pub mod health;
pub mod history;
pub mod ids;
pub mod media;
pub mod queue;
pub mod rules;
pub mod schedule;
pub mod settings;
pub mod state;
pub mod task;
pub mod time;
pub mod torrent;

pub use checkpoint::*;
pub use error::*;
pub use events::*;
pub use ids::*;
pub use state::*;
pub use task::*;
pub use time::Millis;
