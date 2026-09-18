//! osprey-engine-http — the HTTP/HTTPS segmented download engine.
//!
//! See the crate README for the algorithm overview. Module map:
//!
//! | module | responsibility |
//! |---|---|
//! | [`engine`] | `HttpEngine`, the [`osprey_runtime::engine::Transfer`] implementation |
//! | [`probe`] | HEAD / ranged-GET resolution of a URL |
//! | [`plan`] | the segment table and its invariants |
//! | [`segment`] | one connection = one attempt at one segment |
//! | [`controller`] | slots, adaptive concurrency, work-stealing, checkpoints, stop protocol |
//! | [`mirrors`] | mirror probing, ranking and failover |
//! | [`metalink`] | Metalink v3/v4 parsing |
//! | [`request`] | header / auth / client-profile construction |
//! | [`classify`] | transport error → `ErrorKind` mapping |
//! | [`hostlimits`] | process-wide per-host and total connection caps |

pub mod classify;
pub mod controller;
pub mod engine;
pub mod hostlimits;
pub mod metalink;
pub mod mirrors;
pub mod plan;
pub mod probe;
pub mod request;
pub mod segment;
pub mod shared;

pub use engine::HttpEngine;
pub use hostlimits::{HostLimits, HostPermit};
pub use metalink::{parse as parse_metalink, MetalinkFile, MetalinkUrl};
pub use probe::ProbeResult;
pub use shared::Tuning;
