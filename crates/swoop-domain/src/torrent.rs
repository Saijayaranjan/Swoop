//! Torrent-specific task data.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TorrentFile {
    pub index: u32,
    pub path: String,
    pub size: u64,
    pub downloaded: u64,
    /// `false` = skipped.
    pub selected: bool,
    /// 0 = low, 1 = normal, 2 = high (engine maps these to piece ordering).
    pub priority: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackerStatus {
    pub url: String,
    pub tier: u32,
    pub enabled: bool,
    #[serde(default)]
    pub last_announce_at: Option<crate::Millis>,
    #[serde(default)]
    pub next_announce_at: Option<crate::Millis>,
    #[serde(default)]
    pub seeders: Option<u32>,
    #[serde(default)]
    pub leechers: Option<u32>,
    #[serde(default)]
    pub latency_ms: Option<u32>,
    #[serde(default)]
    pub last_error: Option<String>,
    /// `working`, `updating`, `error`, `disabled`, `dead`.
    pub health: String,
    #[serde(default)]
    pub consecutive_failures: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeerInfo {
    pub address: String,
    pub client: Option<String>,
    pub download_speed: u64,
    pub upload_speed: u64,
    pub progress: f32,
    pub flags: String,
    pub downloaded: u64,
    pub uploaded: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TorrentInfo {
    pub info_hash: String,
    pub name: String,
    pub total_size: u64,
    pub piece_length: u32,
    pub piece_count: u32,
    pub files: Vec<TorrentFile>,
    pub trackers: Vec<TrackerStatus>,
    pub private: bool,
    pub comment: Option<String>,
    pub created_by: Option<String>,
    pub magnet: Option<String>,
    /// Metadata received (magnets start without it).
    pub have_metadata: bool,
    pub seeders_total: u32,
    pub leechers_total: u32,
    pub connected_peers: u32,
    pub connected_seeds: u32,
    pub dht_nodes: u32,
    pub uploaded: u64,
    pub ratio: f32,
    /// Fraction of pieces available in the swarm (0..=1+); >1 means multiple full copies.
    pub availability: f32,
    /// Bitfield of piece availability compressed as run lengths for the UI's piece map.
    pub piece_map_rle: Vec<u32>,
    pub seeding_since: Option<crate::Millis>,
}

/// Seeding limits; `None` inherits the global settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SeedingLimits {
    pub ratio_limit: Option<f32>,
    pub time_limit_minutes: Option<u32>,
    pub upload_limit: Option<u64>,
    pub seed_when_complete: Option<bool>,
}
