//! Engine construction. [`start`] wires the store, the transfer engines and the services into a
//! running [`crate::EngineApi`] implementation.

use crate::SharedEngine;
use osprey_domain::DomainResult;
use std::path::PathBuf;

/// Everything needed to bring the engine up.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// Data directory (database, torrent session, tokens, logs). `None` = platform default.
    pub data_dir: Option<PathBuf>,
    /// Running without a desktop UI (affects platform actions, sleep prevention, notifications).
    pub headless: bool,
    /// Displayed in `EngineInfo` and the User-Agent.
    pub app_version: String,
    /// Skip the single-instance lock (tests only).
    pub skip_instance_lock: bool,
    /// Override the download directory (tests / CLI `--dir`).
    pub download_dir: Option<PathBuf>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            data_dir: None,
            headless: false,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            skip_instance_lock: false,
            download_dir: None,
        }
    }
}

/// Start the engine. Must be called from within a Tokio runtime; the engine spawns its background
/// tasks on the current runtime handle.
pub async fn start(config: EngineConfig) -> DomainResult<SharedEngine> {
    let engine = crate::engine::Engine::start(config).await?;
    Ok(engine)
}
