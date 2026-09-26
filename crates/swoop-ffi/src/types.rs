//! FFI records and enums: flat, FFI-friendly mirrors of the domain (`String` ids, `i64` millis,
//! `Option<u64>` sizes). Conversions live next to each type.

use crate::error::{FfiError, FfiResult};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use swoop_domain as d;
use swoop_domain::Millis;
use swoop_services as s;

// ---------------------------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------------------------

pub(crate) fn ms(m: Millis) -> i64 {
    m.0
}

pub(crate) fn ms_opt(m: Option<Millis>) -> Option<i64> {
    m.map(|m| m.0)
}

pub(crate) fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

pub(crate) fn path_opt(p: &Option<PathBuf>) -> Option<String> {
    p.as_deref().map(path_str)
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.trim().is_empty())
}

fn checksum_str(c: &Option<d::Checksum>) -> Option<String> {
    c.as_ref()
        .map(|c| format!("{}:{}", c.algorithm.as_str(), c.value))
}

fn parse_checksum(s: Option<String>) -> FfiResult<Option<d::Checksum>> {
    match non_empty(s) {
        None => Ok(None),
        Some(v) => d::Checksum::parse(&v)
            .map(Some)
            .ok_or_else(|| FfiError::validation(format!("unrecognised checksum `{v}`"))),
    }
}

/// Mirror a fieldless domain enum as a UniFFI enum with `From` both ways.
macro_rules! mirror_enum {
    ($(#[$m:meta])* $ffi:ident <=> $dom:path { $($v:ident),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, uniffi::Enum)]
        pub enum $ffi { $($v),+ }

        impl From<$dom> for $ffi {
            fn from(v: $dom) -> Self {
                use $dom as D;
                match v { $(D::$v => $ffi::$v),+ }
            }
        }
        impl From<$ffi> for $dom {
            fn from(v: $ffi) -> Self {
                use $dom as D;
                match v { $($ffi::$v => D::$v),+ }
            }
        }
    };
}

mirror_enum!(
    /// Task lifecycle state.
    FfiTaskState <=> d::TaskState {
        Pending, Queued, Scheduled, Resolving, Connecting, Downloading, Paused, Retrying,
        Verifying, Processing, Completed, Failed, Cancelled, Seeding,
    }
);
mirror_enum!(FfiTaskKind <=> d::TaskKind { Http, Ftp, Torrent, Magnet, Metalink, Hls });
mirror_enum!(FfiPriority <=> d::Priority { Low, Normal, High, Urgent });
mirror_enum!(FfiConflictPolicy <=> d::ConflictPolicy { Ask, Replace, Rename, Skip, KeepBoth });
mirror_enum!(FfiTrafficMode <=> d::queue::TrafficMode { Unlimited, FullSpeed, Balanced, Browsing, Custom });
mirror_enum!(FfiScope <=> d::device::Scope { Read, Add, Control, Admin });
mirror_enum!(FfiLogLevel <=> d::LogLevel { Trace, Debug, Info, Warn, Error });
mirror_enum!(FfiMediaKind <=> d::media::MediaKind { Video, Audio, Image, HlsPlaylist, DashManifest, Unknown });
mirror_enum!(FfiTaskSort <=> s::TaskSort { Position, CreatedAt, Name, Size, Progress, Speed, Eta, State, Domain });
mirror_enum!(FfiHistorySort <=> d::history::HistorySort { FinishedAt, Name, Size, Domain, Duration, Speed });
mirror_enum!(
    /// Fine-grained failure kind (Swift localises `error.<kind>`).
    FfiErrorKind <=> d::ErrorKind {
        InvalidUrl, UnsupportedScheme, DnsFailure, TlsFailure, CertificateInvalid,
        ConnectionRefused, ConnectionTimeout, ConnectionReset, ReadTimeout, ProxyError,
        AuthenticationRequired, Forbidden, NotFound, Throttled, ServerError, RangeNotSupported,
        SourceChanged, ExpiredUrl, RedirectLoop, Truncated, ChecksumMismatch, DiskFull,
        DiskWriteError, DiskReadError, PermissionDenied, VolumeUnavailable, InvalidFilename,
        PathTraversal, FileExists, InvalidTorrent, TrackerFailure, NoPeers, DhtUnavailable,
        ParseError, ProtectedContent, MirrorExhausted, NetworkUnavailable, UnexpectedContent,
        LiveStreamUnsupported, QuotaExceeded, Cancelled, Internal, Unknown,
    }
);

// ---------------------------------------------------------------------------------------------
// engine config / info / environment
// ---------------------------------------------------------------------------------------------

/// Passed to `SwoopEngine.open`.
#[derive(Clone, Debug, uniffi::Record)]
pub struct FfiEngineConfig {
    /// Data directory; `nil` = platform default (`~/Library/Application Support/app.swoop.Swoop`).
    #[uniffi(default = None)]
    pub data_dir: Option<String>,
    #[uniffi(default = false)]
    pub headless: bool,
    /// `trace` … `error`; empty = the stored setting.
    #[uniffi(default = "info")]
    pub log_level: String,
    #[uniffi(default = "0.1.0")]
    pub app_version: String,
    #[uniffi(default = "app.swoop.desktop")]
    pub bundle_id: String,
    /// Override the download directory (tests / first-run choice).
    #[uniffi(default = None)]
    pub download_dir: Option<String>,
    /// Start the local Unix-socket API used by the CLI and the browser native host.
    #[uniffi(default = true)]
    pub start_local_api: bool,
    /// Tests only: allow two engines on one data directory.
    #[uniffi(default = false)]
    pub skip_instance_lock: bool,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct FfiEngineInfo {
    pub version: String,
    pub build: String,
    pub os: String,
    pub arch: String,
    pub bundle_id: String,
    pub data_dir: String,
    pub log_dir: String,
    pub local_api_port: u16,
    pub remote_enabled: bool,
    pub remote_port: Option<u16>,
    pub uptime_seconds: u64,
    pub headless: bool,
    pub ffmpeg_available: bool,
    /// Unix socket of the local API (CLI / native messaging host), when running.
    pub socket_path: Option<String>,
    /// Loopback TCP address of the local API, when running.
    pub local_address: Option<String>,
    /// Remote listener address, when running.
    pub remote_address: Option<String>,
    /// SHA-256 fingerprint of the remote listener's TLS certificate.
    pub tls_fingerprint: Option<String>,
}

/// Measurements from the platform (NWPathMonitor, IOKit power sources, utun interfaces).
#[derive(Clone, Debug, uniffi::Record)]
pub struct FfiEnvironment {
    pub network_available: bool,
    pub metered: bool,
    pub on_ac_power: bool,
    #[uniffi(default = None)]
    pub battery_percent: Option<u8>,
    #[uniffi(default = false)]
    pub vpn_active: bool,
    #[uniffi(default = None)]
    pub ssid: Option<String>,
}

impl From<FfiEnvironment> for d::schedule::EnvironmentSnapshot {
    fn from(e: FfiEnvironment) -> Self {
        Self {
            network_available: e.network_available,
            metered: e.metered,
            on_ac_power: e.on_ac_power,
            battery_percent: e.battery_percent,
            vpn_active: e.vpn_active,
            ssid: e.ssid,
            download_speed: 0,
            active_transfers: 0,
            at: Millis::now(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// task rows / progress
// ---------------------------------------------------------------------------------------------

/// The light row the Downloads table binds to. Every row carries the task `rev`; the app drops
/// rows/progress older than what it holds.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiTaskRow {
    pub id: String,
    pub rev: u64,
    pub name: String,
    pub kind: FfiTaskKind,
    pub state: FfiTaskState,
    pub domain: Option<String>,
    /// Primary URL / magnet (not set for `.torrent` file sources).
    pub url: Option<String>,
    pub downloaded: u64,
    pub uploaded: u64,
    pub total: Option<u64>,
    /// 0…1 for the progress bar (derived from bytes when the size is known).
    pub fraction: f32,
    pub speed: u64,
    pub upload_speed: u64,
    pub eta_seconds: Option<u64>,
    pub active_connections: u32,
    pub peers: u32,
    pub seeds: u32,
    pub ratio: f32,
    pub queue_id: String,
    pub category_id: Option<String>,
    pub schedule_id: Option<String>,
    pub priority: FfiPriority,
    pub position: i64,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub error_kind: Option<FfiErrorKind>,
    pub error_message: Option<String>,
    /// Health score 0–100.
    pub health: u8,
    pub directory: String,
    pub file_path: Option<String>,
    pub status_detail: Option<String>,
    /// Localisation keys of everything blocking the task (`pause.user`, `pause.schedule`, …).
    pub blocked_by: Vec<String>,
    pub tags: Vec<String>,
}

pub(crate) fn fraction(p: &d::Progress) -> f32 {
    match p.total {
        Some(t) if t > 0 => (p.downloaded as f64 / t as f64).clamp(0.0, 1.0) as f32,
        _ => p.fraction.clamp(0.0, 1.0),
    }
}

impl From<&d::Task> for FfiTaskRow {
    fn from(t: &d::Task) -> Self {
        let p = &t.progress;
        Self {
            id: t.id.0.clone(),
            rev: t.rev,
            name: t.name.clone(),
            kind: t.kind.into(),
            state: t.state.into(),
            domain: t.domain(),
            url: t.source.primary_url().map(str::to_owned),
            downloaded: p.downloaded,
            uploaded: p.uploaded,
            total: p.total,
            fraction: fraction(p),
            speed: p.speed,
            upload_speed: p.upload_speed,
            eta_seconds: p.eta_seconds,
            active_connections: p.active_connections,
            peers: p.peers,
            seeds: p.seeds,
            ratio: p.ratio,
            queue_id: t.queue_id.0.clone(),
            category_id: t.category_id.as_ref().map(|c| c.0.clone()),
            schedule_id: t.schedule_id.as_ref().map(|c| c.0.clone()),
            priority: t.priority.into(),
            position: t.position,
            created_at: ms(t.created_at),
            completed_at: ms_opt(t.completed_at),
            error_kind: t.error.as_ref().map(|e| e.kind.into()),
            error_message: t.error.as_ref().map(|e| e.message.clone()),
            health: t.health.score,
            directory: path_str(&t.directory),
            file_path: path_opt(&t.file_path),
            status_detail: t.status_detail.clone(),
            blocked_by: t
                .blocked_by
                .iter()
                .map(|r| r.label_key().to_owned())
                .collect(),
            tags: t.tags.clone(),
        }
    }
}

impl From<&s::TaskRow> for FfiTaskRow {
    /// Lossy fallback used only when the full task is gone (e.g. dashboard "recent").
    fn from(r: &s::TaskRow) -> Self {
        let p = &r.progress;
        Self {
            id: r.id.0.clone(),
            rev: 0,
            name: r.name.clone(),
            kind: r.kind.into(),
            state: r.state.into(),
            domain: r.domain.clone(),
            url: None,
            downloaded: p.downloaded,
            uploaded: p.uploaded,
            total: p.total,
            fraction: fraction(p),
            speed: p.speed,
            upload_speed: p.upload_speed,
            eta_seconds: p.eta_seconds,
            active_connections: p.active_connections,
            peers: p.peers,
            seeds: p.seeds,
            ratio: p.ratio,
            queue_id: r.queue_id.0.clone(),
            category_id: r.category_id.as_ref().map(|c| c.0.clone()),
            schedule_id: None,
            priority: r.priority.into(),
            position: r.position,
            created_at: ms(r.created_at),
            completed_at: None,
            error_kind: r.error_kind.map(Into::into),
            error_message: None,
            health: r.health,
            directory: r
                .file_path
                .as_deref()
                .and_then(Path::parent)
                .map(path_str)
                .unwrap_or_default(),
            file_path: path_opt(&r.file_path),
            status_detail: None,
            blocked_by: Vec::new(),
            tags: Vec::new(),
        }
    }
}

/// One coalesced progress sample.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiProgress {
    pub task_id: String,
    pub rev: u64,
    pub downloaded: u64,
    pub uploaded: u64,
    pub total: Option<u64>,
    pub fraction: f32,
    pub speed: u64,
    pub upload_speed: u64,
    pub eta_seconds: Option<u64>,
    pub active_connections: u32,
    pub peers: u32,
    pub seeds: u32,
    pub ratio: f32,
}

impl From<&d::ProgressUpdate> for FfiProgress {
    fn from(u: &d::ProgressUpdate) -> Self {
        let p = &u.progress;
        Self {
            task_id: u.task_id.0.clone(),
            rev: u.rev,
            downloaded: p.downloaded,
            uploaded: p.uploaded,
            total: p.total,
            fraction: fraction(p),
            speed: p.speed,
            upload_speed: p.upload_speed,
            eta_seconds: p.eta_seconds,
            active_connections: p.active_connections,
            peers: p.peers,
            seeds: p.seeds,
            ratio: p.ratio,
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiLogEntry {
    pub task_id: String,
    pub at: i64,
    pub level: FfiLogLevel,
    pub code: String,
    pub message: String,
}

impl From<&d::TaskLogEntry> for FfiLogEntry {
    fn from(l: &d::TaskLogEntry) -> Self {
        Self {
            task_id: l.task_id.0.clone(),
            at: ms(l.at),
            level: l.level.into(),
            code: l.code.clone(),
            message: l.message.clone(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// task detail
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiTaskError {
    pub kind: FfiErrorKind,
    pub message: String,
    pub status_code: Option<u16>,
    pub detail: Option<String>,
    pub source_url: Option<String>,
    pub retry_after_ms: Option<u64>,
    pub at: i64,
    /// Localisation key of the plain-language explanation.
    pub explanation_key: String,
    pub retryable: bool,
}

impl From<&d::TaskError> for FfiTaskError {
    fn from(e: &d::TaskError) -> Self {
        Self {
            kind: e.kind.into(),
            message: e.message.clone(),
            status_code: e.status_code,
            detail: e.detail.clone(),
            source_url: e.source_url.clone(),
            retry_after_ms: e.retry_after_ms,
            at: ms(e.at),
            explanation_key: e.kind.explanation_key().to_owned(),
            retryable: e.kind.is_retryable(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiTaskStats {
    pub average_speed: u64,
    pub peak_speed: u64,
    pub retries: u32,
    pub failed_connections: u32,
    pub segments_reassigned: u32,
    pub mirrors_switched: u32,
    pub active_seconds: u64,
    pub bytes_discarded: u64,
    pub throughput_drops: u32,
    pub range_supported: Option<bool>,
    pub http_version: Option<String>,
    pub final_url: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub server: Option<String>,
    pub content_type: Option<String>,
    pub content_disposition: Option<String>,
    pub remote_addr: Option<String>,
}

impl From<&d::TaskStats> for FfiTaskStats {
    fn from(s: &d::TaskStats) -> Self {
        Self {
            average_speed: s.average_speed,
            peak_speed: s.peak_speed,
            retries: s.retries,
            failed_connections: s.failed_connections,
            segments_reassigned: s.segments_reassigned,
            mirrors_switched: s.mirrors_switched,
            active_seconds: s.active_seconds,
            bytes_discarded: s.bytes_discarded,
            throughput_drops: s.throughput_drops,
            range_supported: s.range_supported,
            http_version: s.http_version.clone(),
            final_url: s.final_url.clone(),
            etag: s.etag.clone(),
            last_modified: s.last_modified.clone(),
            server: s.server.clone(),
            content_type: s.content_type.clone(),
            content_disposition: s.content_disposition.clone(),
            remote_addr: s.remote_addr.clone(),
        }
    }
}

/// Health score with the inputs that produced it (never a black box).
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiHealth {
    pub score: u8,
    /// `health.excellent` / `good` / `fair` / `poor`.
    pub label_key: String,
    pub source_stability: u8,
    pub throughput_consistency: u8,
    pub connection_quality: u8,
    pub retry_pressure: u8,
    pub remaining_risk: u8,
    pub notes: Vec<String>,
}

impl From<&d::health::HealthScore> for FfiHealth {
    fn from(h: &d::health::HealthScore) -> Self {
        Self {
            score: h.score,
            label_key: h.label_key().to_owned(),
            source_stability: h.source_stability,
            throughput_consistency: h.throughput_consistency,
            connection_quality: h.connection_quality,
            retry_pressure: h.retry_pressure,
            remaining_risk: h.remaining_risk,
            notes: h.notes.clone(),
        }
    }
}

/// One byte-range segment (Connections inspector tab).
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiSegment {
    pub index: u32,
    pub start: u64,
    /// Exclusive.
    pub end: u64,
    pub committed: u64,
    pub source_index: u32,
    pub done: bool,
}

impl From<&d::Segment> for FfiSegment {
    fn from(s: &d::Segment) -> Self {
        Self {
            index: s.index,
            start: s.start,
            end: s.end,
            committed: s.committed,
            source_index: s.source_index,
            done: s.is_done(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiPauseReason {
    /// `pause.user`, `pause.queue`, `pause.schedule`, `pause.condition`, `pause.shutdown`,
    /// `pause.disk_space`, `pause.network`, `pause.volume`.
    pub key: String,
    pub detail: Option<String>,
}

impl From<&d::PauseReason> for FfiPauseReason {
    fn from(r: &d::PauseReason) -> Self {
        let detail = match r {
            d::PauseReason::Queue(s)
            | d::PauseReason::Schedule(s)
            | d::PauseReason::Condition(s) => Some(s.clone()),
            _ => None,
        };
        Self {
            key: r.label_key().to_owned(),
            detail,
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiTorrentFile {
    pub index: u32,
    pub path: String,
    pub size: u64,
    pub downloaded: u64,
    pub selected: bool,
    /// 0 low, 1 normal, 2 high.
    pub priority: u8,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiTracker {
    pub url: String,
    pub tier: u32,
    pub enabled: bool,
    pub last_announce_at: Option<i64>,
    pub next_announce_at: Option<i64>,
    pub seeders: Option<u32>,
    pub leechers: Option<u32>,
    pub latency_ms: Option<u32>,
    pub last_error: Option<String>,
    /// `working`, `updating`, `error`, `disabled`, `dead`.
    pub health: String,
    pub consecutive_failures: u32,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiTorrentInfo {
    pub info_hash: String,
    pub name: String,
    pub total_size: u64,
    pub piece_length: u32,
    pub piece_count: u32,
    pub files: Vec<FfiTorrentFile>,
    pub trackers: Vec<FfiTracker>,
    pub private: bool,
    pub comment: Option<String>,
    pub created_by: Option<String>,
    pub magnet: Option<String>,
    pub have_metadata: bool,
    pub seeders_total: u32,
    pub leechers_total: u32,
    pub connected_peers: u32,
    pub connected_seeds: u32,
    pub dht_nodes: u32,
    pub uploaded: u64,
    pub ratio: f32,
    pub availability: f32,
    /// Run-length encoded piece map (alternating missing/have runs).
    pub piece_map_rle: Vec<u32>,
    pub seeding_since: Option<i64>,
}

impl From<&d::torrent::TorrentInfo> for FfiTorrentInfo {
    fn from(t: &d::torrent::TorrentInfo) -> Self {
        Self {
            info_hash: t.info_hash.clone(),
            name: t.name.clone(),
            total_size: t.total_size,
            piece_length: t.piece_length,
            piece_count: t.piece_count,
            files: t
                .files
                .iter()
                .map(|f| FfiTorrentFile {
                    index: f.index,
                    path: f.path.clone(),
                    size: f.size,
                    downloaded: f.downloaded,
                    selected: f.selected,
                    priority: f.priority,
                })
                .collect(),
            trackers: t
                .trackers
                .iter()
                .map(|x| FfiTracker {
                    url: x.url.clone(),
                    tier: x.tier,
                    enabled: x.enabled,
                    last_announce_at: ms_opt(x.last_announce_at),
                    next_announce_at: ms_opt(x.next_announce_at),
                    seeders: x.seeders,
                    leechers: x.leechers,
                    latency_ms: x.latency_ms,
                    last_error: x.last_error.clone(),
                    health: x.health.clone(),
                    consecutive_failures: x.consecutive_failures,
                })
                .collect(),
            private: t.private,
            comment: t.comment.clone(),
            created_by: t.created_by.clone(),
            magnet: t.magnet.clone(),
            have_metadata: t.have_metadata,
            seeders_total: t.seeders_total,
            leechers_total: t.leechers_total,
            connected_peers: t.connected_peers,
            connected_seeds: t.connected_seeds,
            dht_nodes: t.dht_nodes,
            uploaded: t.uploaded,
            ratio: t.ratio,
            availability: t.availability,
            piece_map_rle: t.piece_map_rle.clone(),
            seeding_since: ms_opt(t.seeding_since),
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiPeer {
    pub address: String,
    pub client: Option<String>,
    pub download_speed: u64,
    pub upload_speed: u64,
    pub progress: f32,
    pub flags: String,
    pub downloaded: u64,
    pub uploaded: u64,
}

impl From<&d::torrent::PeerInfo> for FfiPeer {
    fn from(p: &d::torrent::PeerInfo) -> Self {
        Self {
            address: p.address.clone(),
            client: p.client.clone(),
            download_speed: p.download_speed,
            upload_speed: p.upload_speed,
            progress: p.progress,
            flags: p.flags.clone(),
            downloaded: p.downloaded,
            uploaded: p.uploaded,
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiFileSelection {
    pub index: u32,
    pub selected: bool,
    /// 0 low, 1 normal, 2 high.
    #[uniffi(default = 1)]
    pub priority: u8,
}

impl From<FfiFileSelection> for s::FileSelection {
    fn from(f: FfiFileSelection) -> Self {
        Self {
            index: f.index,
            selected: f.selected,
            priority: f.priority,
        }
    }
}

/// `nil` fields inherit the global torrent settings.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiSeedingLimits {
    #[uniffi(default = None)]
    pub ratio_limit: Option<f32>,
    #[uniffi(default = None)]
    pub time_limit_minutes: Option<u32>,
    #[uniffi(default = None)]
    pub upload_limit: Option<u64>,
    #[uniffi(default = None)]
    pub seed_when_complete: Option<bool>,
}

impl From<FfiSeedingLimits> for d::torrent::SeedingLimits {
    fn from(l: FfiSeedingLimits) -> Self {
        Self {
            ratio_limit: l.ratio_limit,
            time_limit_minutes: l.time_limit_minutes,
            upload_limit: l.upload_limit,
            seed_when_complete: l.seed_when_complete,
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiMediaVariant {
    pub id: String,
    pub label: String,
    pub url: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bandwidth: Option<u64>,
    pub codecs: Option<String>,
    pub frame_rate: Option<f32>,
    pub estimated_size: Option<u64>,
    pub audio_only: bool,
    pub container: Option<String>,
}

impl From<&d::media::MediaVariant> for FfiMediaVariant {
    fn from(v: &d::media::MediaVariant) -> Self {
        Self {
            id: v.id.clone(),
            label: v.label.clone(),
            url: v.url.clone(),
            width: v.width,
            height: v.height,
            bandwidth: v.bandwidth,
            codecs: v.codecs.clone(),
            frame_rate: v.frame_rate,
            estimated_size: v.estimated_size,
            audio_only: v.audio_only,
            container: v.container.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiMediaInfo {
    pub kind: FfiMediaKind,
    pub title: Option<String>,
    pub format: Option<String>,
    pub duration_seconds: Option<f64>,
    pub variants: Vec<FfiMediaVariant>,
    pub selected_variant: Option<String>,
    pub segment_count: Option<u32>,
    pub segments_done: u32,
    pub protected: bool,
    pub page_url: Option<String>,
}

impl From<&d::media::MediaInfo> for FfiMediaInfo {
    fn from(m: &d::media::MediaInfo) -> Self {
        Self {
            kind: m.kind.into(),
            title: m.title.clone(),
            format: m.format.clone(),
            duration_seconds: m.duration_seconds,
            variants: m.variants.iter().map(Into::into).collect(),
            selected_variant: m.selected_variant.clone(),
            segment_count: m.segment_count,
            segments_done: m.segments_done,
            protected: m.protected,
            page_url: m.page_url.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiDetectedMedia {
    pub url: String,
    pub kind: FfiMediaKind,
    pub title: Option<String>,
    pub mime: Option<String>,
    pub size: Option<u64>,
    pub page_url: Option<String>,
    pub variants: Vec<FfiMediaVariant>,
    pub protected: bool,
}

impl From<&d::media::DetectedMedia> for FfiDetectedMedia {
    fn from(m: &d::media::DetectedMedia) -> Self {
        Self {
            url: m.url.clone(),
            kind: m.kind.into(),
            title: m.title.clone(),
            mime: m.mime.clone(),
            size: m.size,
            page_url: m.page_url.clone(),
            variants: m.variants.iter().map(Into::into).collect(),
            protected: m.protected,
        }
    }
}

/// Per-task options. `nil` inherits from the queue / global settings.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiTaskOptions {
    #[uniffi(default = None)]
    pub max_connections: Option<u8>,
    /// Bytes/s; `0` = explicitly unlimited.
    #[uniffi(default = None)]
    pub download_limit: Option<u64>,
    #[uniffi(default = None)]
    pub upload_limit: Option<u64>,
    pub headers: HashMap<String, String>,
    #[uniffi(default = None)]
    pub user_agent: Option<String>,
    #[uniffi(default = None)]
    pub referer: Option<String>,
    #[uniffi(default = None)]
    pub cookies: Option<String>,
    #[uniffi(default = None)]
    pub credential_id: Option<String>,
    #[uniffi(default = None)]
    pub proxy_id: Option<String>,
    #[uniffi(default = false)]
    pub direct_connection: bool,
    /// `sha256:<hex>`, `md5=<hex>` or bare hex.
    #[uniffi(default = None)]
    pub checksum: Option<String>,
    pub conflict_policy: FfiConflictPolicy,
    #[uniffi(default = None)]
    pub preallocate: Option<bool>,
    #[uniffi(default = None)]
    pub sparse: Option<bool>,
    #[uniffi(default = None)]
    pub adaptive: Option<bool>,
    #[uniffi(default = None)]
    pub max_retries: Option<u32>,
    #[uniffi(default = None)]
    pub sequential: Option<bool>,
    #[uniffi(default = None)]
    pub seed_ratio_limit: Option<f32>,
    #[uniffi(default = None)]
    pub seed_time_limit_minutes: Option<u32>,
    #[uniffi(default = None)]
    pub max_peers: Option<u32>,
    #[uniffi(default = None)]
    pub media_variant: Option<String>,
    #[uniffi(default = None)]
    pub allow_http2: Option<bool>,
    #[uniffi(default = false)]
    pub open_when_done: bool,
}

impl From<&d::TaskOptions> for FfiTaskOptions {
    fn from(o: &d::TaskOptions) -> Self {
        Self {
            max_connections: o.max_connections,
            download_limit: o.download_limit,
            upload_limit: o.upload_limit,
            headers: o.headers.clone().into_iter().collect(),
            user_agent: o.user_agent.clone(),
            referer: o.referer.clone(),
            cookies: o.cookies.clone(),
            credential_id: o.credential.as_ref().map(|c| c.0.clone()),
            proxy_id: o.proxy.as_ref().map(|c| c.0.clone()),
            direct_connection: o.direct_connection,
            checksum: checksum_str(&o.checksum),
            conflict_policy: o.conflict_policy.into(),
            preallocate: o.preallocate,
            sparse: o.sparse,
            adaptive: o.adaptive,
            max_retries: o.max_retries,
            sequential: o.sequential,
            seed_ratio_limit: o.seed_ratio_limit,
            seed_time_limit_minutes: o.seed_time_limit_minutes,
            max_peers: o.max_peers,
            media_variant: o.media_variant.clone(),
            allow_http2: o.allow_http2,
            open_when_done: o.open_when_done,
        }
    }
}

impl TryFrom<FfiTaskOptions> for d::TaskOptions {
    type Error = FfiError;
    fn try_from(o: FfiTaskOptions) -> FfiResult<Self> {
        Ok(Self {
            max_connections: o.max_connections,
            download_limit: o.download_limit,
            upload_limit: o.upload_limit,
            headers: o.headers.into_iter().collect(),
            user_agent: non_empty(o.user_agent),
            referer: non_empty(o.referer),
            cookies: non_empty(o.cookies),
            credential: non_empty(o.credential_id).map(d::CredentialId),
            proxy: non_empty(o.proxy_id).map(d::ProxyId),
            direct_connection: o.direct_connection,
            checksum: parse_checksum(o.checksum)?,
            conflict_policy: o.conflict_policy.into(),
            preallocate: o.preallocate,
            sparse: o.sparse,
            adaptive: o.adaptive,
            max_retries: o.max_retries,
            sequential: o.sequential,
            seed_ratio_limit: o.seed_ratio_limit,
            seed_time_limit_minutes: o.seed_time_limit_minutes,
            max_peers: o.max_peers,
            media_variant: non_empty(o.media_variant),
            allow_http2: o.allow_http2,
            open_when_done: o.open_when_done,
        })
    }
}

/// Everything the inspector shows for the selected task. Peers are fetched separately
/// (`torrent_peers`) because they change every second.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiTaskDetail {
    pub row: FfiTaskRow,
    /// All source URLs (primary first, then mirrors).
    pub urls: Vec<String>,
    /// The raw `Source` as JSON (`{"type":"urls",...}`), for the Network tab.
    pub source_json: String,
    pub origin: String,
    pub mime: Option<String>,
    pub updated_at: i64,
    pub started_at: Option<i64>,
    pub next_retry_at: Option<i64>,
    pub attempt: u32,
    pub name_locked: bool,
    pub directory_locked: bool,
    pub verified_checksum: Option<String>,
    pub options: FfiTaskOptions,
    pub error: Option<FfiTaskError>,
    pub stats: FfiTaskStats,
    pub health: FfiHealth,
    pub blocked_by: Vec<FfiPauseReason>,
    pub segments: Vec<FfiSegment>,
    pub segment_total: Option<u64>,
    pub part_path: Option<String>,
    pub torrent: Option<FfiTorrentInfo>,
    pub media: Option<FfiMediaInfo>,
    /// Newest task log lines (oldest first).
    pub log_tail: Vec<FfiLogEntry>,
}

impl FfiTaskDetail {
    pub(crate) fn build(t: &d::Task, log: &[d::TaskLogEntry]) -> Self {
        let map = t.segment_map.as_ref();
        Self {
            row: t.into(),
            urls: t.source.urls(),
            source_json: serde_json::to_string(&t.source).unwrap_or_default(),
            origin: t.origin.clone(),
            mime: t.mime.clone(),
            updated_at: ms(t.updated_at),
            started_at: ms_opt(t.started_at),
            next_retry_at: ms_opt(t.next_retry_at),
            attempt: t.attempt,
            name_locked: t.name_locked,
            directory_locked: t.directory_locked,
            verified_checksum: checksum_str(&t.verified_checksum),
            options: (&t.options).into(),
            error: t.error.as_ref().map(Into::into),
            stats: (&t.stats).into(),
            health: (&t.health).into(),
            blocked_by: t.blocked_by.iter().map(Into::into).collect(),
            segments: map
                .map(|m| m.segments.iter().map(Into::into).collect())
                .unwrap_or_default(),
            segment_total: map.and_then(|m| m.total),
            part_path: map.and_then(|m| path_opt(&m.part_path)),
            torrent: t.torrent.as_ref().map(Into::into),
            media: t.media.as_ref().map(Into::into),
            log_tail: log.iter().map(Into::into).collect(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// requests
// ---------------------------------------------------------------------------------------------

/// Input for the Add sheet, the browser extension hand-off, the grabber, recipes.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiNewTaskRequest {
    #[uniffi(default = None)]
    pub url: Option<String>,
    #[uniffi(default = [])]
    pub mirrors: Vec<String>,
    #[uniffi(default = None)]
    pub magnet: Option<String>,
    /// Base64 `.torrent` bytes.
    #[uniffi(default = None)]
    pub torrent_base64: Option<String>,
    #[uniffi(default = None)]
    pub metalink_url: Option<String>,
    #[uniffi(default = None)]
    pub hls_playlist_url: Option<String>,
    #[uniffi(default = None)]
    pub name: Option<String>,
    #[uniffi(default = None)]
    pub directory: Option<String>,
    #[uniffi(default = None)]
    pub queue_id: Option<String>,
    #[uniffi(default = None)]
    pub category_id: Option<String>,
    #[uniffi(default = None)]
    pub schedule_id: Option<String>,
    #[uniffi(default = None)]
    pub priority: Option<FfiPriority>,
    #[uniffi(default = [])]
    pub tags: Vec<String>,
    #[uniffi(default = None)]
    pub options: Option<FfiTaskOptions>,
    /// Start immediately, or leave `Pending` for confirmation.
    #[uniffi(default = true)]
    pub start: bool,
    #[uniffi(default = "app")]
    pub origin: String,
    /// Torrent: selected file indices (`nil` = all).
    #[uniffi(default = None)]
    pub selected_files: Option<Vec<u32>>,
    #[uniffi(default = None)]
    pub referer_page: Option<String>,
}

impl TryFrom<FfiNewTaskRequest> for d::NewTaskRequest {
    type Error = FfiError;
    fn try_from(r: FfiNewTaskRequest) -> FfiResult<Self> {
        Ok(Self {
            url: non_empty(r.url),
            mirrors: r.mirrors,
            magnet: non_empty(r.magnet),
            torrent_base64: non_empty(r.torrent_base64),
            metalink_url: non_empty(r.metalink_url),
            hls_playlist_url: non_empty(r.hls_playlist_url),
            name: non_empty(r.name),
            directory: non_empty(r.directory).map(PathBuf::from),
            queue_id: non_empty(r.queue_id).map(d::QueueId),
            category_id: non_empty(r.category_id).map(d::CategoryId),
            schedule_id: non_empty(r.schedule_id).map(d::ScheduleId),
            priority: r.priority.map(Into::into),
            tags: r.tags,
            options: match r.options {
                Some(o) => o.try_into()?,
                None => d::TaskOptions::default(),
            },
            start: r.start,
            origin: if r.origin.is_empty() {
                "app".to_owned()
            } else {
                r.origin
            },
            selected_files: r.selected_files,
            referer_page: non_empty(r.referer_page),
        })
    }
}

/// Filter for `task_rows`. Empty lists / `nil` mean "any".
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiTaskFilter {
    #[uniffi(default = None)]
    pub text: Option<String>,
    #[uniffi(default = [])]
    pub states: Vec<FfiTaskState>,
    #[uniffi(default = [])]
    pub kinds: Vec<FfiTaskKind>,
    #[uniffi(default = None)]
    pub queue_id: Option<String>,
    #[uniffi(default = None)]
    pub category_id: Option<String>,
    #[uniffi(default = None)]
    pub domain: Option<String>,
    #[uniffi(default = None)]
    pub tag: Option<String>,
    /// `active`, `queued`, `scheduled`, `complete`, `failed`, `torrent`, `media`, `paused`.
    #[uniffi(default = None)]
    pub smart: Option<String>,
    #[uniffi(default = None)]
    pub created_since: Option<i64>,
    #[uniffi(default = None)]
    pub min_size: Option<u64>,
    #[uniffi(default = None)]
    pub max_size: Option<u64>,
    #[uniffi(default = None)]
    pub sort: Option<FfiTaskSort>,
    #[uniffi(default = false)]
    pub descending: bool,
    /// 0 = no limit.
    #[uniffi(default = 0)]
    pub limit: u32,
    #[uniffi(default = 0)]
    pub offset: u32,
}

impl From<FfiTaskFilter> for s::TaskFilter {
    fn from(f: FfiTaskFilter) -> Self {
        Self {
            text: non_empty(f.text),
            states: f.states.into_iter().map(Into::into).collect(),
            kinds: f.kinds.into_iter().map(Into::into).collect(),
            queue_id: non_empty(f.queue_id).map(d::QueueId),
            category_id: non_empty(f.category_id).map(d::CategoryId),
            domain: non_empty(f.domain),
            tag: non_empty(f.tag),
            smart: non_empty(f.smart),
            created_since: f.created_since.map(Millis),
            min_size: f.min_size,
            max_size: f.max_size,
            sort: f.sort.map(Into::into).unwrap_or_default(),
            descending: f.descending,
            limit: f.limit,
            offset: f.offset,
        }
    }
}

/// Partial edit of a task. `nil` = unchanged; `clear_*` removes the assignment.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiTaskPatch {
    #[uniffi(default = None)]
    pub name: Option<String>,
    #[uniffi(default = None)]
    pub directory: Option<String>,
    #[uniffi(default = None)]
    pub queue_id: Option<String>,
    #[uniffi(default = None)]
    pub category_id: Option<String>,
    #[uniffi(default = false)]
    pub clear_category: bool,
    #[uniffi(default = None)]
    pub schedule_id: Option<String>,
    #[uniffi(default = false)]
    pub clear_schedule: bool,
    #[uniffi(default = None)]
    pub priority: Option<FfiPriority>,
    #[uniffi(default = None)]
    pub tags: Option<Vec<String>>,
    #[uniffi(default = None)]
    pub options: Option<FfiTaskOptions>,
    #[uniffi(default = None)]
    pub mirrors: Option<Vec<String>>,
}

impl TryFrom<FfiTaskPatch> for s::TaskPatch {
    type Error = FfiError;
    fn try_from(p: FfiTaskPatch) -> FfiResult<Self> {
        let category_id = if p.clear_category {
            Some(None)
        } else {
            p.category_id.map(|c| Some(d::CategoryId(c)))
        };
        let schedule_id = if p.clear_schedule {
            Some(None)
        } else {
            p.schedule_id.map(|c| Some(d::ScheduleId(c)))
        };
        Ok(Self {
            name: p.name,
            directory: p.directory.map(PathBuf::from),
            queue_id: p.queue_id.map(d::QueueId),
            category_id,
            schedule_id,
            priority: p.priority.map(Into::into),
            tags: p.tags,
            options: p.options.map(TryInto::try_into).transpose()?,
            mirrors: p.mirrors,
        })
    }
}

/// Every per-task command of the context menu.
#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum FfiTaskAction {
    Start,
    Pause,
    Resume,
    /// From scratch, discarding partial data.
    Restart,
    /// Keep partial data.
    Retry,
    /// Re-resolve the source (optionally with a fresh URL) and continue.
    RetryFromSource {
        new_url: Option<String>,
    },
    Cancel,
    /// New task for the same source; returns the new row.
    Redownload,
    Verify {
        checksum: Option<String>,
    },
    RetrySegments,
    /// Copy of the task; returns the new row.
    Duplicate,
}

// ---------------------------------------------------------------------------------------------
// probe / add
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiDuplicateInfo {
    /// `path`, `filename`, `size`, `checksum`, `url_history`.
    pub matched_by: String,
    pub existing_path: Option<String>,
    pub existing_task_id: Option<String>,
    pub existing_size: Option<u64>,
    pub existing_checksum: Option<String>,
    pub existing_completed_at: Option<i64>,
}

impl From<&s::DuplicateInfo> for FfiDuplicateInfo {
    fn from(x: &s::DuplicateInfo) -> Self {
        Self {
            matched_by: x.matched_by.clone(),
            existing_path: path_opt(&x.existing_path),
            existing_task_id: x.existing_task_id.as_ref().map(|i| i.0.clone()),
            existing_size: x.existing_size,
            existing_checksum: checksum_str(&x.existing_checksum),
            existing_completed_at: ms_opt(x.existing_completed_at),
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiProbeResult {
    pub kind: FfiTaskKind,
    pub suggested_name: String,
    pub suggested_directory: String,
    pub suggested_queue: String,
    pub suggested_category: Option<String>,
    pub total: Option<u64>,
    pub mime: Option<String>,
    pub resumable: Option<bool>,
    pub final_url: Option<String>,
    pub server: Option<String>,
    pub http_version: Option<String>,
    pub content_disposition: Option<String>,
    pub torrent: Option<FfiTorrentInfo>,
    pub media: Option<FfiMediaInfo>,
    pub free_space: Option<u64>,
    pub duplicate: Option<FfiDuplicateInfo>,
    pub applicable_rules: Vec<String>,
    pub warnings: Vec<String>,
}

impl From<&s::ProbeResult> for FfiProbeResult {
    fn from(p: &s::ProbeResult) -> Self {
        let m = &p.metadata;
        Self {
            kind: p.kind.into(),
            suggested_name: p.suggested_name.clone(),
            suggested_directory: path_str(&p.suggested_directory),
            suggested_queue: p.suggested_queue.0.clone(),
            suggested_category: p.suggested_category.as_ref().map(|c| c.0.clone()),
            total: m.total,
            mime: m.mime.clone(),
            resumable: m.resumable,
            final_url: m.final_url.clone(),
            server: m.server.clone(),
            http_version: m.http_version.clone(),
            content_disposition: m.content_disposition.clone(),
            torrent: m.torrent.as_ref().map(Into::into),
            media: m.media.as_ref().map(Into::into),
            free_space: p.free_space,
            duplicate: p.duplicate.as_ref().map(Into::into),
            applicable_rules: p.applicable_rules.clone(),
            warnings: p.warnings.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiAddTaskResult {
    pub row: FfiTaskRow,
    /// Set when a duplicate was found; the task waits `Pending` for `resolve_duplicate`.
    pub duplicate: Option<FfiDuplicateInfo>,
}

impl From<&s::AddTaskResult> for FfiAddTaskResult {
    fn from(r: &s::AddTaskResult) -> Self {
        Self {
            row: (&r.task).into(),
            duplicate: r.duplicate.as_ref().map(Into::into),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// queues / categories
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum FfiQueueCompletionAction {
    Nothing,
    Notify,
    RunAutomation { automation_id: String },
    Sleep,
    QuitApplication,
}

/// A queue. To create one pass an empty `id`.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiQueue {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub color: Option<String>,
    /// 0 = unlimited.
    pub max_concurrent: u32,
    /// Bytes/s; 0 = unlimited.
    pub download_limit: u64,
    pub upload_limit: u64,
    /// 0 = global default.
    pub connections_per_task: u8,
    pub schedule_id: Option<String>,
    pub directory: Option<String>,
    pub priority: i32,
    pub completion_action: FfiQueueCompletionAction,
    pub paused: bool,
    pub builtin: bool,
    pub position: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<&d::queue::Queue> for FfiQueue {
    fn from(q: &d::queue::Queue) -> Self {
        use d::queue::QueueCompletionAction as A;
        Self {
            id: q.id.0.clone(),
            name: q.name.clone(),
            icon: q.icon.clone(),
            color: q.color.clone(),
            max_concurrent: q.max_concurrent,
            download_limit: q.bandwidth.download_limit,
            upload_limit: q.bandwidth.upload_limit,
            connections_per_task: q.bandwidth.connections_per_task,
            schedule_id: q.schedule_id.as_ref().map(|s| s.0.clone()),
            directory: path_opt(&q.directory),
            priority: q.priority,
            completion_action: match &q.completion_action {
                A::Nothing => FfiQueueCompletionAction::Nothing,
                A::Notify => FfiQueueCompletionAction::Notify,
                A::RunAutomation { automation_id } => FfiQueueCompletionAction::RunAutomation {
                    automation_id: automation_id.0.clone(),
                },
                A::Sleep => FfiQueueCompletionAction::Sleep,
                A::QuitApplication => FfiQueueCompletionAction::QuitApplication,
            },
            paused: q.paused,
            builtin: q.builtin,
            position: q.position,
            created_at: ms(q.created_at),
            updated_at: ms(q.updated_at),
        }
    }
}

impl From<FfiQueue> for d::queue::Queue {
    fn from(q: FfiQueue) -> Self {
        use d::queue::QueueCompletionAction as A;
        let now = Millis::now();
        Self {
            id: if q.id.is_empty() {
                d::QueueId::new()
            } else {
                d::QueueId(q.id)
            },
            name: q.name,
            icon: q.icon,
            color: non_empty(q.color),
            max_concurrent: q.max_concurrent,
            bandwidth: d::queue::BandwidthProfile {
                download_limit: q.download_limit,
                upload_limit: q.upload_limit,
                connections_per_task: q.connections_per_task,
            },
            schedule_id: non_empty(q.schedule_id).map(d::ScheduleId),
            directory: non_empty(q.directory).map(PathBuf::from),
            priority: q.priority,
            completion_action: match q.completion_action {
                FfiQueueCompletionAction::Nothing => A::Nothing,
                FfiQueueCompletionAction::Notify => A::Notify,
                FfiQueueCompletionAction::RunAutomation { automation_id } => A::RunAutomation {
                    automation_id: d::AutomationId(automation_id),
                },
                FfiQueueCompletionAction::Sleep => A::Sleep,
                FfiQueueCompletionAction::QuitApplication => A::QuitApplication,
            },
            paused: q.paused,
            builtin: q.builtin,
            position: q.position,
            created_at: if q.created_at == 0 {
                now
            } else {
                Millis(q.created_at)
            },
            updated_at: now,
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiQueueSummary {
    pub queue_id: String,
    pub active: u32,
    pub waiting: u32,
    pub completed: u32,
    pub failed: u32,
    pub download_speed: u64,
    pub upload_speed: u64,
}

impl From<&d::queue::QueueSummary> for FfiQueueSummary {
    fn from(q: &d::queue::QueueSummary) -> Self {
        Self {
            queue_id: q.queue_id.0.clone(),
            active: q.active,
            waiting: q.waiting,
            completed: q.completed,
            failed: q.failed,
            download_speed: q.download_speed,
            upload_speed: q.upload_speed,
        }
    }
}

/// A category. To create one pass an empty `id`.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiCategory {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub color: Option<String>,
    pub extensions: Vec<String>,
    pub mime_prefixes: Vec<String>,
    pub directory: Option<String>,
    pub builtin: bool,
    pub position: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<&d::category::Category> for FfiCategory {
    fn from(c: &d::category::Category) -> Self {
        Self {
            id: c.id.0.clone(),
            name: c.name.clone(),
            icon: c.icon.clone(),
            color: c.color.clone(),
            extensions: c.extensions.clone(),
            mime_prefixes: c.mime_prefixes.clone(),
            directory: path_opt(&c.directory),
            builtin: c.builtin,
            position: c.position,
            created_at: ms(c.created_at),
            updated_at: ms(c.updated_at),
        }
    }
}

impl From<FfiCategory> for d::category::Category {
    fn from(c: FfiCategory) -> Self {
        let now = Millis::now();
        Self {
            id: if c.id.is_empty() {
                d::CategoryId::new()
            } else {
                d::CategoryId(c.id)
            },
            name: c.name,
            icon: c.icon,
            color: non_empty(c.color),
            extensions: c.extensions,
            mime_prefixes: c.mime_prefixes,
            directory: non_empty(c.directory).map(PathBuf::from),
            builtin: c.builtin,
            position: c.position,
            created_at: if c.created_at == 0 {
                now
            } else {
                Millis(c.created_at)
            },
            updated_at: now,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// history
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiHistoryQuery {
    #[uniffi(default = None)]
    pub text: Option<String>,
    #[uniffi(default = None)]
    pub domain: Option<String>,
    #[uniffi(default = None)]
    pub state: Option<FfiTaskState>,
    #[uniffi(default = None)]
    pub kind: Option<FfiTaskKind>,
    #[uniffi(default = None)]
    pub category_id: Option<String>,
    #[uniffi(default = None)]
    pub queue_id: Option<String>,
    #[uniffi(default = None)]
    pub since: Option<i64>,
    #[uniffi(default = None)]
    pub until: Option<i64>,
    #[uniffi(default = None)]
    pub min_size: Option<u64>,
    #[uniffi(default = None)]
    pub max_size: Option<u64>,
    #[uniffi(default = None)]
    pub tag: Option<String>,
    #[uniffi(default = None)]
    pub sort: Option<FfiHistorySort>,
    #[uniffi(default = true)]
    pub descending: bool,
    #[uniffi(default = 0)]
    pub limit: u32,
    #[uniffi(default = 0)]
    pub offset: u32,
}

impl From<FfiHistoryQuery> for d::history::HistoryQuery {
    fn from(q: FfiHistoryQuery) -> Self {
        Self {
            text: non_empty(q.text),
            domain: non_empty(q.domain),
            state: q.state.map(Into::into),
            kind: q.kind.map(Into::into),
            category_id: non_empty(q.category_id).map(d::CategoryId),
            queue_id: non_empty(q.queue_id).map(d::QueueId),
            since: q.since.map(Millis),
            until: q.until.map(Millis),
            min_size: q.min_size,
            max_size: q.max_size,
            tag: non_empty(q.tag),
            sort: q.sort.map(Into::into).unwrap_or_default(),
            descending: q.descending,
            limit: q.limit,
            offset: q.offset,
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiHistoryEntry {
    pub task_id: String,
    pub kind: FfiTaskKind,
    pub name: String,
    pub original_url: String,
    pub final_url: Option<String>,
    pub domain: String,
    pub size: Option<u64>,
    pub checksum: Option<String>,
    pub state: FfiTaskState,
    pub destination: String,
    pub category_id: Option<String>,
    pub queue_id: String,
    pub started_at: Option<i64>,
    pub finished_at: i64,
    pub duration_seconds: u64,
    pub average_speed: u64,
    pub peak_speed: u64,
    pub error: Option<String>,
    pub tags: Vec<String>,
    pub origin: String,
}

impl From<&d::history::HistoryEntry> for FfiHistoryEntry {
    fn from(h: &d::history::HistoryEntry) -> Self {
        Self {
            task_id: h.task_id.0.clone(),
            kind: h.kind.into(),
            name: h.name.clone(),
            original_url: h.original_url.clone(),
            final_url: h.final_url.clone(),
            domain: h.domain.clone(),
            size: h.size,
            checksum: checksum_str(&h.checksum),
            state: h.state.into(),
            destination: path_str(&h.destination),
            category_id: h.category_id.as_ref().map(|c| c.0.clone()),
            queue_id: h.queue_id.0.clone(),
            started_at: ms_opt(h.started_at),
            finished_at: ms(h.finished_at),
            duration_seconds: h.duration_seconds,
            average_speed: h.average_speed,
            peak_speed: h.peak_speed,
            error: h.error.clone(),
            tags: h.tags.clone(),
            origin: h.origin.clone(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// stats / dashboard / disk
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiGlobalStats {
    pub download_speed: u64,
    pub upload_speed: u64,
    pub active: u32,
    pub downloading: u32,
    pub seeding: u32,
    pub queued: u32,
    pub scheduled: u32,
    pub paused: u32,
    pub completed_today: u32,
    pub failed_today: u32,
    pub total_tasks: u32,
    pub bytes_today: u64,
    pub free_space: Option<u64>,
    pub network_available: bool,
    pub traffic_mode: FfiTrafficMode,
    pub download_limit: u64,
    pub upload_limit: u64,
    pub at: i64,
}

impl From<&d::GlobalStats> for FfiGlobalStats {
    fn from(g: &d::GlobalStats) -> Self {
        Self {
            download_speed: g.download_speed,
            upload_speed: g.upload_speed,
            active: g.active,
            downloading: g.downloading,
            seeding: g.seeding,
            queued: g.queued,
            scheduled: g.scheduled,
            paused: g.paused,
            completed_today: g.completed_today,
            failed_today: g.failed_today,
            total_tasks: g.total_tasks,
            bytes_today: g.bytes_today,
            free_space: g.free_space,
            network_available: g.network_available,
            traffic_mode: g.traffic_mode.into(),
            download_limit: g.download_limit,
            upload_limit: g.upload_limit,
            at: ms(g.at),
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiSpeedSample {
    pub at: i64,
    pub download: u64,
    pub upload: u64,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiDiskInfo {
    pub path: String,
    pub free: Option<u64>,
    pub total: Option<u64>,
    pub reserved: u64,
    pub required_by_active: u64,
    pub volume_available: bool,
}

impl From<&s::DiskInfo> for FfiDiskInfo {
    fn from(x: &s::DiskInfo) -> Self {
        Self {
            path: path_str(&x.path),
            free: x.free,
            total: x.total,
            reserved: x.reserved,
            required_by_active: x.required_by_active,
            volume_available: x.volume_available,
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiScheduleBoundary {
    pub schedule_id: String,
    pub at: i64,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiDashboard {
    pub stats: FfiGlobalStats,
    pub queues: Vec<FfiQueueSummary>,
    pub recent: Vec<FfiTaskRow>,
    /// Last hour of speed samples.
    pub speed_history: Vec<FfiSpeedSample>,
    pub disks: Vec<FfiDiskInfo>,
    pub scheduled_next: Vec<FfiScheduleBoundary>,
}

/// Initial state for the UI; afterwards apply `FfiEvent`s.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiSnapshot {
    /// Snapshot generation (monotonic per engine). Per-task staleness uses `FfiTaskRow.rev`.
    pub rev: u64,
    pub rows: Vec<FfiTaskRow>,
    pub queues: Vec<FfiQueue>,
    pub queue_summaries: Vec<FfiQueueSummary>,
    pub categories: Vec<FfiCategory>,
    pub stats: FfiGlobalStats,
}

// ---------------------------------------------------------------------------------------------
// credentials / devices
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiCredential {
    pub id: String,
    pub name: String,
    pub username: Option<String>,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiDevice {
    pub id: String,
    pub name: String,
    /// `phone`, `tablet`, `browser`, `computer`, `cli`, `extension`.
    pub kind: String,
    pub scopes: Vec<FfiScope>,
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
    pub last_ip: Option<String>,
    pub expires_at: Option<i64>,
    pub revoked: bool,
}

impl From<&d::device::Device> for FfiDevice {
    fn from(x: &d::device::Device) -> Self {
        Self {
            id: x.id.0.clone(),
            name: x.name.clone(),
            kind: x.kind.clone(),
            scopes: x.scopes.iter().map(|s| (*s).into()).collect(),
            created_at: ms(x.created_at),
            last_seen_at: ms_opt(x.last_seen_at),
            last_ip: x.last_ip.clone(),
            expires_at: ms_opt(x.expires_at),
            revoked: x.revoked,
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiAuditEntry {
    pub at: i64,
    pub device_id: Option<String>,
    pub ip: String,
    pub action: String,
    pub target: Option<String>,
    pub success: bool,
    pub detail: Option<String>,
}

impl From<&d::device::AuditEntry> for FfiAuditEntry {
    fn from(x: &d::device::AuditEntry) -> Self {
        Self {
            at: ms(x.at),
            device_id: x.device_id.as_ref().map(|d| d.0.clone()),
            ip: x.ip.clone(),
            action: x.action.clone(),
            target: x.target.clone(),
            success: x.success,
            detail: x.detail.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiPairingInfo {
    pub code: String,
    pub expires_at: i64,
    /// URL the phone opens (host:port + TLS fingerprint, not the code) — render as a QR code.
    pub url: String,
    pub tls_fingerprint: Option<String>,
}

impl From<&s::PairingInfo> for FfiPairingInfo {
    fn from(p: &s::PairingInfo) -> Self {
        Self {
            code: p.code.clone(),
            expires_at: ms(p.expires_at),
            url: p.url.clone(),
            tls_fingerprint: p.tls_fingerprint.clone(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// grabber / archives / import-export / updates / plugins
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiGrabberOptions {
    pub url: String,
    /// 0 = only the start page.
    #[uniffi(default = 1)]
    pub max_depth: u8,
    /// `same_domain`, `subdomains`, `external`.
    #[uniffi(default = "same_domain")]
    pub scope: String,
    #[uniffi(default = true)]
    pub respect_robots: bool,
    #[uniffi(default = 4)]
    pub concurrency: u8,
    #[uniffi(default = 200)]
    pub max_pages: u32,
    #[uniffi(default = [])]
    pub include_extensions: Vec<String>,
    #[uniffi(default = [])]
    pub exclude_patterns: Vec<String>,
    #[uniffi(default = None)]
    pub include_regex: Option<String>,
    #[uniffi(default = None)]
    pub min_size: Option<u64>,
    #[uniffi(default = None)]
    pub max_size: Option<u64>,
    #[uniffi(default = false)]
    pub probe_files: bool,
    #[uniffi(default = false)]
    pub follow_iframes: bool,
}

impl From<FfiGrabberOptions> for s::GrabberOptions {
    fn from(o: FfiGrabberOptions) -> Self {
        Self {
            url: o.url,
            max_depth: o.max_depth,
            scope: o.scope,
            respect_robots: o.respect_robots,
            concurrency: o.concurrency,
            max_pages: o.max_pages,
            include_extensions: o.include_extensions,
            exclude_patterns: o.exclude_patterns,
            include_regex: non_empty(o.include_regex),
            min_size: o.min_size,
            max_size: o.max_size,
            probe_files: o.probe_files,
            follow_iframes: o.follow_iframes,
        }
    }
}

impl From<&s::GrabberOptions> for FfiGrabberOptions {
    fn from(o: &s::GrabberOptions) -> Self {
        Self {
            url: o.url.clone(),
            max_depth: o.max_depth,
            scope: o.scope.clone(),
            respect_robots: o.respect_robots,
            concurrency: o.concurrency,
            max_pages: o.max_pages,
            include_extensions: o.include_extensions.clone(),
            exclude_patterns: o.exclude_patterns.clone(),
            include_regex: o.include_regex.clone(),
            min_size: o.min_size,
            max_size: o.max_size,
            probe_files: o.probe_files,
            follow_iframes: o.follow_iframes,
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiGrabberFile {
    pub url: String,
    pub name: String,
    pub extension: String,
    pub domain: String,
    pub found_on: String,
    pub size: Option<u64>,
    pub mime: Option<String>,
    /// `document`, `image`, `video`, `audio`, `archive`, `software`, `torrent`, `playlist`, `other`.
    pub kind: String,
    pub depth: u8,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiGrabberSession {
    pub id: String,
    pub options: FfiGrabberOptions,
    pub pages_crawled: u32,
    pub pages_queued: u32,
    pub files: Vec<FfiGrabberFile>,
    pub done: bool,
    pub cancelled: bool,
    pub error: Option<String>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub robots_blocked: u32,
}

impl From<&s::GrabberSession> for FfiGrabberSession {
    fn from(x: &s::GrabberSession) -> Self {
        Self {
            id: x.id.clone(),
            options: (&x.options).into(),
            pages_crawled: x.pages_crawled,
            pages_queued: x.pages_queued,
            files: x
                .files
                .iter()
                .map(|f| FfiGrabberFile {
                    url: f.url.clone(),
                    name: f.name.clone(),
                    extension: f.extension.clone(),
                    domain: f.domain.clone(),
                    found_on: f.found_on.clone(),
                    size: f.size,
                    mime: f.mime.clone(),
                    kind: f.kind.clone(),
                    depth: f.depth,
                })
                .collect(),
            done: x.done,
            cancelled: x.cancelled,
            error: x.error.clone(),
            started_at: ms(x.started_at),
            finished_at: ms_opt(x.finished_at),
            robots_blocked: x.robots_blocked,
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiArchiveEntry {
    pub path: String,
    pub size: u64,
    pub compressed_size: Option<u64>,
    pub is_dir: bool,
    pub modified: Option<i64>,
    pub crc32: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiArchiveListing {
    pub format: String,
    pub entries: Vec<FfiArchiveEntry>,
    pub truncated: bool,
    pub intact: Option<bool>,
    pub supports_selective_extraction: bool,
}

impl From<&s::ArchiveListing> for FfiArchiveListing {
    fn from(a: &s::ArchiveListing) -> Self {
        Self {
            format: a.format.clone(),
            entries: a
                .entries
                .iter()
                .map(|e| FfiArchiveEntry {
                    path: e.path.clone(),
                    size: e.size,
                    compressed_size: e.compressed_size,
                    is_dir: e.is_dir,
                    modified: ms_opt(e.modified),
                    crc32: e.crc32,
                })
                .collect(),
            truncated: a.truncated,
            intact: a.intact,
            supports_selective_extraction: a.supports_selective_extraction,
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiImportOptions {
    #[uniffi(default = true)]
    pub settings: bool,
    #[uniffi(default = true)]
    pub queues: bool,
    #[uniffi(default = true)]
    pub categories: bool,
    #[uniffi(default = true)]
    pub rules: bool,
    #[uniffi(default = true)]
    pub schedules: bool,
    #[uniffi(default = true)]
    pub automations: bool,
    #[uniffi(default = true)]
    pub tasks: bool,
    #[uniffi(default = true)]
    pub history: bool,
    #[uniffi(default = true)]
    pub recipes: bool,
    /// Replace existing entries with the same id.
    #[uniffi(default = false)]
    pub overwrite: bool,
}

impl From<FfiImportOptions> for s::ImportOptions {
    fn from(o: FfiImportOptions) -> Self {
        Self {
            settings: o.settings,
            queues: o.queues,
            categories: o.categories,
            rules: o.rules,
            schedules: o.schedules,
            automations: o.automations,
            tasks: o.tasks,
            history: o.history,
            recipes: o.recipes,
            overwrite: o.overwrite,
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiImportReport {
    pub imported: HashMap<String, u32>,
    pub skipped: HashMap<String, u32>,
    pub errors: Vec<String>,
}

impl From<&s::ImportReport> for FfiImportReport {
    fn from(r: &s::ImportReport) -> Self {
        Self {
            imported: r.imported.clone().into_iter().collect(),
            skipped: r.skipped.clone().into_iter().collect(),
            errors: r.errors.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiUpdateInfo {
    pub current_version: String,
    /// `up_to_date`, `available`, `no_releases`, `offline`, `rate_limited` or `error`.
    pub status: String,
    pub available: bool,
    pub skipped: bool,
    pub latest_version: Option<String>,
    pub name: Option<String>,
    /// Release notes (Markdown).
    pub notes: Option<String>,
    pub notes_url: Option<String>,
    /// RFC 3339.
    pub published_at: Option<String>,
    pub prerelease: bool,
    pub download_url: Option<String>,
    pub size: Option<u64>,
    pub message: Option<String>,
    pub signature_valid: Option<bool>,
    pub checked_at: i64,
}

impl From<&s::UpdateInfo> for FfiUpdateInfo {
    fn from(u: &s::UpdateInfo) -> Self {
        Self {
            current_version: u.current_version.clone(),
            status: u.status.clone(),
            available: u.available,
            skipped: u.skipped,
            latest_version: u.latest_version.clone(),
            name: u.name.clone(),
            notes: u.notes.clone(),
            notes_url: u.notes_url.clone(),
            published_at: u.published_at.clone(),
            prerelease: u.prerelease,
            download_url: u.download_url.clone(),
            size: u.size,
            message: u.message.clone(),
            signature_valid: u.signature_valid,
            checked_at: ms(u.checked_at),
        }
    }
}

/// Download / verification / staging progress of the pending update.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiUpdateProgress {
    /// `idle`, `downloading`, `verifying`, `verified`, `staging`, `staged` or `failed`.
    pub phase: String,
    pub version: Option<String>,
    pub received: u64,
    pub total: Option<u64>,
    pub message: Option<String>,
}

impl From<swoop_services::updates::UpdateProgress> for FfiUpdateProgress {
    fn from(p: swoop_services::updates::UpdateProgress) -> Self {
        Self {
            phase: p.phase,
            version: p.version,
            received: p.received,
            total: p.total,
            message: p.message,
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiPluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub kind: String,
    pub permissions: Vec<String>,
    pub enabled: bool,
    pub path: String,
    pub granted_permissions: Vec<String>,
}

impl From<&s::PluginInfo> for FfiPluginInfo {
    fn from(p: &s::PluginInfo) -> Self {
        Self {
            id: p.id.0.clone(),
            name: p.name.clone(),
            version: p.version.clone(),
            description: p.description.clone(),
            author: p.author.clone(),
            kind: p.kind.clone(),
            permissions: p.permissions.clone(),
            enabled: p.enabled,
            path: path_str(&p.path),
            granted_permissions: p.granted_permissions.clone(),
        }
    }
}
