//! The download task: the central entity of the product.

use crate::health::HealthScore;
use crate::media::MediaInfo;
use crate::torrent::TorrentInfo;
use crate::{
    CategoryId, CredentialId, Millis, ProxyId, QueueId, ScheduleId, TaskError, TaskId, TaskState,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    #[default]
    Http,
    Ftp,
    Torrent,
    Magnet,
    Metalink,
    Hls,
}

impl TaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskKind::Http => "http",
            TaskKind::Ftp => "ftp",
            TaskKind::Torrent => "torrent",
            TaskKind::Magnet => "magnet",
            TaskKind::Metalink => "metalink",
            TaskKind::Hls => "hls",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "http" => Some(TaskKind::Http),
            "ftp" => Some(TaskKind::Ftp),
            "torrent" => Some(TaskKind::Torrent),
            "magnet" => Some(TaskKind::Magnet),
            "metalink" => Some(TaskKind::Metalink),
            "hls" => Some(TaskKind::Hls),
            _ => None,
        }
    }
    pub fn is_torrent(self) -> bool {
        matches!(self, TaskKind::Torrent | TaskKind::Magnet)
    }
}

/// Where the bytes come from. A task may have several mirrors for the same content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Source {
    /// One or more URLs serving identical content (mirrors). The first is the primary.
    Urls {
        urls: Vec<String>,
    },
    /// Raw `.torrent` bytes (stored by the persistence layer, referenced here by hash).
    TorrentFile {
        info_hash: String,
        name: String,
    },
    Magnet {
        uri: String,
    },
    /// A Metalink document (v3 or v4) fetched from `url` or supplied inline.
    Metalink {
        url: Option<String>,
        document: Option<String>,
    },
    /// An HLS master or media playlist.
    Hls {
        playlist_url: String,
        variant: Option<String>,
    },
}

impl Source {
    pub fn primary_url(&self) -> Option<&str> {
        match self {
            Source::Urls { urls } => urls.first().map(String::as_str),
            Source::Magnet { uri } => Some(uri),
            Source::Metalink { url, .. } => url.as_deref(),
            Source::Hls { playlist_url, .. } => Some(playlist_url),
            Source::TorrentFile { .. } => None,
        }
    }
    pub fn urls(&self) -> Vec<String> {
        match self {
            Source::Urls { urls } => urls.clone(),
            _ => self
                .primary_url()
                .map(|u| vec![u.to_owned()])
                .unwrap_or_default(),
        }
    }
    pub fn domain(&self) -> Option<String> {
        self.primary_url()
            .and_then(|u| url::Url::parse(u).ok())
            .and_then(|u| u.host_str().map(|h| h.to_lowercase()))
    }
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
    Urgent,
}

impl Priority {
    pub fn as_i32(self) -> i32 {
        match self {
            Priority::Low => 0,
            Priority::Normal => 1,
            Priority::High => 2,
            Priority::Urgent => 3,
        }
    }
    pub fn from_i32(v: i32) -> Self {
        match v {
            0 => Priority::Low,
            2 => Priority::High,
            3 => Priority::Urgent,
            _ => Priority::Normal,
        }
    }
}

/// What to do when the destination already exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Ask the user (task stays `Pending` with a `DuplicateDetected` event).
    #[default]
    Ask,
    Replace,
    /// Append ` (2)`, ` (3)` … before the extension.
    Rename,
    Skip,
    /// Alias for `Rename`, kept distinct so the UI can show the user's explicit intent.
    KeepBoth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumAlgorithm {
    Md5,
    Sha1,
    Sha256,
    Sha512,
    Blake3,
}

impl ChecksumAlgorithm {
    pub fn as_str(self) -> &'static str {
        match self {
            ChecksumAlgorithm::Md5 => "md5",
            ChecksumAlgorithm::Sha1 => "sha1",
            ChecksumAlgorithm::Sha256 => "sha256",
            ChecksumAlgorithm::Sha512 => "sha512",
            ChecksumAlgorithm::Blake3 => "blake3",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().replace('-', "").as_str() {
            "md5" => Some(Self::Md5),
            "sha1" => Some(Self::Sha1),
            "sha256" => Some(Self::Sha256),
            "sha512" => Some(Self::Sha512),
            "blake3" => Some(Self::Blake3),
            _ => None,
        }
    }
    /// Expected hex digest length.
    pub fn hex_len(self) -> usize {
        match self {
            ChecksumAlgorithm::Md5 => 32,
            ChecksumAlgorithm::Sha1 => 40,
            ChecksumAlgorithm::Sha256 | ChecksumAlgorithm::Blake3 => 64,
            ChecksumAlgorithm::Sha512 => 128,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checksum {
    pub algorithm: ChecksumAlgorithm,
    /// Lower-case hex.
    pub value: String,
}

impl Checksum {
    pub fn new(algorithm: ChecksumAlgorithm, value: impl Into<String>) -> Self {
        Self {
            algorithm,
            value: value.into().to_ascii_lowercase(),
        }
    }
    pub fn is_well_formed(&self) -> bool {
        self.value.len() == self.algorithm.hex_len()
            && self.value.chars().all(|c| c.is_ascii_hexdigit())
    }
    /// Parse `sha256:abcd…` or `sha256=abcd…` or a bare 64-hex string (assumed sha256).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if let Some((algo, val)) = s.split_once([':', '=']) {
            let algorithm = ChecksumAlgorithm::parse(algo.trim())?;
            let c = Checksum::new(algorithm, val.trim());
            return c.is_well_formed().then_some(c);
        }
        let algorithm = match s.len() {
            32 => ChecksumAlgorithm::Md5,
            40 => ChecksumAlgorithm::Sha1,
            64 => ChecksumAlgorithm::Sha256,
            128 => ChecksumAlgorithm::Sha512,
            _ => return None,
        };
        let c = Checksum::new(algorithm, s);
        c.is_well_formed().then_some(c)
    }
}

/// Per-task transfer options. `None` means "inherit from queue/global settings".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TaskOptions {
    pub max_connections: Option<u8>,
    /// Bytes per second; `Some(0)` means unlimited explicitly.
    pub download_limit: Option<u64>,
    pub upload_limit: Option<u64>,
    pub headers: BTreeMap<String, String>,
    pub user_agent: Option<String>,
    pub referer: Option<String>,
    pub cookies: Option<String>,
    pub credential: Option<CredentialId>,
    pub proxy: Option<ProxyId>,
    /// Bypass the global proxy for this task.
    pub direct_connection: bool,
    pub checksum: Option<Checksum>,
    pub conflict_policy: ConflictPolicy,
    pub preallocate: Option<bool>,
    pub sparse: Option<bool>,
    /// Adaptive segmentation on/off (default on).
    pub adaptive: Option<bool>,
    pub max_retries: Option<u32>,
    /// Torrent: sequential piece order.
    pub sequential: Option<bool>,
    /// Torrent: seed ratio limit (e.g. 2.0). `Some(0.0)` disables seeding.
    pub seed_ratio_limit: Option<f32>,
    /// Torrent: seed time limit in minutes.
    pub seed_time_limit_minutes: Option<u32>,
    /// Torrent: peer connection limit.
    pub max_peers: Option<u32>,
    /// HLS/media: chosen variant / quality label.
    pub media_variant: Option<String>,
    /// Allow HTTP/2 multiplexing (default: off for segmented downloads so each segment gets its own TCP connection).
    pub allow_http2: Option<bool>,
    /// Post-download: open file automatically.
    pub open_when_done: bool,
}

/// Live counters. Updated by the engine at high frequency; only snapshots reach the UI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Progress {
    pub downloaded: u64,
    pub uploaded: u64,
    /// `None` until the size is known (chunked responses, magnets before metadata).
    pub total: Option<u64>,
    /// Bytes per second, exponentially smoothed over ~2 s.
    pub speed: u64,
    /// Bytes per second over the last sample window.
    pub instant_speed: u64,
    pub upload_speed: u64,
    /// Seconds; `None` if unknown.
    pub eta_seconds: Option<u64>,
    pub active_connections: u32,
    /// Torrent: connected peers / seeds.
    pub peers: u32,
    pub seeds: u32,
    /// Torrent: share ratio.
    pub ratio: f32,
    /// Fraction of segments/pieces complete for the progress bar (0..=1), derived when total is unknown.
    pub fraction: f32,
}

impl Progress {
    pub fn percent(&self) -> Option<f32> {
        match self.total {
            Some(t) if t > 0 => Some((self.downloaded as f64 / t as f64 * 100.0) as f32),
            _ => None,
        }
    }
}

/// Aggregated statistics kept for history and the health score.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TaskStats {
    pub average_speed: u64,
    pub peak_speed: u64,
    pub retries: u32,
    pub failed_connections: u32,
    pub segments_reassigned: u32,
    pub mirrors_switched: u32,
    /// Wall-clock seconds spent in transferring states.
    pub active_seconds: u64,
    pub bytes_discarded: u64,
    /// Number of times the throughput dropped by >50% within a window (instability signal).
    pub throughput_drops: u32,
    /// Server told us it supports byte ranges.
    pub range_supported: Option<bool>,
    pub http_version: Option<String>,
    pub final_url: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub server: Option<String>,
    pub content_type: Option<String>,
    pub content_disposition: Option<String>,
    /// Resolved IP that served the content (for diagnostics).
    pub remote_addr: Option<String>,
}

/// A byte-range segment of an HTTP/FTP download. `committed` is the first byte *not yet* flushed
/// to disk; on restart everything from `committed` is fetched again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub index: u32,
    pub start: u64,
    /// Exclusive.
    pub end: u64,
    pub committed: u64,
    /// Which mirror index served this segment (for diagnostics).
    pub source_index: u32,
}

impl Segment {
    pub fn new(index: u32, start: u64, end: u64) -> Self {
        Self {
            index,
            start,
            end,
            committed: start,
            source_index: 0,
        }
    }
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn remaining(&self) -> u64 {
        self.end.saturating_sub(self.committed)
    }
    pub fn is_done(&self) -> bool {
        self.committed >= self.end
    }
}

/// Durable resume information for a task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SegmentMap {
    pub segments: Vec<Segment>,
    /// Validators captured on first connect; a mismatch on resume means `SourceChanged`.
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub total: Option<u64>,
    /// The partial file we are writing into.
    pub part_path: Option<PathBuf>,
}

impl SegmentMap {
    pub fn committed_bytes(&self) -> u64 {
        self.segments
            .iter()
            .map(|s| s.committed.saturating_sub(s.start))
            .sum()
    }
    pub fn remaining_bytes(&self) -> u64 {
        self.segments.iter().map(Segment::remaining).sum()
    }
    pub fn is_complete(&self) -> bool {
        !self.segments.is_empty() && self.segments.iter().all(Segment::is_done)
    }
}

/// The task record. This is what persistence stores and what the API returns.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub kind: TaskKind,
    pub source: Source,
    /// Display / target file name (sanitised).
    pub name: String,
    /// Destination directory.
    pub directory: PathBuf,
    /// Final path once known (directory/name, or the torrent root).
    #[serde(default)]
    pub file_path: Option<PathBuf>,
    pub state: TaskState,
    /// Everything currently preventing the task from running (see [`crate::state::PauseReason`]).
    #[serde(default)]
    pub blocked_by: Vec<crate::state::PauseReason>,
    /// Monotonic revision, bumped on every persisted change; consumers drop stale updates.
    #[serde(default)]
    pub rev: u64,
    /// The user edited the name / directory; engine-resolved metadata must not overwrite them.
    #[serde(default)]
    pub name_locked: bool,
    #[serde(default)]
    pub directory_locked: bool,
    #[serde(default)]
    pub status_detail: Option<String>,
    #[serde(default)]
    pub error: Option<TaskError>,
    #[serde(default)]
    pub progress: Progress,
    #[serde(default)]
    pub stats: TaskStats,
    pub queue_id: QueueId,
    #[serde(default)]
    pub category_id: Option<CategoryId>,
    #[serde(default)]
    pub schedule_id: Option<ScheduleId>,
    #[serde(default)]
    pub priority: Priority,
    /// Manual order within the queue (lower first).
    #[serde(default)]
    pub position: i64,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub options: TaskOptions,
    #[serde(default)]
    pub segment_map: Option<SegmentMap>,
    #[serde(default)]
    pub torrent: Option<TorrentInfo>,
    #[serde(default)]
    pub media: Option<MediaInfo>,
    #[serde(default)]
    pub health: HealthScore,
    /// Where the task came from (`browser`, `cli`, `remote:<device>`, `grabber`, `recipe:<id>`).
    #[serde(default)]
    pub origin: String,
    #[serde(default)]
    pub mime: Option<String>,
    pub created_at: Millis,
    pub updated_at: Millis,
    #[serde(default)]
    pub started_at: Option<Millis>,
    #[serde(default)]
    pub completed_at: Option<Millis>,
    /// Verified checksum of the completed file (algorithm chosen by settings; sha256 by default).
    #[serde(default)]
    pub verified_checksum: Option<Checksum>,
    /// Retry attempt counter for the current run.
    #[serde(default)]
    pub attempt: u32,
    /// Next automatic retry time when `state == Retrying`.
    #[serde(default)]
    pub next_retry_at: Option<Millis>,
}

impl Task {
    /// Build a new task in `Pending` state with sensible defaults.
    pub fn new(
        kind: TaskKind,
        source: Source,
        name: impl Into<String>,
        directory: PathBuf,
        queue_id: QueueId,
    ) -> Self {
        let now = Millis::now();
        Self {
            id: TaskId::new(),
            kind,
            source,
            name: name.into(),
            directory,
            file_path: None,
            state: TaskState::Pending,
            blocked_by: Vec::new(),
            rev: 0,
            name_locked: false,
            directory_locked: false,
            status_detail: None,
            error: None,
            progress: Progress::default(),
            stats: TaskStats::default(),
            queue_id,
            category_id: None,
            schedule_id: None,
            priority: Priority::Normal,
            position: now.0,
            tags: Vec::new(),
            options: TaskOptions::default(),
            segment_map: None,
            torrent: None,
            media: None,
            health: HealthScore::default(),
            origin: String::new(),
            mime: None,
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
            verified_checksum: None,
            attempt: 0,
            next_retry_at: None,
        }
    }

    pub fn domain(&self) -> Option<String> {
        self.source.domain()
    }

    pub fn target_path(&self) -> PathBuf {
        self.file_path
            .clone()
            .unwrap_or_else(|| self.directory.join(&self.name))
    }

    pub fn extension(&self) -> Option<String> {
        std::path::Path::new(&self.name)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
    }

    /// Apply a state transition, validating it. Returns the previous state.
    pub fn transition(&mut self, to: TaskState) -> Result<TaskState, crate::DomainError> {
        if !self.state.can_transition_to(to) {
            return Err(crate::DomainError::InvalidTransition(format!(
                "task {} cannot go from {} to {}",
                self.id, self.state, to
            )));
        }
        let from = self.state;
        self.state = to;
        self.touch();
        match to {
            TaskState::Downloading if self.started_at.is_none() => {
                self.started_at = Some(self.updated_at)
            }
            TaskState::Completed => self.completed_at = Some(self.updated_at),
            _ => {}
        }
        if !matches!(to, TaskState::Paused | TaskState::Scheduled) {
            self.blocked_by.clear();
        }
        // keep the last error visible on Failed only; clear it when we move on
        if matches!(
            to,
            TaskState::Queued | TaskState::Resolving | TaskState::Connecting
        ) {
            self.error = None;
        }
        if to != TaskState::Retrying {
            self.next_retry_at = None;
        }
        Ok(from)
    }

    /// Bump the revision and `updated_at`. Every persisted mutation goes through here.
    pub fn touch(&mut self) {
        self.rev = self.rev.wrapping_add(1);
        self.updated_at = Millis::now();
    }

    /// Add a block reason (idempotent). Returns true if it was newly added.
    pub fn block(&mut self, reason: crate::state::PauseReason) -> bool {
        if self.blocked_by.contains(&reason) {
            return false;
        }
        self.blocked_by.push(reason);
        self.touch();
        true
    }

    /// Remove a block reason. Returns true if it was present.
    pub fn unblock(&mut self, reason: &crate::state::PauseReason) -> bool {
        let before = self.blocked_by.len();
        self.blocked_by.retain(|r| r != reason);
        let changed = self.blocked_by.len() != before;
        if changed {
            self.touch();
        }
        changed
    }

    /// Remove every automatic block reason (schedule, condition, disk…), leaving a user pause.
    pub fn clear_automatic_blocks(&mut self) -> bool {
        let before = self.blocked_by.len();
        self.blocked_by.retain(|r| !r.is_automatic());
        let changed = self.blocked_by.len() != before;
        if changed {
            self.touch();
        }
        changed
    }

    pub fn is_user_paused(&self) -> bool {
        self.blocked_by.contains(&crate::state::PauseReason::User)
    }

    /// Prepare a Completed/Failed/Cancelled task to run again. `keep_partial = false` discards
    /// resume data (the services layer deletes the part file).
    pub fn reset_for_rerun(&mut self, keep_partial: bool) {
        self.progress = Progress::default();
        self.error = None;
        self.completed_at = None;
        self.started_at = None;
        self.verified_checksum = None;
        self.attempt = 0;
        self.next_retry_at = None;
        self.blocked_by.clear();
        self.stats = TaskStats {
            range_supported: self.stats.range_supported,
            http_version: None,
            ..TaskStats::default()
        };
        self.health = HealthScore::default();
        if !keep_partial {
            self.segment_map = None;
            if let Some(m) = &mut self.media {
                m.segments_done = 0;
            }
        }
        self.touch();
    }
}

/// A compact per-task progress update, batched by the event coalescer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProgressUpdate {
    pub task_id: TaskId,
    pub progress: Progress,
    /// Task revision the update belongs to; consumers ignore updates older than what they hold.
    #[serde(default)]
    pub rev: u64,
}

/// Input for creating a task through any front door (UI, API, CLI, extension, grabber).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct NewTaskRequest {
    pub url: Option<String>,
    pub mirrors: Vec<String>,
    pub magnet: Option<String>,
    /// Base64 `.torrent` bytes.
    pub torrent_base64: Option<String>,
    pub metalink_url: Option<String>,
    pub hls_playlist_url: Option<String>,
    pub name: Option<String>,
    pub directory: Option<PathBuf>,
    pub queue_id: Option<QueueId>,
    pub category_id: Option<CategoryId>,
    pub schedule_id: Option<ScheduleId>,
    pub priority: Option<Priority>,
    pub tags: Vec<String>,
    pub options: TaskOptions,
    /// Start immediately (`true`) or leave `Pending` for the user to confirm.
    pub start: bool,
    pub origin: String,
    /// Torrent: selected file indices (None = all).
    pub selected_files: Option<Vec<u32>>,
    /// Page the link was found on (extension / grabber), used by rules and history.
    pub referer_page: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_parsing() {
        let c = Checksum::parse("sha256:AbCd".repeat(16).as_str());
        assert!(c.is_none());
        let hex64 = "a".repeat(64);
        let c = Checksum::parse(&format!("SHA-256={hex64}")).unwrap();
        assert_eq!(c.algorithm, ChecksumAlgorithm::Sha256);
        let bare = Checksum::parse(&"b".repeat(40)).unwrap();
        assert_eq!(bare.algorithm, ChecksumAlgorithm::Sha1);
        assert!(Checksum::parse("zz").is_none());
    }

    #[test]
    fn segment_accounting() {
        let mut m = SegmentMap::default();
        m.segments.push(Segment::new(0, 0, 100));
        m.segments.push(Segment::new(1, 100, 250));
        m.segments[0].committed = 100;
        m.segments[1].committed = 120;
        assert_eq!(m.committed_bytes(), 120);
        assert_eq!(m.remaining_bytes(), 130);
        assert!(!m.is_complete());
        m.segments[1].committed = 250;
        assert!(m.is_complete());
    }

    #[test]
    fn transition_updates_timestamps() {
        let mut t = Task::new(
            TaskKind::Http,
            Source::Urls {
                urls: vec!["https://example.com/a.zip".into()],
            },
            "a.zip",
            PathBuf::from("/tmp"),
            QueueId::default_queue(),
        );
        t.transition(TaskState::Queued).unwrap();
        t.transition(TaskState::Connecting).unwrap();
        t.transition(TaskState::Downloading).unwrap();
        assert!(t.started_at.is_some());
        assert!(t.transition(TaskState::Pending).is_err());
        t.transition(TaskState::Verifying).unwrap();
        t.transition(TaskState::Completed).unwrap();
        assert!(t.completed_at.is_some());
        let rev = t.rev;
        t.reset_for_rerun(false);
        assert!(t.completed_at.is_none() && t.segment_map.is_none() && t.rev > rev);
        assert!(t.block(crate::state::PauseReason::User));
        assert!(!t.block(crate::state::PauseReason::User));
        t.block(crate::state::PauseReason::DiskSpace);
        assert!(t.clear_automatic_blocks());
        assert!(t.is_user_paused());
        assert_eq!(t.domain().as_deref(), Some("example.com"));
        assert_eq!(t.extension().as_deref(), Some("zip"));
    }
}
