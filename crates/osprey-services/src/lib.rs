//! Osprey application services.
//!
//! The [`api::EngineApi`] trait is the one command surface consumed by the macOS app (through
//! `osprey-ffi`), the REST/WebSocket server, and the CLI. [`Engine`] is its production
//! implementation, wiring the persistence layer, the transfer engines and the services.
//!
//! Module map (see `docs/architecture/005-services.md`):
//!
//! | module | responsibility |
//! |---|---|
//! | [`engine`] | the `Engine` facade: construction (`Engine::start`) and the `EngineApi` impl |
//! | [`tasks`] | in-memory task table, per-run handles and the engine-facing progress sink |
//! | [`persist`] | ordered persistence queue in front of the store |
//! | [`manager`] | task lifecycle: add, admission, runs, outcomes, pause/resume, recovery |
//! | [`queues`] | queue CRUD, summaries, queue pause gating |
//! | [`bandwidth`] | limiter tree (global → queue → task), traffic modes, `optimize` |
//! | [`scheduler`] | schedule windows, conditions with hysteresis, gating, schedule actions |
//! | [`rules`] | organisation rules applied at add time and after completion |
//! | [`history`] | history queries and duplicate detection |
//! | [`disk`] | free-space and volume monitoring |
//! | [`diagnostics`] | per-task log ring, health score, diagnostics report |
//! | [`devices`] | pairing, device tokens, lockout, audit |
//! | [`credentials`] | keyring-backed secrets with a file fallback |
//! | [`settings`] | settings validation, persistence and propagation |
//! | [`recipes`] | reusable download workflows |
//! | [`automation`] | automation runner (events → rules → executor) |
//! | [`grabber`] | site grabber sessions |
//! | [`import_export`] | JSON bundles |
//! | [`logs`] | tracing subscriber with ring buffer + daily file |
//! | [`updates`], [`plugins`], [`archives`] | thin adapters over the helper crates |

pub mod api;
pub mod archives;
pub mod automation;
pub mod bandwidth;
pub mod bootstrap;
pub mod categories;
pub mod credentials;
pub mod devices;
pub mod diagnostics;
pub mod disk;
pub mod engine;
pub mod grabber;
pub mod history;
pub mod import_export;
pub mod lifecycle;
pub mod logs;
pub mod manager;
pub mod persist;
pub mod plugins;
pub mod probe;
pub mod queues;
pub mod recipes;
pub mod rules;
pub mod scheduler;
pub mod settings;
pub mod tasks;
pub mod torrents;
pub mod updates;

pub use api::*;
pub use bootstrap::EngineConfig;
pub use engine::Engine;
