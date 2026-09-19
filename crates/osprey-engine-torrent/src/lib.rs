//! osprey-engine-torrent — BitTorrent engine built on librqbit.
//!
//! See `README.md` in this crate for capabilities, the private-torrent policy and the list of
//! things librqbit does not expose (and how this crate compensates). Architecture context:
//! `docs/architecture/001-architecture-decision.md`.
//!
//! The engine owns exactly one librqbit [`librqbit::Session`], created lazily on first use so
//! a headless instance without torrents pays nothing. librqbit spawns its own tasks on the
//! ambient Tokio runtime, so construct and drive the engine from inside the services runtime.

pub mod engine;
pub mod limits;
pub mod metainfo;
pub mod peers;
mod run;
pub mod session;
pub mod status;
pub mod trackers;

pub use engine::{
    DeferredSettings, Selection, TorrentBlobProvider, TorrentEngine, TorrentEngineConfig,
};
pub use session::SessionTuning;
