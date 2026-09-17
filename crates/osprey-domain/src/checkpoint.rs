//! Durable resume data. Each engine kind persists the state it needs to continue after a crash;
//! the services layer stores it opaquely (`task_checkpoints.blob`) and hands it back on restart.
//!
//! Invariant (enforced by every engine): a checkpoint is only emitted **after** the data it
//! describes has been flushed to stable storage. Anything not covered is fetched again.

use crate::{Millis, SegmentMap};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// HLS: which media segments are on disk in the part directory.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HlsCheckpoint {
    /// Resolved media playlist URL (after variant selection).
    pub media_playlist_url: String,
    pub part_dir: PathBuf,
    pub segment_count: u32,
    /// Bitmap of completed segments packed little-endian (bit i = segment i done).
    pub done_bitmap: Vec<u8>,
    /// AES-128 keys by key URI (hex) — key URLs expire, so cache them.
    pub key_cache: BTreeMap<String, String>,
    /// Segment sizes in bytes for completed segments (for progress + merge).
    pub segment_sizes: BTreeMap<u32, u64>,
    /// `ts` or `fmp4`.
    pub container: String,
    pub init_segment_done: bool,
}

impl HlsCheckpoint {
    pub fn is_done(&self, i: u32) -> bool {
        self.done_bitmap
            .get((i / 8) as usize)
            .map(|b| b & (1 << (i % 8)) != 0)
            .unwrap_or(false)
    }
    pub fn set_done(&mut self, i: u32) {
        let idx = (i / 8) as usize;
        if self.done_bitmap.len() <= idx {
            self.done_bitmap.resize(idx + 1, 0);
        }
        self.done_bitmap[idx] |= 1 << (i % 8);
    }
    pub fn done_count(&self) -> u32 {
        (0..self.segment_count).filter(|&i| self.is_done(i)).count() as u32
    }
}

/// Torrent: librqbit owns piece state in its session dir; we persist what it does not.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TorrentCheckpoint {
    pub info_hash: String,
    /// Selected file indices (None = all) and priorities (index → 0/1/2).
    pub selected_files: Option<Vec<u32>>,
    pub priorities: BTreeMap<u32, u8>,
    pub uploaded: u64,
    pub downloaded: u64,
    pub seeding_since: Option<Millis>,
    pub sequential: bool,
    pub output_folder: PathBuf,
    /// Disabled tracker URLs.
    pub disabled_trackers: Vec<String>,
    /// User-added tracker URLs.
    pub extra_trackers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Checkpoint {
    Segments(SegmentMap),
    Hls(HlsCheckpoint),
    Torrent(TorrentCheckpoint),
}

impl Checkpoint {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Checkpoint::Segments(_) => "segments",
            Checkpoint::Hls(_) => "hls",
            Checkpoint::Torrent(_) => "torrent",
        }
    }
    pub fn as_segments(&self) -> Option<&SegmentMap> {
        match self {
            Checkpoint::Segments(m) => Some(m),
            _ => None,
        }
    }
    pub fn as_hls(&self) -> Option<&HlsCheckpoint> {
        match self {
            Checkpoint::Hls(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_torrent(&self) -> Option<&TorrentCheckpoint> {
        match self {
            Checkpoint::Torrent(t) => Some(t),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hls_bitmap() {
        let mut c = HlsCheckpoint {
            segment_count: 20,
            ..Default::default()
        };
        assert!(!c.is_done(9));
        c.set_done(9);
        c.set_done(0);
        c.set_done(19);
        assert!(c.is_done(9) && c.is_done(0) && c.is_done(19));
        assert_eq!(c.done_count(), 3);
    }
}
