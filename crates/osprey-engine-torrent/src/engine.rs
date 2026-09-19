//! [`TorrentEngine`]: the `Transfer` implementation for `.torrent` and magnet tasks plus the
//! control surface the services layer calls (trackers, peers, seeding limits, live info).

use crate::metainfo::{self, ParsedTorrent};
use crate::peers::PeerSampler;
use crate::run::Runner;
use crate::session::ManagedTorrentHandle;
use crate::session::{SessionTuning, TorrentSession};
use crate::status;
use crate::trackers::{self, TrackerRegistry, TrackerSource};
use librqbit::dht::Id20;
use osprey_domain::settings::Settings;
use osprey_domain::torrent::{PeerInfo, SeedingLimits, TorrentInfo, TrackerStatus};
use osprey_domain::{ErrorKind, Millis, Source, Task, TaskError, TaskId, TaskKind};
use osprey_runtime::engine::{
    ResolvedMetadata, Transfer, TransferContext, TransferOutcome, TransferSecrets,
};
use osprey_runtime::redact::redact;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// Where `.torrent` bytes come from: the services layer implements this over the store's
/// `torrent_blobs` table.
#[async_trait::async_trait]
pub trait TorrentBlobProvider: Send + Sync {
    async fn torrent_bytes(&self, info_hash: &str) -> Option<Vec<u8>>;
}

pub struct TorrentEngineConfig {
    /// `AppPaths::torrent_session_dir()`: librqbit session JSON, `.torrent` copies, piece
    /// bitfields and the DHT routing table live here.
    pub session_dir: PathBuf,
    /// Fallback output folder for librqbit; every task passes its own directory explicitly.
    pub default_output: PathBuf,
    pub settings: Arc<Settings>,
    pub blobs: Arc<dyn TorrentBlobProvider>,
    /// Client used for tracker probes and curated tracker-list downloads (build it through
    /// `ClientFactory::default_client()` so proxy/TLS policy applies).
    pub http: reqwest::Client,
    pub tuning: SessionTuning,
}

/// Settings that could not be applied live; they take effect after `shutdown()` + restart.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeferredSettings {
    pub keys: Vec<&'static str>,
}

/// Effective file selection of one torrent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub selected: Vec<bool>,
    pub priority: Vec<u8>,
}

impl Selection {
    pub fn all(n: usize, padding: &[bool]) -> Self {
        Self {
            selected: (0..n)
                .map(|i| !padding.get(i).copied().unwrap_or(false))
                .collect(),
            priority: vec![1; n],
        }
    }
    pub fn set(&mut self, index: usize, selected: bool, priority: u8) {
        if let Some(s) = self.selected.get_mut(index) {
            *s = selected;
        }
        if let Some(p) = self.priority.get_mut(index) {
            *p = priority.min(2);
        }
    }
    /// librqbit `only_files`: `None` when everything is selected.
    pub fn only_files(&self) -> Option<Vec<usize>> {
        if self.selected.iter().all(|s| *s) {
            None
        } else {
            Some(
                self.selected
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| **s)
                    .map(|(i, _)| i)
                    .collect(),
            )
        }
    }
    pub fn selected_indices(&self) -> Vec<u32> {
        self.selected
            .iter()
            .enumerate()
            .filter(|(_, s)| **s)
            .filter_map(|(i, _)| u32::try_from(i).ok())
            .collect()
    }
}

/// Per-task state that outlives a single `run()` so paused torrents still answer queries.
pub(crate) struct TaskEntry {
    pub task_id: TaskId,
    pub info_hash: Mutex<Option<Id20>>,
    pub parsed: Mutex<Option<Arc<ParsedTorrent>>>,
    pub handle: Mutex<Option<ManagedTorrentHandle>>,
    pub trackers: Arc<TrackerRegistry>,
    pub seeding_override: Mutex<Option<SeedingLimits>>,
    pub peers: Mutex<PeerSampler>,
    /// Uploaded bytes before the current librqbit live state (previous runs / re-adds).
    pub uploaded_base: AtomicU64,
    pub seeding_since: Mutex<Option<Millis>>,
    pub selection: Mutex<Selection>,
    pub running: AtomicBool,
}

impl TaskEntry {
    fn new(task_id: TaskId) -> Arc<Self> {
        Arc::new(Self {
            task_id,
            info_hash: Mutex::new(None),
            parsed: Mutex::new(None),
            handle: Mutex::new(None),
            trackers: Arc::new(TrackerRegistry::new()),
            seeding_override: Mutex::new(None),
            peers: Mutex::new(PeerSampler::default()),
            uploaded_base: AtomicU64::new(0),
            seeding_since: Mutex::new(None),
            selection: Mutex::new(Selection::default()),
            running: AtomicBool::new(false),
        })
    }

    pub fn uploaded_total(&self, snap: &status::Snapshot) -> u64 {
        self.uploaded_base.load(Ordering::Relaxed) + snap.uploaded_live
    }
}

pub struct TorrentEngine {
    pub(crate) session_dir: PathBuf,
    pub(crate) default_output: PathBuf,
    pub(crate) blobs: Arc<dyn TorrentBlobProvider>,
    pub(crate) http: reqwest::Client,
    pub(crate) tuning: SessionTuning,
    pub(crate) settings: Mutex<Arc<Settings>>,
    session: tokio::sync::OnceCell<Arc<TorrentSession>>,
    pub(crate) entries: Mutex<HashMap<TaskId, Arc<TaskEntry>>>,
    /// Trackers from the last `refresh_tracker_list`, added to public torrents on start.
    pub(crate) curated_trackers: Mutex<Vec<String>>,
}

impl TorrentEngine {
    /// Cheap: the librqbit session (listener, DHT, persistence) starts on first use.
    pub fn new(config: TorrentEngineConfig) -> Result<Arc<Self>, TaskError> {
        if !config.session_dir.is_absolute() {
            return Err(TaskError::new(
                ErrorKind::InvalidFilename,
                "torrent session directory must be absolute",
            ));
        }
        Ok(Arc::new(Self {
            session_dir: config.session_dir,
            default_output: config.default_output,
            blobs: config.blobs,
            http: config.http,
            tuning: config.tuning,
            settings: Mutex::new(config.settings),
            session: tokio::sync::OnceCell::new(),
            entries: Mutex::new(HashMap::new()),
            curated_trackers: Mutex::new(Vec::new()),
        }))
    }

    pub fn settings(&self) -> Arc<Settings> {
        self.settings.lock().clone()
    }

    /// The shared librqbit session, started on first call.
    pub(crate) async fn session(&self) -> Result<Arc<TorrentSession>, TaskError> {
        self.session
            .get_or_try_init(|| async {
                let settings = self.settings();
                TorrentSession::start(
                    self.session_dir.clone(),
                    self.default_output.clone(),
                    &settings,
                    self.tuning.clone(),
                )
                .await
            })
            .await
            .cloned()
    }

    pub(crate) fn session_if_started(&self) -> Option<Arc<TorrentSession>> {
        self.session.get().cloned()
    }

    /// Actual peer listener port once the session runs (`0` in settings picks a random one).
    pub fn listen_port(&self) -> Option<u16> {
        self.session_if_started().and_then(|s| s.listen_port())
    }

    /// Apply changed settings. Global rate limits and tracker additions apply live; DHT,
    /// listen port, IPv4-only and the leech-only switch need a restart and are reported.
    pub fn apply_settings(&self, settings: &Settings) -> DeferredSettings {
        let previous = std::mem::replace(&mut *self.settings.lock(), Arc::new(settings.clone()));
        let mut deferred = DeferredSettings::default();
        if let Some(session) = self.session_if_started() {
            session.apply_global_limits(
                settings.torrent.download_limit,
                settings.torrent.upload_limit,
            );
            let p = &previous.torrent;
            let n = &settings.torrent;
            if p.dht != n.dht {
                deferred.keys.push("torrent.dht");
            }
            if p.listen_port != n.listen_port {
                deferred.keys.push("torrent.listen_port");
            }
            if previous.network.ipv4_only != settings.network.ipv4_only {
                deferred.keys.push("network.ipv4_only");
            }
            if crate::limits::upload_disabled(p) != crate::limits::upload_disabled(n) {
                deferred.keys.push("torrent.seed_when_complete");
            }
            if !n.additional_trackers.is_empty() {
                for entry in self.entries.lock().values() {
                    if !entry.trackers.is_private() && entry.parsed.lock().is_some() {
                        let _ = entry
                            .trackers
                            .add(&n.additional_trackers, TrackerSource::Settings);
                    }
                }
            }
        }
        if !deferred.keys.is_empty() {
            info!(?deferred.keys, "torrent settings deferred until restart");
        }
        deferred
    }

    /// Persist librqbit state and stop networking. The engine is unusable afterwards.
    pub async fn shutdown(&self) {
        let entries: Vec<Arc<TaskEntry>> = self.entries.lock().values().cloned().collect();
        for e in entries {
            e.running.store(false, Ordering::Relaxed);
        }
        if let Some(session) = self.session_if_started() {
            session.stop().await;
        }
    }

    /// Parse `.torrent` bytes for the add dialog (no session needed).
    pub fn parse_torrent(bytes: &[u8]) -> Result<TorrentInfo, TaskError> {
        metainfo::parse_torrent(bytes).map(|p| p.info)
    }

    /// Fetch a magnet's metadata (DHT/trackers/peers) without downloading. Returns the
    /// description and the metainfo bytes so the services layer can store a blob and turn the
    /// task into a `.torrent` task.
    pub async fn resolve_magnet(
        &self,
        uri: &str,
        timeout: Duration,
    ) -> Result<(TorrentInfo, Vec<u8>), TaskError> {
        let preview = metainfo::parse_magnet(uri)?;
        let session = self.session().await?;
        if let Some(handle) = session.get(preview.info_hash) {
            if let Ok(bytes) =
                handle.with_metadata(|m| -> Vec<u8> { m.torrent_bytes.as_ref().to_vec() })
            {
                let parsed = metainfo::parse_torrent(&bytes)?;
                return Ok((parsed.info, bytes));
            }
        }
        let resolved = tokio::time::timeout(timeout, session.resolve_magnet(uri))
            .await
            .map_err(|_| {
                TaskError::new(
                    ErrorKind::NoPeers,
                    format!(
                        "no peer supplied metadata for {} within {}s",
                        redact(uri),
                        timeout.as_secs()
                    ),
                )
            })??;
        let bytes = resolved.torrent_bytes.to_vec();
        let parsed = metainfo::parse_torrent(&bytes)?;
        Ok((parsed.info, bytes))
    }

    // ----- control surface --------------------------------------------------------------

    pub(crate) fn entry(&self, task_id: &TaskId) -> Arc<TaskEntry> {
        self.entries
            .lock()
            .entry(task_id.clone())
            .or_insert_with(|| TaskEntry::new(task_id.clone()))
            .clone()
    }

    fn known(&self, task_id: &TaskId) -> Result<Arc<TaskEntry>, TaskError> {
        self.entries
            .lock()
            .get(task_id)
            .filter(|e| e.parsed.lock().is_some())
            .cloned()
            .ok_or_else(|| {
                TaskError::new(
                    ErrorKind::NotFound,
                    format!("task {task_id} is not loaded in the torrent engine"),
                )
            })
    }

    /// Override seeding limits for a task (applies live; `None` fields inherit).
    pub fn set_seeding_limits(&self, task_id: &TaskId, limits: SeedingLimits) {
        *self.entry(task_id).seeding_override.lock() = Some(limits);
    }

    pub fn trackers(&self, task_id: &TaskId) -> Vec<TrackerStatus> {
        self.entries
            .lock()
            .get(task_id)
            .map(|e| e.trackers.statuses())
            .unwrap_or_default()
    }

    /// Add trackers (public torrents only). They are probed immediately and handed to librqbit
    /// through a re-add of the torrent within a couple of seconds.
    pub fn add_trackers(&self, task_id: &TaskId, urls: Vec<String>) -> Result<usize, TaskError> {
        self.known(task_id)?
            .trackers
            .add(&urls, TrackerSource::User)
    }

    pub fn remove_tracker(&self, task_id: &TaskId, url: &str) -> Result<(), TaskError> {
        self.known(task_id)?.trackers.remove(url)
    }

    pub fn set_tracker_enabled(
        &self,
        task_id: &TaskId,
        url: &str,
        enabled: bool,
    ) -> Result<(), TaskError> {
        self.known(task_id)?.trackers.set_enabled(url, enabled)
    }

    /// Probe every enabled tracker now.
    pub fn reannounce(&self, task_id: &TaskId) -> Result<(), TaskError> {
        self.known(task_id)?.trackers.force_all();
        Ok(())
    }

    /// Download a newline-separated tracker list and add it to every loaded public torrent
    /// (and to torrents started later). Returns the number of valid tracker URLs in the list.
    pub async fn refresh_tracker_list(&self, url: &str) -> Result<u32, TaskError> {
        let parsed = url::Url::parse(url)
            .map_err(|e| TaskError::new(ErrorKind::InvalidUrl, format!("tracker list URL: {e}")))?;
        if parsed.scheme() != "https" && parsed.scheme() != "http" {
            return Err(TaskError::new(
                ErrorKind::UnsupportedScheme,
                "tracker list must be fetched over HTTP(S)",
            ));
        }
        let response = self
            .http
            .get(parsed)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| TaskError::new(ErrorKind::ConnectionReset, redact(&e.to_string())))?;
        if !response.status().is_success() {
            return Err(TaskError::from_http_status(response.status().as_u16(), url));
        }
        let body = response
            .bytes()
            .await
            .map_err(|e| TaskError::new(ErrorKind::Truncated, redact(&e.to_string())))?;
        if body.len() > 1024 * 1024 {
            return Err(TaskError::new(
                ErrorKind::UnexpectedContent,
                "tracker list larger than 1 MiB",
            ));
        }
        let list = trackers::parse_tracker_list(&String::from_utf8_lossy(&body));
        let count = u32::try_from(list.len()).unwrap_or(u32::MAX);
        for entry in self.entries.lock().values() {
            if !entry.trackers.is_private() && entry.parsed.lock().is_some() {
                if let Err(e) = entry.trackers.add(&list, TrackerSource::Settings) {
                    warn!(task = %entry.task_id, "tracker list not applied: {e}");
                }
            }
        }
        *self.curated_trackers.lock() = list;
        Ok(count)
    }

    pub fn peers(&self, task_id: &TaskId) -> Vec<PeerInfo> {
        let Some(entry) = self.entries.lock().get(task_id).cloned() else {
            return Vec::new();
        };
        let Some(handle) = entry.handle.lock().clone() else {
            return Vec::new();
        };
        let raw = status::raw_peers(&handle);
        let now = std::time::Instant::now();
        let mut sampler = entry.peers.lock();
        sampler.sample(now, raw)
    }

    /// Static description merged with live counters; `None` until the torrent was loaded.
    pub fn torrent_info(&self, task_id: &TaskId) -> Option<TorrentInfo> {
        let entry = self.entries.lock().get(task_id).cloned()?;
        let parsed = entry.parsed.lock().clone()?;
        let handle = entry.handle.lock().clone();
        let session = self.session_if_started();
        let snap = handle.as_ref().map(status::snapshot).unwrap_or_default();
        let haves = match (&session, parsed.info_hash) {
            (Some(s), hash) => s.haves(hash),
            _ => None,
        };
        let selection = entry.selection.lock().clone();
        let ctx = status::LiveContext {
            registry: &entry.trackers,
            uploaded_total: entry.uploaded_total(&snap),
            seeding_since: *entry.seeding_since.lock(),
            dht_nodes: session.map(|s| s.dht_nodes()).unwrap_or(0),
            haves,
            selected: &selection.selected,
            priorities: &selection.priority,
        };
        Some(status::torrent_info(&parsed, &snap, &ctx))
    }

    fn drop_entry(&self, task_id: &TaskId) -> Option<Arc<TaskEntry>> {
        self.entries.lock().remove(task_id)
    }
}

#[async_trait::async_trait]
impl Transfer for TorrentEngine {
    fn kinds(&self) -> &'static [TaskKind] {
        &[TaskKind::Torrent, TaskKind::Magnet]
    }

    async fn run(&self, ctx: TransferContext) -> TransferOutcome {
        let entry = self.entry(&ctx.task.id);
        if entry.running.swap(true, Ordering::AcqRel) {
            return TransferOutcome::Failed(TaskError::internal(
                "torrent task is already running in this engine",
            ));
        }
        let outcome = Runner::new(self, ctx, entry.clone()).run().await;
        entry.running.store(false, Ordering::Release);
        outcome
    }

    async fn probe(
        &self,
        task: &Task,
        _settings: Arc<Settings>,
        _secrets: TransferSecrets,
        _clients: Arc<osprey_runtime::net::ClientFactory>,
    ) -> Result<ResolvedMetadata, TaskError> {
        let info = match &task.source {
            Source::TorrentFile { info_hash, .. } => {
                let bytes = self.blobs.torrent_bytes(info_hash).await.ok_or_else(|| {
                    TaskError::new(
                        ErrorKind::InvalidTorrent,
                        "torrent file is not in the store",
                    )
                })?;
                Self::parse_torrent(&bytes)?
            }
            Source::Magnet { uri } => self.resolve_magnet(uri, Duration::from_secs(60)).await?.0,
            _ => {
                return Err(TaskError::new(
                    ErrorKind::UnsupportedScheme,
                    "torrent engine only handles .torrent files and magnet links",
                ))
            }
        };
        Ok(ResolvedMetadata {
            name: Some(info.name.clone()),
            total: Some(info.total_size),
            torrent: Some(info),
            ..Default::default()
        })
    }

    async fn forget(&self, task: &Task, delete_files: bool) {
        let entry = self.drop_entry(&task.id);
        let hash = entry
            .as_ref()
            .and_then(|e| *e.info_hash.lock())
            .or_else(|| match &task.source {
                Source::TorrentFile { info_hash, .. } => {
                    Id20::from_bytes(&hex::decode(info_hash).ok()?).ok()
                }
                Source::Magnet { uri } => metainfo::parse_magnet(uri).ok().map(|m| m.info_hash),
                _ => None,
            });
        let (Some(hash), Some(session)) = (hash, self.session_if_started()) else {
            return;
        };
        if let Err(e) = session.delete(hash, delete_files).await {
            warn!(task = %task.id, "forget: {}", e.message);
        }
    }
}
