//! The boundary between the services layer and the transfer engines.
//!
//! An engine implements [`Transfer`]; the task manager gives it a [`TransferContext`] with a
//! progress sink, a control handle (pause/cancel/limits) and a checkpoint sink for durable resume
//! data. Engines never touch the database or the UI.

use crate::{Millis, Progress, SegmentMap, Task, TaskError, TaskKind, TaskState};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use tokio::sync::watch;
use tokio_util_lite::CancellationToken;

/// Minimal cancellation token so this crate does not depend on tokio-util.
pub mod tokio_util_lite {
    use std::sync::Arc;
    use tokio::sync::Notify;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Clone, Debug, Default)]
    pub struct CancellationToken {
        inner: Arc<Inner>,
    }

    #[derive(Debug, Default)]
    struct Inner {
        cancelled: AtomicBool,
        notify: Notify,
    }

    impl CancellationToken {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn cancel(&self) {
            self.inner.cancelled.store(true, Ordering::SeqCst);
            self.inner.notify.notify_waiters();
        }
        pub fn is_cancelled(&self) -> bool {
            self.inner.cancelled.load(Ordering::SeqCst)
        }
        pub async fn cancelled(&self) {
            loop {
                if self.is_cancelled() {
                    return;
                }
                let notified = self.inner.notify.notified();
                if self.is_cancelled() {
                    return;
                }
                notified.await;
            }
        }
    }
}

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
    pub torrent: Option<crate::torrent::TorrentInfo>,
    pub media: Option<crate::media::MediaInfo>,
    /// Final path if the engine decided it (torrents: the root folder).
    pub file_path: Option<PathBuf>,
}

/// What an engine reports back while running.
pub trait ProgressSink: Send + Sync {
    fn progress(&self, progress: Progress);
    fn state(&self, state: TaskState, detail: Option<String>);
    fn metadata(&self, metadata: ResolvedMetadata);
    /// Durable resume data; the services layer persists it (debounced).
    fn checkpoint(&self, map: SegmentMap);
    fn log(&self, level: crate::events::LogLevel, code: &str, message: String);
    fn stat(&self, stat: EngineStat);
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

/// Live control knobs the services layer can turn while a transfer runs.
#[derive(Debug)]
pub struct TransferControl {
    pub cancel: CancellationToken,
    /// `true` while the user has asked to pause; engines must stop transferring and return
    /// [`TransferOutcome::Paused`] promptly (< 1 s), persisting a checkpoint first.
    pub pause: CancellationToken,
    /// Bytes per second; 0 = unlimited. Changed live from the UI.
    pub download_limit: AtomicU64,
    pub upload_limit: AtomicU64,
    /// Target connection count; 0 = engine decides (adaptive).
    pub max_connections: AtomicU8,
    /// Set when the destination volume disappeared; engines should pause.
    pub volume_lost: AtomicBool,
    /// Selected torrent files / priorities may change live.
    pub file_selection: watch::Sender<Option<Vec<(u32, bool, u8)>>>,
}

impl TransferControl {
    pub fn new() -> Arc<Self> {
        let (tx, _rx) = watch::channel(None);
        Arc::new(Self {
            cancel: CancellationToken::new(),
            pause: CancellationToken::new(),
            download_limit: AtomicU64::new(0),
            upload_limit: AtomicU64::new(0),
            max_connections: AtomicU8::new(0),
            volume_lost: AtomicBool::new(false),
            file_selection: tx,
        })
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
    /// Resolves when either pause or cancel is requested.
    pub async fn stopped(&self) {
        tokio::select! {
            _ = self.cancel.cancelled() => {},
            _ = self.pause.cancelled() => {},
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
        }
    }
}

/// Everything an engine needs to run one task.
pub struct TransferContext {
    /// Snapshot of the task at start (engines read options/segment map from here).
    pub task: Task,
    pub control: Arc<TransferControl>,
    pub sink: Arc<dyn ProgressSink>,
    /// Effective settings snapshot.
    pub settings: Arc<crate::settings::Settings>,
    /// Resolved secrets (never persisted here): header/basic-auth username+password, proxy URL.
    pub secrets: TransferSecrets,
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
    Completed { file_path: PathBuf, bytes: u64 },
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
    async fn probe(&self, task: &Task, settings: Arc<crate::settings::Settings>, secrets: TransferSecrets) -> Result<ResolvedMetadata, TaskError>;
    /// Called when a task is removed so engines can drop per-task state (torrent session entry).
    async fn forget(&self, task: &Task) {
        let _ = task;
    }
}

/// A no-op sink for tests and probes.
pub struct NullSink;
impl ProgressSink for NullSink {
    fn progress(&self, _: Progress) {}
    fn state(&self, _: TaskState, _: Option<String>) {}
    fn metadata(&self, _: ResolvedMetadata) {}
    fn checkpoint(&self, _: SegmentMap) {}
    fn log(&self, _: crate::events::LogLevel, _: &str, _: String) {}
    fn stat(&self, _: EngineStat) {}
}
