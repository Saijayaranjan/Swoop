//! Read-only views over a librqbit torrent handle: a cheap per-tick snapshot, the domain
//! [`Progress`], the live [`TorrentInfo`], and the piece-map encoding.

use crate::limits::ratio;
use crate::metainfo::ParsedTorrent;
use crate::peers::RawPeer;
use crate::session::ManagedTorrentHandle;
use crate::trackers::TrackerRegistry;
use librqbit::TorrentStatsState;
use osprey_domain::torrent::TorrentInfo;
use osprey_domain::{Millis, Progress};

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub initializing: bool,
    pub live: bool,
    pub paused: bool,
    pub error: Option<String>,
    /// Bytes of the *selected* files we have (during the initial check: bytes checked).
    pub downloaded: u64,
    /// Bytes of the selected files.
    pub total: u64,
    pub finished: bool,
    /// Uploaded since the current live state started.
    pub uploaded_live: u64,
    pub speed: u64,
    pub upload_speed: u64,
    pub eta_seconds: Option<u64>,
    pub peers_live: u32,
    pub peers_connecting: u32,
    pub file_progress: Vec<u64>,
}

pub fn snapshot(handle: &ManagedTorrentHandle) -> Snapshot {
    let stats = handle.stats();
    let mut s = Snapshot {
        downloaded: stats.progress_bytes,
        total: stats.total_bytes,
        finished: stats.finished,
        uploaded_live: stats.uploaded_bytes,
        file_progress: stats.file_progress,
        error: stats.error,
        ..Snapshot::default()
    };
    match stats.state {
        TorrentStatsState::Initializing { .. } => s.initializing = true,
        TorrentStatsState::Live => s.live = true,
        TorrentStatsState::Paused => s.paused = true,
        TorrentStatsState::Error => {
            if s.error.is_none() {
                s.error = Some("torrent is in error state".into());
            }
        }
    }
    if let Some(live) = handle.live() {
        s.speed = live.down_speed_estimator().bps();
        s.upload_speed = live.up_speed_estimator().bps();
        s.eta_seconds = if s.finished {
            None
        } else {
            live.down_speed_estimator()
                .time_remaining()
                .map(|d| d.as_secs())
        };
        let peer_stats = live.stats_snapshot().peer_stats;
        s.peers_live = peer_stats.live;
        s.peers_connecting = peer_stats.connecting;
    }
    s
}

/// Domain progress for one tick. `uploaded_total` includes previous runs.
pub fn progress(s: &Snapshot, uploaded_total: u64) -> Progress {
    let fraction = if s.total > 0 {
        (s.downloaded as f64 / s.total as f64).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    Progress {
        downloaded: s.downloaded,
        uploaded: uploaded_total,
        total: Some(s.total),
        speed: s.speed,
        instant_speed: s.speed,
        upload_speed: s.upload_speed,
        eta_seconds: s.eta_seconds,
        active_connections: s.peers_live,
        peers: s.peers_live,
        // librqbit does not expose peer bitfields, so connected seeds cannot be counted.
        seeds: 0,
        ratio: ratio(uploaded_total, s.downloaded),
        fraction,
    }
}

/// Inputs for [`torrent_info`] that live outside the handle.
pub struct LiveContext<'a> {
    pub registry: &'a TrackerRegistry,
    pub uploaded_total: u64,
    pub seeding_since: Option<Millis>,
    pub dht_nodes: u32,
    pub haves: Option<(Vec<bool>, u32)>,
    pub selected: &'a [bool],
    pub priorities: &'a [u8],
}

/// Merge the static description with live counters.
pub fn torrent_info(parsed: &ParsedTorrent, snap: &Snapshot, ctx: &LiveContext<'_>) -> TorrentInfo {
    let mut info = parsed.info.clone();
    for (i, f) in info.files.iter_mut().enumerate() {
        f.downloaded = snap.file_progress.get(i).copied().unwrap_or(0).min(f.size);
        f.selected = ctx.selected.get(i).copied().unwrap_or(true)
            && !parsed.padding.get(i).copied().unwrap_or(false);
        f.priority = ctx.priorities.get(i).copied().unwrap_or(1);
    }
    info.trackers = ctx.registry.statuses();
    let (seeders, leechers) = ctx.registry.swarm_estimate();
    info.seeders_total = seeders;
    info.leechers_total = leechers;
    info.connected_peers = snap.peers_live;
    info.connected_seeds = 0;
    info.dht_nodes = ctx.dht_nodes;
    info.uploaded = ctx.uploaded_total;
    info.ratio = ratio(ctx.uploaded_total, snap.downloaded);
    info.seeding_since = ctx.seeding_since;
    if let Some((bits, count)) = &ctx.haves {
        let have = bits.iter().filter(|b| **b).count();
        // Lower bound: our own copy. Swarm-wide availability needs peer bitfields, which
        // librqbit keeps private.
        info.availability = if *count > 0 {
            have as f32 / *count as f32
        } else {
            0.0
        };
        info.piece_map_rle = rle(bits);
    }
    info
}

/// Run-length encode a bitfield as alternating run lengths starting with a (possibly empty)
/// run of missing pieces: `[missing, have, missing, have, …]`.
pub fn rle(bits: &[bool]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut current = false;
    let mut run: u32 = 0;
    for &b in bits {
        if b == current {
            run = run.saturating_add(1);
        } else {
            out.push(run);
            current = b;
            run = 1;
        }
    }
    if run > 0 || !bits.is_empty() {
        out.push(run);
    }
    out
}

/// Adapt librqbit's per-peer counters to the sampler's input.
pub fn raw_peers(handle: &ManagedTorrentHandle) -> Vec<RawPeer> {
    let Some(live) = handle.live() else {
        return Vec::new();
    };
    live.per_peer_stats_snapshot(Default::default())
        .peers
        .into_iter()
        .map(|(addr, p)| RawPeer {
            address: addr,
            client: p.client_name,
            fetched_bytes: p.counters.fetched_bytes,
            uploaded_bytes: p.counters.uploaded_bytes,
            incoming: p.counters.incoming_connections > 0,
            connection_kind: p.conn_kind.map(|k| format!("{k:?}")),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rle_encoding() {
        assert_eq!(rle(&[]), Vec::<u32>::new());
        assert_eq!(rle(&[true, true]), vec![0, 2]);
        assert_eq!(rle(&[false, false, true, false]), vec![2, 1, 1]);
        assert_eq!(rle(&[false]), vec![1]);
    }

    #[test]
    fn progress_fraction() {
        let s = Snapshot {
            downloaded: 50,
            total: 200,
            ..Default::default()
        };
        let p = progress(&s, 25);
        assert_eq!(p.fraction, 0.25);
        assert_eq!(p.ratio, 0.5);
        assert_eq!(p.total, Some(200));
    }
}
