//! The `Engine`: owns every long-lived component and implements [`EngineApi`].
//!
//! Construction (`Engine::start`) follows the order in `docs/architecture/005-services.md`:
//! paths → instance lock → logging → store → settings → clients → bus → engines → load +
//! recover → seed builtins → tickers → `EngineStarted` → resume per recovery.

use crate::api::*;
use crate::automation::AutomationRunner;
use crate::bandwidth::BandwidthManager;
use crate::bootstrap::EngineConfig;
use crate::credentials::CredentialStore;
use crate::devices::DeviceManager;
use crate::disk::DiskMonitor;
use crate::persist::{PersistOp, Persister};
use crate::queues::QueueManager;
use crate::scheduler::Scheduler;
use crate::tasks::{LogRings, TaskTable};
use crate::torrents::StoreBlobs;
use crate::updates::UpdateState;
use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use swoop_domain::automation::{AutomationRule, AutomationRun};
use swoop_domain::category::Category;
use swoop_domain::device::{AuditEntry, Device, Scope};
use swoop_domain::events::{GlobalStats, TaskLogEntry};
use swoop_domain::history::{HistoryEntry, HistoryQuery};
use swoop_domain::media::DetectedMedia;
use swoop_domain::queue::{Queue, QueueSummary, TrafficMode};
use swoop_domain::rules::{Rule, RuleAction, RuleSubject};
use swoop_domain::schedule::{EnvironmentSnapshot, Schedule};
use swoop_domain::settings::Settings;
use swoop_domain::state::PauseReason;
use swoop_domain::torrent::PeerInfo;
use swoop_domain::*;
use swoop_engine_ftp::FtpEngine;
use swoop_engine_http::HttpEngine;
use swoop_engine_torrent::{SessionTuning, TorrentEngine, TorrentEngineConfig};
use swoop_grabber::Crawler;
use swoop_media::HlsEngine;
use swoop_plugins::PluginHost;
use swoop_runtime::bus::EventBus;
use swoop_runtime::lock::InstanceLock;
use swoop_runtime::net::ClientFactory;
use swoop_runtime::paths::AppPaths;
use swoop_store::{ProgressBatcher, RecoveryReport, Store};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

/// Progress coalescing interval of the bus.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
/// Bus capacity.
const BUS_CAPACITY: usize = 8192;
/// Time budget for engines to pause at shutdown.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(10);

/// The production [`EngineApi`] implementation.
pub struct Engine {
    pub(crate) this: Weak<Engine>,
    pub(crate) config: EngineConfig,
    pub(crate) paths: AppPaths,
    _lock: Option<InstanceLock>,
    started_at: Instant,
    pub(crate) store: Arc<Store>,
    pub(crate) persist: Persister,
    pub(crate) progress_writer: ProgressBatcher,
    pub(crate) settings_cell: RwLock<Arc<Settings>>,
    pub(crate) bus: EventBus,
    pub(crate) clients: Arc<ClientFactory>,
    pub(crate) http: Arc<HttpEngine>,
    pub(crate) ftp: Arc<FtpEngine>,
    pub(crate) hls: Arc<HlsEngine>,
    pub(crate) torrent: Arc<TorrentEngine>,
    pub(crate) tasks: TaskTable,
    pub(crate) log_rings: LogRings,
    pub(crate) log_batch: Mutex<Vec<TaskLogEntry>>,
    pub(crate) queues: QueueManager,
    pub(crate) categories: RwLock<Vec<Category>>,
    pub(crate) rules: RwLock<Vec<Rule>>,
    pub(crate) bandwidth: BandwidthManager,
    pub(crate) scheduler: Scheduler,
    pub(crate) automation: AutomationRunner,
    pub(crate) disk: DiskMonitor,
    pub(crate) devices: DeviceManager,
    pub(crate) credentials: CredentialStore,
    pub(crate) grabber: Arc<Crawler>,
    pub(crate) plugins: PluginHost,
    pub(crate) updates: UpdateState,
    pub(crate) environment: RwLock<EnvironmentSnapshot>,
    pub(crate) stats: RwLock<GlobalStats>,
    pub(crate) active_window: AtomicBool,
    pub(crate) shutting_down: AtomicBool,
    pub(crate) admission: Arc<Notify>,
    tickers: Mutex<Vec<JoinHandle<()>>>,
    pub(crate) local_token: String,
    logs: crate::logs::LogHandle,
}

impl Engine {
    /// Bring the engine up (see the module docs for the order).
    pub async fn start(config: EngineConfig) -> DomainResult<Arc<Engine>> {
        // 1. paths
        let paths = match &config.data_dir {
            Some(d) => {
                let d = AppPaths::expand_home(&d.to_string_lossy());
                AppPaths {
                    data_dir: d.clone(),
                    config_dir: d.clone(),
                    cache_dir: d.join("cache"),
                    log_dir: d.join("logs"),
                }
            }
            None => AppPaths::resolve(),
        };
        paths
            .ensure()
            .map_err(|e| DomainError::Storage(format!("create data directories: {e}")))?;
        // 2. instance lock
        let lock = if config.skip_instance_lock {
            None
        } else {
            match InstanceLock::try_acquire(&paths.data_dir.join("swoop.lock")) {
                Ok(Some(l)) => Some(l),
                Ok(None) => {
                    return Err(DomainError::Conflict(
                        "another Swoop instance is using this data directory".into(),
                    ))
                }
                Err(e) => return Err(DomainError::Storage(format!("instance lock: {e}"))),
            }
        };
        // 3. logging (idempotent)
        let logs = crate::logs::init_logging(&paths.log_dir, "info", 14);
        // 4. store
        let db = paths.database();
        let store = tokio::task::spawn_blocking(move || Store::open(&db))
            .await
            .map_err(DomainError::internal)??;
        // 5. settings
        let stored = store.load_settings().await?;
        let mut settings = match stored.map(crate::settings::normalise) {
            Some(Ok(s)) => s,
            Some(Err(e)) => {
                tracing::warn!(error = %e, "stored settings invalid; using defaults");
                default_settings()
            }
            None => default_settings(),
        };
        if let Some(d) = &config.download_dir {
            settings.storage.download_directory = AppPaths::expand_home(&d.to_string_lossy());
        }
        let settings = crate::settings::normalise(settings)?;
        if let Err(e) = tokio::fs::create_dir_all(&settings.storage.download_directory).await {
            tracing::warn!(error = %e, "download directory could not be created");
        }
        store.save_settings(&settings).await?;
        let settings = Arc::new(settings);
        crate::logs::set_level(&settings.privacy.log_level);
        // 6. clients
        let clients = ClientFactory::new(settings.clone());
        // 7. bus
        let bus = EventBus::new(
            &tokio::runtime::Handle::current(),
            BUS_CAPACITY,
            PROGRESS_INTERVAL,
        );
        // 8. engines
        let http = Arc::new(HttpEngine::new());
        let ftp = FtpEngine::new();
        let hls = HlsEngine::new();
        let torrent = TorrentEngine::new(TorrentEngineConfig {
            session_dir: paths.torrent_session_dir(),
            default_output: settings.storage.download_directory.clone(),
            settings: settings.clone(),
            blobs: Arc::new(StoreBlobs {
                store: store.clone(),
            }),
            http: clients
                .default_client()
                .map_err(|e| DomainError::Engine(e.message))?,
            tuning: SessionTuning {
                client_name: Some(format!("Swoop/{}", config.app_version)),
                ..Default::default()
            },
        })
        .map_err(|e| DomainError::Engine(e.message))?;
        // local token
        let local_token = load_or_create_local_token(&paths.local_token_file())?;
        let persist = Persister::spawn(store.clone());
        let progress_writer = store.progress_writer();

        let engine = Arc::new_cyclic(|this: &Weak<Engine>| Engine {
            this: this.clone(),
            paths,
            _lock: lock,
            started_at: Instant::now(),
            store: store.clone(),
            persist,
            progress_writer,
            settings_cell: RwLock::new(settings.clone()),
            bus,
            clients: clients.clone(),
            http,
            ftp,
            hls,
            torrent,
            tasks: TaskTable::default(),
            log_rings: LogRings::default(),
            log_batch: Mutex::new(Vec::new()),
            queues: QueueManager::default(),
            categories: RwLock::new(Vec::new()),
            rules: RwLock::new(Vec::new()),
            bandwidth: BandwidthManager::new(),
            scheduler: Scheduler::default(),
            automation: AutomationRunner::new(clients.clone()),
            disk: DiskMonitor::default(),
            devices: DeviceManager::default(),
            credentials: CredentialStore::new(
                &config
                    .data_dir
                    .clone()
                    .unwrap_or_else(|| AppPaths::resolve().data_dir),
            ),
            grabber: Crawler::new(clients),
            plugins: PluginHost::new(AppPaths::resolve_plugins_dir(&config)),
            updates: UpdateState::default(),
            environment: RwLock::new(EnvironmentSnapshot::assume_desktop()),
            stats: RwLock::new(GlobalStats::default()),
            active_window: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
            admission: Arc::new(Notify::new()),
            tickers: Mutex::new(Vec::new()),
            local_token,
            logs,
            config,
        });
        engine.apply_settings(settings).await;
        // 9. config lists + builtins
        engine.load_config().await?;
        // 10. tasks + recovery
        let report = store
            .recover(&engine.settings().storage.temp_suffix)
            .await?;
        for t in &report.tasks {
            engine.tasks.insert(t.clone());
        }
        tracing::info!(
            scanned = report.scanned,
            requeued = report.requeued_with_checkpoint.len() + report.requeued_from_scratch.len(),
            ready = report.ready_to_complete.len(),
            paused_by_shutdown = report.paused_by_shutdown.len(),
            "recovery applied"
        );
        // 11. tickers
        engine.spawn_tickers();
        // 12. started
        engine.bus.publish(Event::EngineStarted {
            version: engine.config.app_version.clone(),
            at: Millis::now(),
        });
        // 13. resume per recovery
        engine.apply_recovery(report).await;
        Ok(engine)
    }

    async fn load_config(&self) -> DomainResult<()> {
        let mut queues = self.store.list_queues().await?;
        if queues.is_empty() {
            for q in Queue::builtin_defaults() {
                self.store.upsert_queue(&q).await?;
                queues.push(q);
            }
        } else if !queues.iter().any(|q| q.id == QueueId::default_queue()) {
            let mut q = Queue::new("Default", 3);
            q.id = QueueId::default_queue();
            q.builtin = true;
            self.store.upsert_queue(&q).await?;
            queues.push(q);
        }
        for q in &queues {
            self.bandwidth.apply_queue(q);
        }
        self.queues.replace_all(queues);
        let mut cats = self.store.list_categories().await?;
        if cats.is_empty() {
            for c in Category::builtin_defaults() {
                self.store.upsert_category(&c).await?;
                cats.push(c);
            }
        }
        *self.categories.write() = cats;
        *self.rules.write() = self.store.list_rules().await?;
        self.scheduler
            .replace_all(self.store.list_schedules().await?);
        self.automation
            .replace_all(self.store.list_automations().await?);
        Ok(())
    }

    /// Finish what recovery could not decide on its own.
    async fn apply_recovery(&self, report: RecoveryReport) {
        for id in &report.ready_to_complete {
            let Some(t) = self.tasks.snapshot(id) else {
                continue;
            };
            let path = t.target_path();
            let bytes = tokio::fs::metadata(&path)
                .await
                .map(|m| m.len())
                .unwrap_or(0);
            let run = crate::tasks::RunHandle::new(
                self.tasks.next_run_id(),
                swoop_runtime::engine::TransferControl::new(),
                swoop_runtime::RateLimiter::unlimited("recovery"),
                swoop_runtime::RateLimiter::unlimited("recovery-up"),
            );
            self.tasks.set_run(id, run.clone());
            self.handle_outcome(
                id,
                run.run_id,
                swoop_runtime::engine::TransferOutcome::Completed {
                    file_path: path,
                    bytes,
                },
            )
            .await;
        }
        for id in &report.paused_by_shutdown {
            self.unblock_task(id, &PauseReason::Shutdown).await;
        }
        // Tasks left in an active state without a run (Verifying/Processing/Resolving after
        // a crash) are re-queued through `Retrying`, which every active state may enter.
        for t in self.tasks.all() {
            if t.state.is_active()
                && self.tasks.run(&t.id).is_none()
                && t.state != TaskState::Seeding
            {
                let res = self.mutate_from(&t.id, |x| {
                    if x.state != TaskState::Queued {
                        if x.state.can_transition_to(TaskState::Queued) {
                            x.transition(TaskState::Queued)?;
                        } else {
                            x.transition(TaskState::Retrying)?;
                            x.transition(TaskState::Queued)?;
                        }
                    }
                    Ok(())
                });
                if let Ok((s, from)) = res {
                    self.persist.send(PersistOp::State(Box::new(s.clone())));
                    self.publish_state(&s, from);
                }
            } else if t.state == TaskState::Seeding && self.tasks.run(&t.id).is_none() {
                let res = self.mutate_from(&t.id, |x| {
                    x.transition(TaskState::Resolving)?;
                    x.transition(TaskState::Queued)?;
                    Ok(())
                });
                if let Ok((s, from)) = res {
                    self.persist.send(PersistOp::State(Box::new(s.clone())));
                    self.publish_state(&s, from);
                }
            }
        }
        self.scheduler_tick_now().await;
        self.admission.notify_one();
    }

    fn spawn_tickers(&self) {
        let mut handles = Vec::new();
        // admission loop: every second or on demand
        handles.push(spawn_loop(
            self.this.clone(),
            Duration::from_secs(1),
            true,
            |e| async move {
                e.admit_once().await;
            },
        ));
        // progress 250 ms + stats every 1 s
        handles.push(spawn_loop(
            self.this.clone(),
            PROGRESS_INTERVAL,
            false,
            |e| async move {
                e.progress_tick();
            },
        ));
        handles.push(spawn_loop(
            self.this.clone(),
            Duration::from_secs(1),
            false,
            |e| async move {
                e.stats_tick().await;
            },
        ));
        handles.push(spawn_loop(
            self.this.clone(),
            Duration::from_secs(60),
            false,
            |e| async move {
                e.scheduler_tick_now().await;
            },
        ));
        handles.push(spawn_loop(
            self.this.clone(),
            Duration::from_secs(30),
            false,
            |e| async move {
                e.disk_tick().await;
            },
        ));
        handles.push(spawn_loop(
            self.this.clone(),
            Duration::from_secs(2),
            false,
            |e| async move {
                e.flush_log_batch();
            },
        ));
        handles.push(spawn_loop(
            self.this.clone(),
            Duration::from_secs(5),
            false,
            |e| async move {
                e.health_tick();
            },
        ));
        *self.tickers.lock() = handles;
    }

    /// Sample counters of every running task (250 ms).
    fn progress_tick(&self) {
        for (id, run) in self.tasks.running() {
            let Some(cell) = self.tasks.get(&id) else {
                continue;
            };
            let downloaded = run.control.counters.downloaded();
            let uploaded = run.control.counters.uploaded();
            let prev_d = run.last_downloaded.swap(downloaded, Ordering::Relaxed);
            let prev_u = run.last_uploaded.swap(uploaded, Ordering::Relaxed);
            let (speed, instant, up_speed) = {
                let mut m = run.meter.lock();
                if downloaded > prev_d {
                    m.record(downloaded - prev_d);
                }
                let speed = m.tick();
                let instant = m.instant();
                let mut um = run.upload_meter.lock();
                if uploaded > prev_u {
                    um.record(uploaded - prev_u);
                }
                (speed, instant, um.tick())
            };
            let update = {
                let mut t = cell.lock();
                if !matches!(
                    t.state,
                    TaskState::Downloading
                        | TaskState::Seeding
                        | TaskState::Connecting
                        | TaskState::Resolving
                ) {
                    continue;
                }
                if downloaded > 0 && downloaded >= t.progress.downloaded {
                    t.progress.downloaded = downloaded;
                }
                if uploaded > t.progress.uploaded {
                    t.progress.uploaded = uploaded;
                }
                t.progress.speed = speed;
                t.progress.instant_speed = instant;
                t.progress.upload_speed = up_speed;
                let conns = run
                    .control
                    .counters
                    .active_connections
                    .load(Ordering::Relaxed) as u32;
                if conns > 0 {
                    t.progress.active_connections = conns;
                }
                t.progress.eta_seconds = t.progress.total.and_then(|total| {
                    let remaining = total.saturating_sub(t.progress.downloaded);
                    run.meter.lock().eta(remaining)
                });
                if let Some(total) = t.progress.total.filter(|x| *x > 0) {
                    t.progress.fraction = (t.progress.downloaded as f32 / total as f32).min(1.0);
                }
                ProgressUpdate {
                    task_id: id.clone(),
                    progress: t.progress.clone(),
                    rev: t.rev,
                }
            };
            self.progress_writer.update(&id, &update.progress);
            self.bus.progress(update);
        }
    }

    /// Aggregate stats (1 s), speed sample (10 s).
    async fn stats_tick(&self) {
        let tasks = self.tasks.all();
        let settings = self.settings();
        let mut s = GlobalStats {
            at: Millis::now(),
            network_available: self.environment_snapshot().network_available,
            traffic_mode: settings.bandwidth.mode,
            total_tasks: tasks.len() as u32,
            ..Default::default()
        };
        let (dl, ul) = settings.bandwidth.effective_limits();
        s.download_limit = dl;
        s.upload_limit = ul;
        for t in &tasks {
            match t.state {
                TaskState::Downloading => {
                    s.downloading += 1;
                    s.active += 1;
                }
                TaskState::Seeding => {
                    s.seeding += 1;
                    s.active += 1;
                }
                st if st.is_active() => s.active += 1,
                TaskState::Queued => s.queued += 1,
                TaskState::Scheduled => s.scheduled += 1,
                TaskState::Paused => s.paused += 1,
                _ => {}
            }
            if t.state.is_transferring() || t.state == TaskState::Connecting {
                s.download_speed += t.progress.speed;
                s.upload_speed += t.progress.upload_speed;
            }
        }
        let prev = self.stats.read().clone();
        let tick = self.started_at.elapsed().as_secs();
        if tick.is_multiple_of(10) || prev.at.0 == 0 {
            let (c, f, b) = self.today_counters().await;
            s.completed_today = c;
            s.failed_today = f;
            s.bytes_today = b;
            let dir = settings.storage.download_directory.clone();
            s.free_space =
                tokio::task::spawn_blocking(move || swoop_runtime::disk::free_space(&dir))
                    .await
                    .unwrap_or(None);
            if let Err(e) = self
                .store
                .append_speed_sample(s.at, s.download_speed, s.upload_speed)
                .await
            {
                tracing::debug!(error = %e, "speed sample not recorded");
            }
        } else {
            s.completed_today = prev.completed_today;
            s.failed_today = prev.failed_today;
            s.bytes_today = prev.bytes_today;
            s.free_space = prev.free_space;
        }
        *self.stats.write() = s.clone();
        {
            let mut env = self.environment.write();
            env.download_speed = s.download_speed;
            env.active_transfers = s.active;
        }
        self.bus.publish(Event::GlobalStats(s));
        self.bus
            .publish(Event::QueueSummaries(self.queue_summaries_now()));
    }

    /// Current settings snapshot.
    pub fn settings(&self) -> Arc<Settings> {
        self.settings_cell.read().clone()
    }

    pub(crate) fn environment_snapshot(&self) -> EnvironmentSnapshot {
        self.environment.read().clone()
    }

    pub(crate) fn global_stats_snapshot(&self) -> GlobalStats {
        self.stats.read().clone()
    }

    /// Whether the main window is frontmost (quiet notifications). FFI-only.
    pub fn set_active_window(&self, active: bool) {
        self.active_window.store(active, Ordering::Relaxed);
    }

    /// The store, for tests and CLI parity checks.
    pub fn store(&self) -> Arc<Store> {
        self.store.clone()
    }

    async fn update_environment_inner(&self, env: EnvironmentSnapshot) {
        let previous = {
            let mut cur = self.environment.write();
            let prev = cur.clone();
            let mut next = env;
            next.download_speed = prev.download_speed;
            next.active_transfers = prev.active_transfers;
            if next.at.0 == 0 {
                next.at = Millis::now();
            }
            *cur = next;
            prev
        };
        let now = self.environment_snapshot();
        if previous.network_available != now.network_available || previous.metered != now.metered {
            self.bus.publish(Event::NetworkChanged {
                available: now.network_available,
                metered: now.metered,
            });
        }
        if previous.network_available && !now.network_available {
            for t in self.tasks.all() {
                if t.state.is_active() || t.state == TaskState::Queued {
                    self.block_task(&t.id, PauseReason::NetworkUnavailable)
                        .await;
                }
            }
        } else if !previous.network_available && now.network_available {
            for t in self.tasks.all() {
                if t.blocked_by.contains(&PauseReason::NetworkUnavailable) {
                    self.unblock_task(&t.id, &PauseReason::NetworkUnavailable)
                        .await;
                }
            }
        }
        self.scheduler_tick_now().await;
    }

    async fn shutdown_inner(&self) {
        if self.shutting_down.swap(true, Ordering::Relaxed) {
            return;
        }
        self.bus.publish(Event::EngineStopping);
        for h in self.tickers.lock().drain(..) {
            h.abort();
        }
        let start = Instant::now();
        let mut runs = Vec::new();
        for t in self.tasks.all() {
            if self.tasks.run(&t.id).is_some() {
                if let Some(run) = self.tasks.run(&t.id) {
                    runs.push(run);
                }
                self.block_task(&t.id, PauseReason::Shutdown).await;
            }
        }
        for run in runs {
            let remaining = SHUTDOWN_WAIT.saturating_sub(start.elapsed());
            run.wait_finished(remaining.max(Duration::from_millis(100)))
                .await;
        }
        for s in self.grabber.list() {
            if let Some(session) = self.grabber.get(&s.id) {
                session.cancel();
            }
        }
        self.flush_log_batch();
        let _ = self.persist.flush().await;
        let _ = self.progress_writer.flush().await;
        let _ = self.store.flush().await;
        self.torrent.shutdown().await;
        if let Err(e) = self.store.close().await {
            tracing::warn!(error = %e, "store close failed");
        }
        tracing::info!("engine stopped");
    }

    fn info_inner(&self) -> EngineInfo {
        let s = self.settings();
        EngineInfo {
            version: self.config.app_version.clone(),
            build: format!("{} {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS),
            os: crate::diagnostics::os_description(),
            arch: std::env::consts::ARCH.to_owned(),
            data_dir: self.paths.data_dir.clone(),
            local_api_port: s.remote.local_port,
            remote_enabled: s.remote.enabled,
            remote_port: s.remote.enabled.then_some(s.remote.port),
            uptime_seconds: self.started_at.elapsed().as_secs(),
            headless: self.config.headless,
            ffmpeg_available: ffmpeg_available(),
        }
    }

    async fn dashboard_inner(&self) -> DomainResult<Dashboard> {
        let mut recent: Vec<Task> = self.tasks.all();
        recent.sort_by_key(|r| std::cmp::Reverse(r.created_at));
        let recent: Vec<TaskRow> = recent.iter().take(20).map(TaskRow::from).collect();
        let since = Millis::now().saturating_add_ms(-3_600_000);
        let speed_history = self
            .store
            .speed_samples(since)
            .await?
            .into_iter()
            .map(|s| SpeedSample {
                at: s.at,
                download: s.download,
                upload: s.upload,
            })
            .collect();
        let disks = vec![self.disk_info_inner(None).await?];
        let scheduled_next = self.scheduler.next_boundary(Millis::now());
        Ok(Dashboard {
            stats: self.global_stats_snapshot(),
            queues: self.queue_summaries_now(),
            recent,
            speed_history,
            disks,
            scheduled_next,
        })
    }

    /// Recent process log lines.
    pub fn recent_logs_now(&self, limit: u32, level: Option<String>) -> Vec<String> {
        let level = level.as_deref().and_then(crate::logs::parse_level);
        self.logs.ring().recent(limit as usize, level)
    }
}

fn default_settings() -> Settings {
    let mut s = Settings::default();
    s.storage.download_directory = AppPaths::default_download_dir();
    s
}

fn ffmpeg_available() -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join("ffmpeg").is_file()))
        .unwrap_or(false)
        || std::path::Path::new("/opt/homebrew/bin/ffmpeg").is_file()
        || std::path::Path::new("/usr/local/bin/ffmpeg").is_file()
}

/// Read the local API token or create it (0600) on first start.
fn load_or_create_local_token(path: &std::path::Path) -> DomainResult<String> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        let t = existing.trim();
        if t.len() >= 32 {
            return Ok(t.to_owned());
        }
    }
    let token = crate::devices::generate_token();
    std::fs::write(path, &token).map_err(|e| DomainError::Storage(format!("local token: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(token)
}

/// Spawn a periodic loop calling `f` with a strong engine reference; stops when the engine
/// is dropped. `on_notify` additionally wakes the loop on `engine.admission`.
fn spawn_loop<F, Fut>(weak: Weak<Engine>, period: Duration, on_notify: bool, f: F) -> JoinHandle<()>
where
    F: Fn(Arc<Engine>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            let Some(engine) = weak.upgrade() else {
                break;
            };
            if engine.shutting_down.load(Ordering::Relaxed) {
                break;
            }
            if on_notify {
                let admission = engine.admission.clone();
                drop(engine);
                let notified = admission.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                tokio::select! {
                    _ = &mut notified => {}
                    _ = tokio::time::sleep(period) => {}
                }
            } else {
                drop(engine);
                tokio::time::sleep(period).await;
            }
            let Some(engine) = weak.upgrade() else {
                break;
            };
            if engine.shutting_down.load(Ordering::Relaxed) {
                break;
            }
            f(engine).await;
        }
    })
}

trait PluginsDir {
    fn resolve_plugins_dir(config: &EngineConfig) -> PathBuf;
}

impl PluginsDir for AppPaths {
    fn resolve_plugins_dir(config: &EngineConfig) -> PathBuf {
        match &config.data_dir {
            Some(d) => AppPaths::expand_home(&d.to_string_lossy()).join("plugins"),
            None => AppPaths::resolve().plugins_dir(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// EngineApi
// ---------------------------------------------------------------------------------------------

#[async_trait]
impl EngineApi for Engine {
    fn info(&self) -> EngineInfo {
        self.info_inner()
    }
    fn subscribe(&self) -> EventSubscription {
        self.bus.subscribe()
    }
    async fn update_environment(&self, env: EnvironmentSnapshot) {
        self.update_environment_inner(env).await
    }
    fn environment(&self) -> EnvironmentSnapshot {
        self.environment_snapshot()
    }
    async fn shutdown(&self) {
        self.shutdown_inner().await
    }

    async fn probe(&self, request: NewTaskRequest) -> DomainResult<ProbeResult> {
        self.probe_inner(request).await
    }
    async fn add_task(&self, request: NewTaskRequest) -> DomainResult<AddTaskResult> {
        self.add_task_inner(request, None).await
    }
    async fn add_tasks(&self, requests: Vec<NewTaskRequest>) -> DomainResult<Vec<AddTaskResult>> {
        let mut out = Vec::with_capacity(requests.len());
        for r in requests {
            out.push(self.add_task_inner(r, None).await?);
        }
        Ok(out)
    }
    async fn resolve_duplicate(&self, id: TaskId, policy: ConflictPolicy) -> DomainResult<Task> {
        self.resolve_duplicate_inner(id, policy).await
    }
    async fn get_task(&self, id: TaskId) -> DomainResult<Task> {
        self.snapshot(&id)
    }
    async fn list_tasks(&self, filter: TaskFilter) -> DomainResult<TaskPage> {
        let all = self.filtered_tasks(&filter);
        let total = all.len() as u32;
        Ok(TaskPage {
            tasks: Self::page(all, filter.limit, filter.offset),
            total,
        })
    }
    async fn list_rows(&self, filter: TaskFilter) -> DomainResult<Vec<TaskRow>> {
        Ok(self.rows(&filter))
    }
    async fn count_tasks(&self, filter: TaskFilter) -> DomainResult<u32> {
        Ok(self.filtered_tasks(&filter).len() as u32)
    }
    async fn start_task(&self, id: TaskId) -> DomainResult<Task> {
        self.start_task_inner(id).await
    }
    async fn pause_task(&self, id: TaskId) -> DomainResult<Task> {
        self.pause_task_inner(id).await
    }
    async fn resume_task(&self, id: TaskId) -> DomainResult<Task> {
        self.resume_task_inner(id).await
    }
    async fn restart_task(&self, id: TaskId) -> DomainResult<Task> {
        self.restart_task_inner(id).await
    }
    async fn retry_task(&self, id: TaskId) -> DomainResult<Task> {
        self.retry_task_inner(id).await
    }
    async fn retry_from_source(&self, id: TaskId, new_url: Option<String>) -> DomainResult<Task> {
        self.retry_from_source_inner(id, new_url).await
    }
    async fn redownload(&self, id: TaskId) -> DomainResult<Task> {
        self.redownload_inner(id).await
    }
    async fn cancel_task(&self, id: TaskId) -> DomainResult<Task> {
        self.cancel_task_inner(id).await
    }
    async fn remove_task(&self, id: TaskId, delete_file: bool) -> DomainResult<()> {
        self.remove_task_inner(id, delete_file).await
    }
    async fn remove_tasks(&self, ids: Vec<TaskId>, delete_file: bool) -> DomainResult<u32> {
        let mut n = 0;
        for id in ids {
            if self.remove_task_inner(id, delete_file).await.is_ok() {
                n += 1;
            }
        }
        Ok(n)
    }
    async fn update_task(&self, id: TaskId, patch: TaskPatch) -> DomainResult<Task> {
        self.update_task_inner(id, patch).await
    }
    async fn duplicate_task(&self, id: TaskId) -> DomainResult<Task> {
        self.duplicate_task_inner(id).await
    }
    async fn set_task_limit(
        &self,
        id: TaskId,
        download: Option<u64>,
        upload: Option<u64>,
    ) -> DomainResult<Task> {
        self.set_task_limit_inner(id, download, upload).await
    }
    async fn set_task_connections(&self, id: TaskId, connections: u8) -> DomainResult<Task> {
        self.set_task_connections_inner(id, connections).await
    }
    async fn set_task_priority(&self, id: TaskId, priority: Priority) -> DomainResult<Task> {
        self.set_task_priority_inner(id, priority).await
    }
    async fn reorder_tasks(&self, ids: Vec<TaskId>, after: Option<TaskId>) -> DomainResult<()> {
        self.reorder_tasks_inner(ids, after).await
    }
    async fn retry_failed_segments(&self, id: TaskId) -> DomainResult<Task> {
        self.retry_failed_segments_inner(id).await
    }
    async fn verify_task(&self, id: TaskId, checksum: Option<Checksum>) -> DomainResult<Task> {
        self.verify_task_inner(id, checksum).await
    }
    async fn task_log(&self, id: TaskId, limit: u32) -> DomainResult<Vec<TaskLogEntry>> {
        self.task_log_inner(id, limit).await
    }
    async fn diagnostics(&self, id: TaskId) -> DomainResult<TaskDiagnostics> {
        self.diagnostics_inner(id).await
    }
    async fn diagnostics_text(&self, id: TaskId) -> DomainResult<String> {
        self.diagnostics_text_inner(id).await
    }
    async fn pause_all(&self) -> DomainResult<u32> {
        self.pause_all_inner().await
    }
    async fn resume_all(&self) -> DomainResult<u32> {
        self.resume_all_inner().await
    }
    async fn retry_all_failed(&self) -> DomainResult<u32> {
        self.retry_all_failed_inner().await
    }
    async fn clear_completed(&self) -> DomainResult<u32> {
        self.clear_completed_inner().await
    }

    async fn set_torrent_files(
        &self,
        id: TaskId,
        selection: Vec<FileSelection>,
    ) -> DomainResult<Task> {
        self.set_torrent_files_inner(id, selection).await
    }
    async fn set_torrent_sequential(&self, id: TaskId, sequential: bool) -> DomainResult<Task> {
        self.set_torrent_sequential_inner(id, sequential).await
    }
    async fn set_seeding_limits(
        &self,
        id: TaskId,
        limits: swoop_domain::torrent::SeedingLimits,
    ) -> DomainResult<Task> {
        self.set_seeding_limits_inner(id, limits).await
    }
    async fn torrent_peers(&self, id: TaskId) -> DomainResult<Vec<PeerInfo>> {
        self.torrent_peers_inner(id).await
    }
    async fn add_trackers(&self, id: TaskId, trackers: Vec<String>) -> DomainResult<Task> {
        self.add_trackers_inner(id, trackers).await
    }
    async fn remove_tracker(&self, id: TaskId, tracker: String) -> DomainResult<Task> {
        self.remove_tracker_inner(id, tracker).await
    }
    async fn set_tracker_enabled(
        &self,
        id: TaskId,
        tracker: String,
        enabled: bool,
    ) -> DomainResult<Task> {
        self.set_tracker_enabled_inner(id, tracker, enabled).await
    }
    async fn reannounce(&self, id: TaskId) -> DomainResult<()> {
        self.reannounce_inner(id).await
    }
    async fn refresh_tracker_list(&self) -> DomainResult<u32> {
        self.refresh_tracker_list_inner().await
    }

    async fn detect_media(
        &self,
        url: String,
        page_url: Option<String>,
    ) -> DomainResult<DetectedMedia> {
        self.detect_media_inner(url, page_url).await
    }

    async fn list_queues(&self) -> DomainResult<Vec<Queue>> {
        Ok(self.queues.all())
    }
    async fn queue_summaries(&self) -> DomainResult<Vec<QueueSummary>> {
        Ok(self.queue_summaries_now())
    }
    async fn create_queue(&self, queue: Queue) -> DomainResult<Queue> {
        self.create_queue_inner(queue).await
    }
    async fn update_queue(&self, queue: Queue) -> DomainResult<Queue> {
        self.update_queue_inner(queue).await
    }
    async fn delete_queue(&self, id: QueueId, move_tasks_to: Option<QueueId>) -> DomainResult<()> {
        self.delete_queue_inner(id, move_tasks_to).await
    }
    async fn pause_queue(&self, id: QueueId) -> DomainResult<Queue> {
        self.set_queue_paused(id, true).await
    }
    async fn resume_queue(&self, id: QueueId) -> DomainResult<Queue> {
        self.set_queue_paused(id, false).await
    }
    async fn reorder_queues(&self, ids: Vec<QueueId>) -> DomainResult<()> {
        self.reorder_queues_inner(ids).await
    }

    async fn list_categories(&self) -> DomainResult<Vec<Category>> {
        let mut c = self.categories.read().clone();
        c.sort_by_key(|x| (x.position, x.created_at));
        Ok(c)
    }
    async fn create_category(&self, c: Category) -> DomainResult<Category> {
        self.create_category_inner(c).await
    }
    async fn update_category(&self, c: Category) -> DomainResult<Category> {
        self.update_category_inner(c).await
    }
    async fn delete_category(&self, id: CategoryId) -> DomainResult<()> {
        self.delete_category_inner(id).await
    }

    async fn list_rules(&self) -> DomainResult<Vec<Rule>> {
        Ok(crate::rules::ordered(&self.rules.read())
            .into_iter()
            .chain(self.rules.read().iter().filter(|r| !r.enabled).cloned())
            .collect())
    }
    async fn create_rule(&self, r: Rule) -> DomainResult<Rule> {
        self.create_rule_inner(r).await
    }
    async fn update_rule(&self, r: Rule) -> DomainResult<Rule> {
        self.update_rule_inner(r).await
    }
    async fn delete_rule(&self, id: RuleId) -> DomainResult<()> {
        self.delete_rule_inner(id).await
    }
    async fn test_rules(&self, subject: RuleSubject) -> DomainResult<Vec<(Rule, Vec<RuleAction>)>> {
        Ok(crate::rules::evaluate(&self.rules.read(), &subject))
    }

    async fn list_schedules(&self) -> DomainResult<Vec<Schedule>> {
        Ok(self.scheduler.all())
    }
    async fn create_schedule(&self, s: Schedule) -> DomainResult<Schedule> {
        self.create_schedule_inner(s).await
    }
    async fn update_schedule(&self, s: Schedule) -> DomainResult<Schedule> {
        self.update_schedule_inner(s).await
    }
    async fn delete_schedule(&self, id: ScheduleId) -> DomainResult<()> {
        self.delete_schedule_inner(id).await
    }

    async fn list_automations(&self) -> DomainResult<Vec<AutomationRule>> {
        Ok(self.automation.all())
    }
    async fn create_automation(&self, a: AutomationRule) -> DomainResult<AutomationRule> {
        self.create_automation_inner(a).await
    }
    async fn update_automation(&self, a: AutomationRule) -> DomainResult<AutomationRule> {
        self.update_automation_inner(a).await
    }
    async fn delete_automation(&self, id: AutomationId) -> DomainResult<()> {
        self.delete_automation_inner(id).await
    }
    async fn grant_automation_consent(&self, id: AutomationId) -> DomainResult<AutomationRule> {
        self.grant_consent_inner(id).await
    }
    async fn automation_runs(
        &self,
        id: Option<AutomationId>,
        limit: u32,
    ) -> DomainResult<Vec<AutomationRun>> {
        self.automation_runs_inner(id, limit).await
    }
    async fn run_automation(
        &self,
        id: AutomationId,
        task_id: TaskId,
    ) -> DomainResult<AutomationRun> {
        self.run_automation_inner(id, task_id).await
    }

    async fn list_recipes(&self) -> DomainResult<Vec<Recipe>> {
        self.list_recipes_inner().await
    }
    async fn save_recipe(&self, r: Recipe) -> DomainResult<Recipe> {
        self.save_recipe_inner(r).await
    }
    async fn delete_recipe(&self, id: RecipeId) -> DomainResult<()> {
        self.delete_recipe_inner(id).await
    }
    async fn apply_recipe(
        &self,
        id: RecipeId,
        request: NewTaskRequest,
    ) -> DomainResult<AddTaskResult> {
        self.apply_recipe_inner(id, request).await
    }

    async fn history(&self, query: HistoryQuery) -> DomainResult<Vec<HistoryEntry>> {
        self.history_inner(query).await
    }
    async fn history_count(&self, query: HistoryQuery) -> DomainResult<u32> {
        self.history_count_inner(query).await
    }
    async fn delete_history(&self, ids: Vec<TaskId>) -> DomainResult<u32> {
        self.delete_history_inner(ids).await
    }
    async fn clear_history(&self) -> DomainResult<u32> {
        self.clear_history_inner().await
    }

    async fn set_traffic_mode(&self, mode: TrafficMode) -> DomainResult<()> {
        self.set_traffic_mode_inner(mode).await
    }
    async fn set_global_limits(&self, download: u64, upload: u64) -> DomainResult<()> {
        self.set_global_limits_inner(download, upload).await
    }
    async fn optimize(&self) -> DomainResult<Settings> {
        self.optimize_inner().await.map(|s| (*s).clone())
    }

    fn settings(&self) -> Arc<Settings> {
        Engine::settings(self)
    }
    async fn update_settings(&self, settings: Settings) -> DomainResult<Settings> {
        self.update_settings_inner(settings)
            .await
            .map(|s| (*s).clone())
    }
    async fn store_credential(
        &self,
        name: String,
        username: Option<String>,
        secret: String,
    ) -> DomainResult<CredentialId> {
        self.store_credential_inner(name, username, secret).await
    }
    async fn list_credentials(&self) -> DomainResult<Vec<(CredentialId, String, Option<String>)>> {
        self.list_credentials_inner().await
    }
    async fn delete_credential(&self, id: CredentialId) -> DomainResult<()> {
        self.delete_credential_inner(id).await
    }

    fn global_stats(&self) -> GlobalStats {
        self.global_stats_snapshot()
    }
    async fn dashboard(&self) -> DomainResult<Dashboard> {
        self.dashboard_inner().await
    }
    async fn disk_info(&self, path: Option<PathBuf>) -> DomainResult<DiskInfo> {
        self.disk_info_inner(path.map(|p| Self::expand(&p))).await
    }

    async fn list_devices(&self) -> DomainResult<Vec<Device>> {
        self.list_devices_inner().await
    }
    async fn start_pairing(&self, scopes: Vec<Scope>) -> DomainResult<PairingInfo> {
        self.start_pairing_inner(scopes).await
    }
    async fn cancel_pairing(&self) -> DomainResult<()> {
        self.cancel_pairing_inner().await
    }
    async fn complete_pairing(
        &self,
        code: String,
        device_name: String,
        device_kind: String,
        ip: String,
    ) -> DomainResult<(Device, String)> {
        self.complete_pairing_inner(code, device_name, device_kind, ip)
            .await
    }
    async fn revoke_device(&self, id: DeviceId) -> DomainResult<()> {
        self.revoke_device_inner(id).await
    }
    async fn rename_device(&self, id: DeviceId, name: String) -> DomainResult<Device> {
        self.rename_device_inner(id, name).await
    }
    async fn authenticate(&self, token: &str, ip: &str) -> DomainResult<Device> {
        self.authenticate_inner(token, ip).await
    }
    async fn audit_log(&self, limit: u32) -> DomainResult<Vec<AuditEntry>> {
        self.audit_log_inner(limit).await
    }
    async fn record_audit(&self, entry: AuditEntry) -> DomainResult<()> {
        self.record_audit_inner(entry).await
    }
    fn local_token(&self) -> String {
        self.local_token.clone()
    }

    async fn grabber_start(&self, options: GrabberOptions) -> DomainResult<GrabberSession> {
        self.grabber_start_inner(options).await
    }
    async fn grabber_status(&self, id: String) -> DomainResult<GrabberSession> {
        self.grabber_status_inner(id).await
    }
    async fn grabber_cancel(&self, id: String) -> DomainResult<()> {
        self.grabber_cancel_inner(id).await
    }
    async fn grabber_add(
        &self,
        id: String,
        urls: Vec<String>,
        request: NewTaskRequest,
    ) -> DomainResult<Vec<AddTaskResult>> {
        self.grabber_add_inner(id, urls, request).await
    }
    async fn grabber_list(&self) -> DomainResult<Vec<GrabberSession>> {
        self.grabber_list_inner().await
    }

    async fn archive_list(&self, path: PathBuf) -> DomainResult<ArchiveListing> {
        self.archive_list_inner(path).await
    }
    async fn archive_extract(
        &self,
        path: PathBuf,
        entries: Option<Vec<String>>,
        destination: PathBuf,
    ) -> DomainResult<u32> {
        self.archive_extract_inner(path, entries, destination).await
    }

    async fn export(
        &self,
        include_tasks: bool,
        include_history: bool,
    ) -> DomainResult<ExportBundle> {
        self.export_inner(include_tasks, include_history).await
    }
    async fn import(
        &self,
        bundle: ExportBundle,
        options: ImportOptions,
    ) -> DomainResult<ImportReport> {
        self.import_inner(bundle, options).await
    }

    async fn check_for_updates(&self) -> DomainResult<UpdateInfo> {
        self.check_for_updates_inner().await
    }
    async fn download_update(&self) -> DomainResult<PathBuf> {
        self.download_update_inner().await
    }

    async fn list_plugins(&self) -> DomainResult<Vec<PluginInfo>> {
        self.list_plugins_inner().await
    }
    async fn set_plugin_enabled(
        &self,
        id: PluginId,
        enabled: bool,
        granted_permissions: Vec<String>,
    ) -> DomainResult<PluginInfo> {
        self.set_plugin_enabled_inner(id, enabled, granted_permissions)
            .await
    }
    async fn uninstall_plugin(&self, id: PluginId) -> DomainResult<()> {
        self.uninstall_plugin_inner(id).await
    }

    async fn recent_logs(&self, limit: u32, level: Option<String>) -> DomainResult<Vec<String>> {
        Ok(self.recent_logs_now(limit, level))
    }
}
