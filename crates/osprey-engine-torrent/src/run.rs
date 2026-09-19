//! One `run()` of a torrent task: acquire metainfo, attach to librqbit, supervise until an
//! outcome. Everything here is driven by a 1 s tick and `TransferControl`.

use crate::engine::{Selection, TaskEntry, TorrentEngine};
use crate::limits::{self, SeedingPolicy};
use crate::metainfo::{self, ParsedTorrent};
use crate::session::{classify, ManagedTorrentHandle, TorrentSession};
use crate::status::{self, Snapshot};
use crate::trackers::{self, AnnounceStats, AnnounceStatsSource, ProbeContext};
use librqbit::{AddTorrentOptions, AddTorrentResponse};
use osprey_domain::checkpoint::{Checkpoint, TorrentCheckpoint};
use osprey_domain::{ErrorKind, LogLevel, Millis, Source, TaskError, TaskState};
use osprey_runtime::engine::{ResolvedMetadata, TransferContext, TransferOutcome};
use osprey_runtime::redact::redact;
use osprey_runtime::safety::validate_destination_dir;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

const TICK: Duration = Duration::from_secs(1);
const CHECKPOINT_EVERY: Duration = Duration::from_secs(10);
/// Option changes (limits, peer cap, tracker set) are applied through a re-add once they have
/// been stable for this long, so a slider drag does not thrash the torrent.
const READD_DEBOUNCE: Duration = Duration::from_secs(2);
const NO_PEERS_TIMEOUT: Duration = Duration::from_secs(600);

/// Early exit from a step: either a final outcome or a classified failure.
pub(crate) struct Stop(pub TransferOutcome);

impl From<TaskError> for Stop {
    fn from(e: TaskError) -> Self {
        Stop(TransferOutcome::Failed(e))
    }
}

/// Options librqbit only accepts at add time; a change means the torrent must be re-added.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AddFingerprint {
    download_limit: u64,
    upload_limit: u64,
    peer_limit: usize,
    tracker_generation: u64,
    output_folder: PathBuf,
}

struct Plan {
    bytes: Vec<u8>,
    opts: AddTorrentOptions,
    fingerprint: AddFingerprint,
    root: PathBuf,
}

struct StatsSource {
    handle: parking_lot::Mutex<Option<ManagedTorrentHandle>>,
    base: Arc<TaskEntry>,
}

impl AnnounceStatsSource for StatsSource {
    fn announce_stats(&self) -> AnnounceStats {
        let Some(handle) = self.handle.lock().clone() else {
            return AnnounceStats::default();
        };
        let snap = status::snapshot(&handle);
        AnnounceStats {
            uploaded: self.base.uploaded_total(&snap),
            downloaded: snap.downloaded,
            left: snap.total.saturating_sub(snap.downloaded),
        }
    }
}

pub(crate) struct Runner<'a> {
    engine: &'a TorrentEngine,
    ctx: TransferContext,
    entry: Arc<TaskEntry>,
    session: Option<Arc<TorrentSession>>,
    root: PathBuf,
    initial_peers: Vec<SocketAddr>,
    probe_source: Option<Arc<StatsSource>>,
}

impl<'a> Runner<'a> {
    pub fn new(engine: &'a TorrentEngine, ctx: TransferContext, entry: Arc<TaskEntry>) -> Self {
        Self {
            engine,
            ctx,
            entry,
            session: None,
            root: PathBuf::new(),
            initial_peers: Vec::new(),
            probe_source: None,
        }
    }

    pub async fn run(mut self) -> TransferOutcome {
        let cancel = CancellationToken::new();
        let outcome = match self.run_inner(&cancel).await {
            Ok(o) | Err(Stop(o)) => o,
        };
        cancel.cancel();
        if let TransferOutcome::Failed(e) = &outcome {
            self.ctx
                .sink
                .log(LogLevel::Error, "torrent.failed", redact(&e.message));
        }
        outcome
    }

    async fn run_inner(&mut self, cancel: &CancellationToken) -> Result<TransferOutcome, Stop> {
        validate_destination_dir(&self.ctx.task.directory)?;
        let session = self.engine.session().await?;
        self.session = Some(session.clone());

        let bytes = self.acquire_metainfo(&session).await?;
        let parsed = Arc::new(metainfo::parse_torrent(&bytes)?);
        *self.entry.info_hash.lock() = Some(parsed.info_hash);
        *self.entry.parsed.lock() = Some(parsed.clone());
        self.restore_from_checkpoint(&parsed);
        self.setup_trackers(&parsed);

        let plan = self.plan(&parsed);
        self.root = plan.root.clone();
        let handle = self.attach(&session, &parsed, plan).await?;
        *self.entry.handle.lock() = Some(handle.clone());
        self.emit_metadata(&parsed, &handle);
        self.spawn_probes(&session, &parsed, &handle, cancel);
        self.supervise(&session, &parsed, handle).await
    }

    // ----- metainfo -----------------------------------------------------------------------

    /// `.torrent` bytes from the blob store, or a magnet resolved through the session.
    async fn acquire_metainfo(&mut self, session: &Arc<TorrentSession>) -> Result<Vec<u8>, Stop> {
        match &self.ctx.task.source {
            Source::TorrentFile { info_hash, .. } => self
                .engine
                .blobs
                .torrent_bytes(info_hash)
                .await
                .ok_or_else(|| {
                    Stop::from(TaskError::new(
                        ErrorKind::InvalidTorrent,
                        format!("torrent file {info_hash} is not in the store"),
                    ))
                }),
            Source::Magnet { uri } => {
                let preview = metainfo::parse_magnet(uri)?;
                if let Some(handle) = session.get(preview.info_hash) {
                    if let Ok(bytes) =
                        handle.with_metadata(|m| -> Vec<u8> { m.torrent_bytes.as_ref().to_vec() })
                    {
                        return Ok(bytes);
                    }
                }
                self.ctx
                    .sink
                    .state(TaskState::Resolving, Some("fetching metadata".into()));
                self.ctx.sink.metadata(ResolvedMetadata {
                    name: Some(preview.info.name.clone()),
                    torrent: Some(preview.info.clone()),
                    ..Default::default()
                });
                let resolved = tokio::select! {
                    r = session.resolve_magnet(uri) => r?,
                    _ = self.ctx.control.stopped() => return Err(Stop(self.ctx.control.stop_outcome())),
                };
                self.initial_peers = resolved.seen_peers;
                Ok(resolved.torrent_bytes.to_vec())
            }
            _ => Err(Stop::from(TaskError::new(
                ErrorKind::UnsupportedScheme,
                "torrent engine only handles .torrent files and magnet links",
            ))),
        }
    }

    fn restore_from_checkpoint(&self, parsed: &ParsedTorrent) {
        let n = parsed.info.files.len();
        let mut selection = Selection::all(n, &parsed.padding);
        let cp = self
            .ctx
            .checkpoint
            .as_ref()
            .and_then(Checkpoint::as_torrent);
        if let Some(cp) = cp {
            if let Some(sel) = &cp.selected_files {
                for s in selection.selected.iter_mut() {
                    *s = false;
                }
                for i in sel {
                    selection.set(*i as usize, true, 1);
                }
            }
            for (i, p) in &cp.priorities {
                if let Some(slot) = selection.priority.get_mut(*i as usize) {
                    *slot = (*p).min(2);
                }
            }
            self.entry
                .uploaded_base
                .store(cp.uploaded, Ordering::Relaxed);
            *self.entry.seeding_since.lock() = cp.seeding_since;
        } else if let Some(t) = &self.ctx.task.torrent {
            for f in &t.files {
                selection.set(f.index as usize, f.selected, f.priority);
            }
        }
        // Whatever the UI pushed before we subscribed wins.
        let mut rx = self.ctx.control.file_selection();
        if let Some(updates) = rx.borrow_and_update().as_ref() {
            for u in updates {
                selection.set(u.index as usize, u.selected, u.priority);
            }
        }
        *self.entry.selection.lock() = selection;
    }

    fn setup_trackers(&self, parsed: &ParsedTorrent) {
        let cp = self
            .ctx
            .checkpoint
            .as_ref()
            .and_then(Checkpoint::as_torrent);
        let magnet: Vec<String> = match &self.ctx.task.source {
            Source::Magnet { uri } => metainfo::parse_magnet(uri)
                .map(|m| m.trackers)
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let disabled = cp.map(|c| c.disabled_trackers.clone()).unwrap_or_default();
        let mut extra = cp.map(|c| c.extra_trackers.clone()).unwrap_or_default();
        // Keep user trackers added while the task was paused (registry survives runs).
        extra.extend(self.entry.trackers.user_urls());
        let settings = self.engine.settings();
        let mut from_settings = settings.torrent.additional_trackers.clone();
        from_settings.extend(self.engine.curated_trackers.lock().iter().cloned());
        self.entry.trackers.reset(
            parsed.info.private,
            &parsed.trackers,
            &magnet,
            &disabled,
            &extra,
            &from_settings,
        );
    }

    // ----- add / adopt ----------------------------------------------------------------------

    fn desired_fingerprint(&self, output_folder: PathBuf) -> AddFingerprint {
        let settings = self.engine.settings();
        AddFingerprint {
            download_limit: self.ctx.control.download_limit(),
            upload_limit: self.ctx.control.upload_limit(),
            peer_limit: self
                .ctx
                .task
                .options
                .max_peers
                .filter(|n| *n > 0)
                .unwrap_or(settings.torrent.max_peers_per_torrent.max(1))
                as usize,
            tracker_generation: self.entry.trackers.generation(),
            output_folder,
        }
    }

    fn plan(&self, parsed: &ParsedTorrent) -> Plan {
        let (output_folder, root) = metainfo::output_layout(&self.ctx.task.directory, parsed);
        let fingerprint = self.desired_fingerprint(output_folder.clone());
        let tiers = if self.engine.tuning.disable_trackers {
            Vec::new()
        } else {
            self.entry.trackers.enabled_tiers()
        };
        let bytes = metainfo::wrap_with_trackers(
            &parsed.info_bytes,
            &tiers,
            parsed.comment.as_deref(),
            parsed.created_by.as_deref(),
        );
        let mut initial_peers = self.engine.tuning.initial_peers.clone();
        initial_peers.extend(self.initial_peers.iter().copied());
        let opts = AddTorrentOptions {
            paused: false,
            only_files: self.entry.selection.lock().only_files(),
            overwrite: true,
            output_folder: Some(output_folder.to_string_lossy().into_owned()),
            ratelimits: limits::limits_config(fingerprint.download_limit, fingerprint.upload_limit),
            initial_peers: (!initial_peers.is_empty()).then_some(initial_peers),
            peer_limit: Some(fingerprint.peer_limit),
            ..Default::default()
        };
        Plan {
            bytes,
            opts,
            fingerprint,
            root,
        }
    }

    /// Add the torrent, or adopt one the session already manages (restored from librqbit's
    /// persistence or left by a previous run), re-adding when its fixed options differ.
    async fn attach(
        &self,
        session: &Arc<TorrentSession>,
        parsed: &ParsedTorrent,
        plan: Plan,
    ) -> Result<ManagedTorrentHandle, Stop> {
        self.ctx
            .sink
            .state(TaskState::Connecting, Some("checking existing data".into()));
        let existing = session.get(parsed.info_hash);
        let handle = match existing {
            None => match session.add(plan.bytes, plan.opts).await? {
                AddTorrentResponse::Added(_, h) | AddTorrentResponse::AlreadyManaged(_, h) => h,
                AddTorrentResponse::ListOnly(_) => {
                    return Err(TaskError::internal("add returned list-only").into())
                }
            },
            Some(h) => {
                let same_trackers = same_trackers(
                    &h,
                    &self.entry.trackers.enabled_tiers(),
                    self.engine.tuning.disable_trackers,
                );
                let same_folder = h.output_folder() == plan.fingerprint.output_folder;
                // Restored torrents carry no limits / peer cap, so they only match unlimited.
                let restored_matches = plan.fingerprint.download_limit == 0
                    && plan.fingerprint.upload_limit == 0
                    && self.ctx.task.options.max_peers.is_none();
                if same_trackers && same_folder && restored_matches {
                    debug!(task = %self.ctx.task.id, "adopting managed torrent");
                    if let Some(only) = plan.opts.only_files.clone() {
                        let want: HashSet<usize> = only.into_iter().collect();
                        if h.only_files()
                            .map(|v| v.into_iter().collect::<HashSet<_>>())
                            != Some(want.clone())
                        {
                            session.update_only_files(&h, &want).await?;
                        }
                    }
                    if h.is_paused() {
                        session.unpause(&h).await?;
                    }
                    h
                } else {
                    debug!(task = %self.ctx.task.id, "re-adding managed torrent with new options");
                    session.readd(&h, plan.bytes, plan.opts).await?
                }
            }
        };
        Ok(handle)
    }

    fn emit_metadata(&self, parsed: &ParsedTorrent, handle: &ManagedTorrentHandle) {
        let snap = status::snapshot(handle);
        let info = self.live_info(parsed, &snap);
        self.ctx.sink.metadata(ResolvedMetadata {
            name: Some(parsed.info.name.clone()),
            total: Some(parsed.info.total_size),
            torrent: Some(info),
            file_path: Some(self.root.clone()),
            ..Default::default()
        });
    }

    fn live_info(
        &self,
        parsed: &ParsedTorrent,
        snap: &Snapshot,
    ) -> osprey_domain::torrent::TorrentInfo {
        let selection = self.entry.selection.lock().clone();
        let session = self.session.as_ref();
        let ctx = status::LiveContext {
            registry: &self.entry.trackers,
            uploaded_total: self.entry.uploaded_total(snap),
            seeding_since: *self.entry.seeding_since.lock(),
            dht_nodes: session.map(|s| s.dht_nodes()).unwrap_or(0),
            haves: session.and_then(|s| s.haves(parsed.info_hash)),
            selected: &selection.selected,
            priorities: &selection.priority,
        };
        status::torrent_info(parsed, snap, &ctx)
    }

    fn spawn_probes(
        &mut self,
        session: &Arc<TorrentSession>,
        parsed: &ParsedTorrent,
        handle: &ManagedTorrentHandle,
        cancel: &CancellationToken,
    ) {
        if self.engine.tuning.disable_trackers {
            return;
        }
        let source = Arc::new(StatsSource {
            handle: parking_lot::Mutex::new(Some(handle.clone())),
            base: self.entry.clone(),
        });
        self.probe_source = Some(source.clone());
        let interval = self.engine.settings().torrent.announce_interval_seconds;
        trackers::spawn_probe_loop(ProbeContext {
            registry: self.entry.trackers.clone(),
            http: self.engine.http.clone(),
            info_hash: parsed.info_hash.0,
            peer_id: handle.shared().peer_id.0,
            port: session.announce_port(),
            stats: source,
            interval_override: (interval > 0).then(|| Duration::from_secs(u64::from(interval))),
            sink: self.ctx.sink.clone(),
            cancel: cancel.clone(),
        });
    }

    // ----- supervision ---------------------------------------------------------------------

    async fn supervise(
        &mut self,
        session: &Arc<TorrentSession>,
        parsed: &Arc<ParsedTorrent>,
        mut handle: ManagedTorrentHandle,
    ) -> Result<TransferOutcome, Stop> {
        let mut sel_rx = self.ctx.control.file_selection();
        sel_rx.mark_unchanged();
        let mut applied = self.desired_fingerprint(handle.output_folder().to_path_buf());
        let mut pending_since: Option<Instant> = None;
        let mut last_checkpoint = Instant::now();
        let mut last_peer_activity = Instant::now();
        let mut reported = TaskState::Connecting;
        let mut sequential = self.ctx.control.sequential.load(Ordering::Relaxed);
        let mut last_downloaded = 0u64;

        loop {
            tokio::select! {
                _ = self.ctx.control.stopped() => {
                    return Ok(self.stop(session, parsed, &handle).await);
                }
                changed = sel_rx.changed() => {
                    if changed.is_ok() {
                        let updates = sel_rx.borrow_and_update().clone();
                        if let Some(updates) = updates {
                            self.apply_selection(session, &handle, &updates).await;
                        }
                    }
                }
                _ = tokio::time::sleep(TICK) => {}
            }

            let mut snap = status::snapshot(&handle);
            if let Some(err) = &snap.error {
                return Err(classify("torrent stopped", &anyhow::anyhow!("{err}")).into());
            }
            if snap.initializing {
                // While librqbit checks pieces, `progress_bytes` counts checked bytes; keep
                // the last real figure so the progress bar does not fall back to zero.
                snap.downloaded = last_downloaded;
            } else {
                last_downloaded = snap.downloaded;
            }

            // State reporting.
            if snap.live && snap.finished && reported != TaskState::Seeding {
                reported = TaskState::Seeding;
                let mut since = self.entry.seeding_since.lock();
                if since.is_none() {
                    *since = Some(Millis::now());
                }
                self.ctx.sink.state(TaskState::Seeding, None);
                self.ctx.sink.log(
                    LogLevel::Info,
                    "torrent.complete",
                    "all selected files verified".into(),
                );
            } else if snap.live && !snap.finished && reported != TaskState::Downloading {
                reported = TaskState::Downloading;
                self.ctx.sink.state(TaskState::Downloading, None);
            }

            let uploaded = self.entry.uploaded_total(&snap);
            let progress = status::progress(&snap, uploaded);
            self.ctx.control.counters.set_downloaded(snap.downloaded);
            self.ctx
                .control
                .counters
                .uploaded
                .store(uploaded, Ordering::Relaxed);
            self.ctx.sink.progress(progress);

            if snap.peers_live > 0 || snap.finished || !snap.live || session.dht_nodes() > 0 {
                last_peer_activity = Instant::now();
            } else if last_peer_activity.elapsed() >= NO_PEERS_TIMEOUT {
                let _ = session.pause(&handle).await;
                self.emit_checkpoint(parsed, &snap);
                return Err(TaskError::new(
                    ErrorKind::NoPeers,
                    "no peers and no DHT nodes for 10 minutes",
                )
                .into());
            }

            if last_checkpoint.elapsed() >= CHECKPOINT_EVERY {
                self.emit_checkpoint(parsed, &snap);
                last_checkpoint = Instant::now();
            }

            let seq = self.ctx.control.sequential.load(Ordering::Relaxed);
            if seq != sequential {
                sequential = seq;
                self.ctx.sink.log(
                    LogLevel::Info,
                    "torrent.sequential",
                    "librqbit always fetches pieces in file order; the flag is recorded only"
                        .into(),
                );
            }

            if snap.finished {
                let policy = SeedingPolicy::resolve(
                    &self.ctx.task.options,
                    self.entry.seeding_override.lock().as_ref(),
                    &self.engine.settings().torrent,
                );
                let since = *self.entry.seeding_since.lock();
                if let Some(reason) =
                    policy.stop_reason(uploaded, snap.downloaded, since, Millis::now())
                {
                    info!(task = %self.ctx.task.id, reason, "seeding finished");
                    self.ctx
                        .sink
                        .log(LogLevel::Info, "torrent.seeding_done", reason.into());
                    session.pause(&handle).await?;
                    self.emit_checkpoint(parsed, &snap);
                    return Ok(TransferOutcome::Completed {
                        file_path: self.root.clone(),
                        bytes: snap.total,
                    });
                }
            }

            // Options librqbit fixes at add time: re-add after a quiet period.
            let desired = self.desired_fingerprint(applied.output_folder.clone());
            if desired != applied && snap.live {
                let since = *pending_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= READD_DEBOUNCE {
                    self.ctx.sink.log(
                        LogLevel::Debug,
                        "torrent.readd",
                        "applying changed limits/trackers".into(),
                    );
                    self.entry
                        .uploaded_base
                        .fetch_add(snap.uploaded_live, Ordering::Relaxed);
                    let plan = self.plan(parsed);
                    let new_handle = session.readd(&handle, plan.bytes, plan.opts).await?;
                    if let Some(src) = &self.probe_source {
                        *src.handle.lock() = Some(new_handle.clone());
                    }
                    *self.entry.handle.lock() = Some(new_handle.clone());
                    handle = new_handle;
                    applied = plan.fingerprint;
                    pending_since = None;
                }
            } else {
                pending_since = None;
            }
        }
    }

    async fn apply_selection(
        &self,
        session: &Arc<TorrentSession>,
        handle: &ManagedTorrentHandle,
        updates: &[osprey_runtime::engine::FileSelectionUpdate],
    ) {
        let only = {
            let mut sel = self.entry.selection.lock();
            for u in updates {
                sel.set(u.index as usize, u.selected, u.priority);
            }
            sel.only_files()
                .unwrap_or_else(|| (0..sel.selected.len()).collect())
        };
        let want: HashSet<usize> = only.into_iter().collect();
        match session.update_only_files(handle, &want).await {
            Ok(()) => self.ctx.sink.log(
                LogLevel::Info,
                "torrent.selection",
                format!(
                    "{} of {} files selected",
                    want.len(),
                    self.entry.selection.lock().selected.len()
                ),
            ),
            Err(e) => {
                warn!(task = %self.ctx.task.id, "file selection: {}", e.message);
                self.ctx
                    .sink
                    .log(LogLevel::Warn, "torrent.selection_failed", e.message);
            }
        }
    }

    async fn stop(
        &self,
        session: &Arc<TorrentSession>,
        parsed: &ParsedTorrent,
        handle: &ManagedTorrentHandle,
    ) -> TransferOutcome {
        let snap = status::snapshot(handle);
        let paused = tokio::time::timeout(Duration::from_secs(5), session.pause(handle)).await;
        match paused {
            Ok(Ok(())) => self.emit_checkpoint(parsed, &snap),
            Ok(Err(e)) => warn!(task = %self.ctx.task.id, "pause: {}", e.message),
            Err(_) => {
                warn!(task = %self.ctx.task.id, "pause timed out; keeping previous checkpoint")
            }
        }
        self.entry
            .uploaded_base
            .fetch_add(snap.uploaded_live, Ordering::Relaxed);
        self.ctx.control.stop_outcome()
    }

    fn emit_checkpoint(&self, parsed: &ParsedTorrent, snap: &Snapshot) {
        let selection = self.entry.selection.lock().clone();
        let priorities = selection
            .priority
            .iter()
            .enumerate()
            .filter(|(_, p)| **p != 1)
            .filter_map(|(i, p)| Some((u32::try_from(i).ok()?, *p)))
            .collect();
        let cp = TorrentCheckpoint {
            info_hash: parsed.info_hash.as_string(),
            selected_files: selection.only_files().map(|_| selection.selected_indices()),
            priorities,
            uploaded: self.entry.uploaded_total(snap),
            downloaded: snap.downloaded,
            seeding_since: *self.entry.seeding_since.lock(),
            sequential: self.ctx.control.sequential.load(Ordering::Relaxed),
            output_folder: metainfo::output_layout(&self.ctx.task.directory, parsed).0,
            disabled_trackers: self.entry.trackers.disabled_urls(),
            extra_trackers: self.entry.trackers.user_urls(),
        };
        self.ctx.sink.checkpoint(Checkpoint::Torrent(cp));
    }
}

/// Does the managed torrent already announce to exactly our enabled trackers?
fn same_trackers(
    handle: &ManagedTorrentHandle,
    tiers: &[Vec<String>],
    trackers_disabled: bool,
) -> bool {
    if trackers_disabled {
        return true;
    }
    let want: HashSet<String> = tiers
        .iter()
        .flatten()
        .filter_map(|u| url::Url::parse(u).ok())
        .map(|u| u.to_string())
        .collect();
    let have: HashSet<String> = handle
        .shared()
        .trackers
        .iter()
        .map(|u| u.to_string())
        .collect();
    want == have
}
