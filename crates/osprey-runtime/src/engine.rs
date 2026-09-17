//! The boundary between the services layer and the transfer engines.
//!
//! An engine implements [`Transfer`]; the task manager gives it a [`TransferContext`] with a
//! progress sink, a control handle (pause/cancel/limits) and a checkpoint sink for durable resume
//! data. Engines never touch the database or the UI.
//!
//! # Pause protocol
//! `TransferControl::pause` is a one-shot token. When it fires, an engine must:
//! 1. stop issuing network reads (select every await against [`TransferControl::stopped`]),
//! 2. flush its writer(s) with a bounded timeout (~5 s),
//! 3. emit a checkpoint **only if** the flush succeeded (otherwise the previously persisted,
//!    conservative checkpoint stands),
//! 4. return [`TransferOutcome::Paused`] — within about one second of the request.
//!
//! The services layer creates a fresh [`TransferControl`] for every run; handles held by the UI
//! are re-pointed on resume.

use osprey_domain::checkpoint::Checkpoint;
use osprey_domain::events::LogLevel;
use osprey_domain::{Millis, Progress, Task, TaskError, TaskKind, TaskState};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use tokio::sync::watch;
pub use tokio_util::sync::CancellationToken;

/// Metadata an engine learns while resolving the source.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ResolvedMetadata {
    pub name: Option<String>,
    pub total: Option<u64>,
    pub mime: Option<String>,
    pub resumable: Option<bool>,
    pub final_url: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub server: Option<String>,
    pub content_disposition: Option<String>,
    pub http_version: Option<String>,
    pub remote_addr: Option<String>,
    pub torrent: Option<osprey_domain::torrent::TorrentInfo>,
    pub media: Option<osprey_domain::media::MediaInfo>,
    /// Final path if the engine decided it (torrents: the root folder).
    pub file_path: Option<PathBuf>,
}

/// Incremental statistics events for the health score and diagnostics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum EngineStat {
    ConnectionOpened,
    ConnectionFailed,
    Retry,
    SegmentReassigned,
    MirrorSwitched { index: u32 },
    Throttled,
    ThroughputDrop,
    RangeSupport { supported: bool },
    BytesDiscarded { bytes: u64 },
    Peak { speed: u64 },
}

/// What an engine reports back while running. Implementations must be cheap and non-blocking:
/// they are called from hot paths.
pub trait ProgressSink: Send + Sync {
    /// Latest counters. Engines call this at most a few times per second (the services layer
    /// also samples [`TransferCounters`] directly, so per-chunk calls are unnecessary).
    fn progress(&self, progress: Progress);
    fn state(&self, state: TaskState, detail: Option<String>);
    fn metadata(&self, metadata: ResolvedMetadata);
    /// Durable resume data — emit only after the data it covers has been flushed.
    fn checkpoint(&self, checkpoint: Checkpoint);
    fn log(&self, level: LogLevel, code: &str, message: String);
    fn stat(&self, stat: EngineStat);
}

/// Lock-free counters bumped per chunk; sampled by the services ticker to derive speed/ETA.
#[derive(Debug, Default)]
pub struct TransferCounters {
    pub downloaded: AtomicU64,
    pub uploaded: AtomicU64,
    pub active_connections: AtomicU64,
}

impl TransferCounters {
    pub fn add_downloaded(&self, n: u64) {
        self.downloaded.fetch_add(n, Ordering::Relaxed);
    }
    pub fn add_uploaded(&self, n: u64) {
        self.uploaded.fetch_add(n, Ordering::Relaxed);
    }
    pub fn set_downloaded(&self, n: u64) {
        self.downloaded.store(n, Ordering::Relaxed);
    }
    pub fn downloaded(&self) -> u64 {
        self.downloaded.load(Ordering::Relaxed)
    }
    pub fn uploaded(&self) -> u64 {
        self.uploaded.load(Ordering::Relaxed)
    }
}

/// Torrent file selection change pushed live into a running engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSelectionUpdate {
    pub index: u32,
    pub selected: bool,
    /// 0 low, 1 normal, 2 high.
    pub priority: u8,
}

/// Live control knobs the services layer can turn while a transfer runs.
#[derive(Debug)]
pub struct TransferControl {
    pub cancel: CancellationToken,
    /// Fires when the user (or a queue/schedule/condition) asks to pause — see the module docs.
    pub pause: CancellationToken,
    /// Bytes per second; 0 = unlimited. Changed live from the UI.
    pub download_limit: AtomicU64,
    pub upload_limit: AtomicU64,
    /// Target connection count; 0 = engine decides (adaptive).
    pub max_connections: AtomicU8,
    /// Set when the destination volume disappeared; engines should pause.
    pub volume_lost: AtomicBool,
    /// Torrent: selected files / priorities may change live. `send_replace` keeps the latest
    /// value even when no receiver is attached yet.
    file_selection: watch::Sender<Option<Vec<FileSelectionUpdate>>>,
    /// Sequential mode toggle for torrents.
    pub sequential: AtomicBool,
    pub counters: TransferCounters,
}

impl TransferControl {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    pub fn should_stop(&self) -> bool {
        self.cancel.is_cancelled() || self.pause.is_cancelled()
    }
    pub fn download_limit(&self) -> u64 {
        self.download_limit.load(Ordering::Relaxed)
    }
    pub fn upload_limit(&self) -> u64 {
        self.upload_limit.load(Ordering::Relaxed)
    }
    pub fn max_connections(&self) -> u8 {
        self.max_connections.load(Ordering::Relaxed)
    }
    pub fn set_file_selection(&self, sel: Vec<FileSelectionUpdate>) {
        self.file_selection.send_replace(Some(sel));
    }
    pub fn file_selection(&self) -> watch::Receiver<Option<Vec<FileSelectionUpdate>>> {
        self.file_selection.subscribe()
    }
    /// Resolves when either pause or cancel is requested.
    pub async fn stopped(&self) {
        tokio::select! {
            _ = self.cancel.cancelled() => {},
            _ = self.pause.cancelled() => {},
        }
    }
    /// Outcome to return when [`Self::stopped`] fired.
    pub fn stop_outcome(&self) -> TransferOutcome {
        if self.cancel.is_cancelled() {
            TransferOutcome::Cancelled
        } else {
            TransferOutcome::Paused
        }
    }
}

impl Default for TransferControl {
    fn default() -> Self {
        let (tx, _rx) = watch::channel(None);
        Self {
            cancel: CancellationToken::new(),
            pause: CancellationToken::new(),
            download_limit: AtomicU64::new(0),
            upload_limit: AtomicU64::new(0),
            max_connections: AtomicU8::new(0),
            volume_lost: AtomicBool::new(false),
            file_selection: tx,
            sequential: AtomicBool::new(false),
            counters: TransferCounters::default(),
        }
    }
}

/// Everything an engine needs to run one task.
pub struct TransferContext {
    /// Snapshot of the task at start (engines read options/checkpoint from here).
    pub task: Task,
    /// Durable resume data from the previous run, if any.
    pub checkpoint: Option<Checkpoint>,
    pub control: Arc<TransferControl>,
    pub sink: Arc<dyn ProgressSink>,
    /// Effective settings snapshot.
    pub settings: Arc<osprey_domain::settings::Settings>,
    /// Resolved secrets (never persisted here).
    pub secrets: TransferSecrets,
    /// Per-task limiter, already parented to queue and global limiters.
    pub limiter: Arc<crate::RateLimiter>,
    pub upload_limiter: Arc<crate::RateLimiter>,
    /// Shared HTTP client factory (proxy/TLS/UA policy applied).
    pub clients: Arc<crate::net::ClientFactory>,
    /// Wall-clock the run started.
    pub started_at: Millis,
}

#[derive(Clone, Debug, Default)]
pub struct TransferSecrets {
    pub username: Option<String>,
    pub password: Option<String>,
    /// Full proxy URL including credentials, if any.
    pub proxy_url: Option<String>,
    /// Extra headers resolved from a credential (e.g. `Authorization: Bearer …`).
    pub headers: Vec<(String, String)>,
}

/// How a run ended.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum TransferOutcome {
    /// All bytes are on disk and verified by the engine where it can (torrent pieces, HLS
    /// segment counts). The services layer may still run a checksum.
    Completed {
        file_path: PathBuf,
        bytes: u64,
    },
    /// Paused at the user's request with a checkpoint written.
    Paused,
    Cancelled,
    /// Torrent completed and is now seeding; the engine keeps running until told to stop.
    Seeding,
    Failed(TaskError),
}

#[async_trait::async_trait]
pub trait Transfer: Send + Sync {
    /// Which kinds this engine handles.
    fn kinds(&self) -> &'static [TaskKind];
    /// Run the transfer to an outcome. Must honour `ctx.control` and never panic.
    async fn run(&self, ctx: TransferContext) -> TransferOutcome;
    /// Probe a source without downloading (HEAD / metadata fetch). Used by the add dialog.
    async fn probe(
        &self,
        task: &Task,
        settings: Arc<osprey_domain::settings::Settings>,
        secrets: TransferSecrets,
        clients: Arc<crate::net::ClientFactory>,
    ) -> Result<ResolvedMetadata, TaskError>;
    /// Called when a task is removed so engines can drop per-task state (torrent session entry).
    async fn forget(&self, task: &Task, delete_files: bool) {
        let _ = (task, delete_files);
    }
}

/// A no-op sink for tests and probes.
pub struct NullSink;
impl ProgressSink for NullSink {
    fn progress(&self, _: Progress) {}
    fn state(&self, _: TaskState, _: Option<String>) {}
    fn metadata(&self, _: ResolvedMetadata) {}
    fn checkpoint(&self, _: Checkpoint) {}
    fn log(&self, _: LogLevel, _: &str, _: String) {}
    fn stat(&self, _: EngineStat) {}
}
