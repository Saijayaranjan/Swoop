//! `SwoopEngine`: the UniFFI object the macOS app holds for its whole lifetime.
//!
//! It owns a dedicated multi-thread Tokio runtime (`swoop-rt`, 4 workers). UniFFI polls exported
//! futures on the foreign (Swift) executor, so every async method hands its work to that runtime
//! with `rt.spawn(..).await` (see [`ffi_async!`]); panics surface as `FfiError::Internal`.

use crate::error::{panic_message, FfiError, FfiResult};
use crate::events::{self, EventListener, FfiEvent};
use crate::server::ServerSupervisor;
use crate::types::*;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use swoop_domain as d;
use swoop_domain::automation::AutomationRule;
use swoop_domain::rules::{Rule, RuleSubject};
use swoop_domain::schedule::{Recurrence, Schedule};
use swoop_domain::settings::Settings;
use swoop_domain::Millis;
use swoop_runtime::paths::AppPaths;
use swoop_services::{self as s, Engine, SharedEngine};
use tokio::runtime::{Handle, Runtime};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

/// Number of runtime worker threads.
const WORKERS: usize = 4;
/// Log lines included in `task_detail`.
const LOG_TAIL: u32 = 50;

/// Run `$body` (an expression producing `FfiResult<T>`, `?` allowed) on the engine runtime with
/// `$api: SharedEngine` in scope.
macro_rules! ffi_async {
    ($self:ident, |$api:ident| $body:expr) => {
        $self
            .run(move |$api: SharedEngine| async move { $body })
            .await
    };
}

#[derive(uniffi::Object)]
pub struct SwoopEngine {
    /// `None` after drop started; never dropped inside async context (see `Drop`).
    rt: Mutex<Option<Runtime>>,
    handle: Handle,
    api: SharedEngine,
    engine: Arc<Engine>,
    config: FfiEngineConfig,
    data_dir: PathBuf,
    listeners: Mutex<HashMap<u64, JoinHandle<()>>>,
    next_listener: AtomicU64,
    /// FFI-originated events (e.g. `RecipesChanged`) merged into every forwarder.
    local_tx: broadcast::Sender<FfiEvent>,
    server: Arc<ServerSupervisor>,
    watcher: Mutex<Option<JoinHandle<()>>>,
    snapshot_gen: AtomicU64,
    stopped: AtomicBool,
}

impl SwoopEngine {
    fn open_inner(config: FfiEngineConfig) -> FfiResult<Arc<Self>> {
        let data_dir = match config.data_dir.as_deref().filter(|d| !d.trim().is_empty()) {
            Some(d) => AppPaths::expand_home(d),
            None => AppPaths::resolve().data_dir,
        };
        // Logging first so engine start-up is captured (idempotent per process).
        let level = if config.log_level.trim().is_empty() {
            "info"
        } else {
            config.log_level.as_str()
        };
        swoop_services::logs::init_logging(&data_dir.join("logs"), level, 14);

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(WORKERS)
            .thread_name("swoop-rt")
            .enable_all()
            .build()
            .map_err(|e| FfiError::internal(format!("tokio runtime: {e}")))?;
        let handle = rt.handle().clone();

        let engine_config = s::EngineConfig {
            data_dir: Some(data_dir.clone()),
            headless: config.headless,
            app_version: config.app_version.clone(),
            skip_instance_lock: config.skip_instance_lock,
            download_dir: config
                .download_dir
                .as_deref()
                .filter(|d| !d.trim().is_empty())
                .map(PathBuf::from),
        };
        let engine = block_on_spawn(&rt, Engine::start(engine_config))??;
        // Honour an explicit level from the app over the stored setting.
        if !config.log_level.trim().is_empty() {
            swoop_services::logs::set_level(&config.log_level);
        }
        let api: SharedEngine = engine.clone();
        let server = Arc::new(ServerSupervisor::new(api.clone(), data_dir.clone()));
        if config.start_local_api {
            let sv = server.clone();
            let settings = api.settings();
            if let Err(e) = block_on_spawn(&rt, async move {
                sv.start_local().await;
                sv.apply_remote(&settings).await;
            }) {
                tracing::warn!(error = %e, "local API start-up failed");
            }
        }
        let (local_tx, _) = broadcast::channel(256);
        let this = Arc::new(Self {
            rt: Mutex::new(Some(rt)),
            handle,
            api,
            engine,
            config,
            data_dir,
            listeners: Mutex::new(HashMap::new()),
            next_listener: AtomicU64::new(1),
            local_tx,
            server,
            watcher: Mutex::new(None),
            snapshot_gen: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
        });
        if this.config.start_local_api {
            *this.watcher.lock() = Some(this.spawn_settings_watcher());
        }
        tracing::info!(data_dir = %this.data_dir.display(), "swoop engine opened via FFI");
        Ok(this)
    }

    /// Start/stop the remote listener when the remote settings change.
    fn spawn_settings_watcher(&self) -> JoinHandle<()> {
        let mut rx = self.api.subscribe();
        let server = self.server.clone();
        let api = self.api.clone();
        self.handle.spawn(async move {
            use tokio::sync::broadcast::error::RecvError;
            loop {
                match rx.recv().await {
                    Ok(ev) => match &*ev {
                        d::Event::SettingsChanged(settings) => server.apply_remote(settings).await,
                        d::Event::EngineStopping => break,
                        _ => {}
                    },
                    Err(RecvError::Lagged(_)) => server.apply_remote(&api.settings()).await,
                    Err(RecvError::Closed) => break,
                }
            }
        })
    }

    /// Run `f(api)` on the engine runtime and await it from the foreign executor.
    async fn run<T, F, Fut>(&self, f: F) -> FfiResult<T>
    where
        F: FnOnce(SharedEngine) -> Fut,
        Fut: Future<Output = FfiResult<T>> + Send + 'static,
        T: Send + 'static,
    {
        let fut = f(self.api.clone());
        join(self.handle.spawn(fut).await)
    }

    fn emit_local(&self, e: FfiEvent) {
        let _ = self.local_tx.send(e);
    }
}

/// Map a join result: panics → `Internal`, cancellation → `Unavailable`.
fn join<T>(r: Result<FfiResult<T>, tokio::task::JoinError>) -> FfiResult<T> {
    match r {
        Ok(v) => v,
        Err(e) if e.is_panic() => Err(FfiError::internal(panic_message(&*e.into_panic()))),
        Err(_) => Err(FfiError::unavailable("the engine is shutting down")),
    }
}

/// Block the calling (non-runtime) thread on a future spawned onto `rt`.
fn block_on_spawn<T: Send + 'static>(
    rt: &Runtime,
    fut: impl Future<Output = T> + Send + 'static,
) -> FfiResult<T> {
    let h = rt.spawn(fut);
    join(rt.block_on(async { h.await.map(Ok) }))
}

/// Catch panics of synchronous entry points.
fn guard<T>(f: impl FnOnce() -> FfiResult<T>) -> FfiResult<T> {
    std::panic::catch_unwind(AssertUnwindSafe(f))
        .unwrap_or_else(|p| Err(FfiError::internal(panic_message(&*p))))
}

fn row(t: &d::Task) -> FfiTaskRow {
    t.into()
}

fn now() -> Value {
    Value::from(Millis::now().0)
}

/// Overlay the user's JSON object on a serialised template so optional-but-required fields
/// (id, timestamps, empty lists) need not be sent from Swift. Empty/absent `id` keeps the
/// template's fresh id; `updated_at` is always refreshed.
fn overlay<T: Serialize>(template: &T, json: &str) -> FfiResult<Value> {
    let mut base = serde_json::to_value(template)?;
    let user: Value = serde_json::from_str(json)?;
    let Value::Object(user) = user else {
        return Err(FfiError::validation("expected a JSON object"));
    };
    let obj = base
        .as_object_mut()
        .ok_or_else(|| FfiError::internal("template is not an object"))?;
    for (k, v) in user {
        let keep_template = k == "id" && (v.is_null() || v.as_str().is_some_and(str::is_empty));
        if !keep_template {
            obj.insert(k, v);
        }
    }
    obj.insert("updated_at".into(), now());
    Ok(base)
}

/// Recursive JSON merge (objects merge, everything else replaces).
fn merge(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                match b.get_mut(&k) {
                    Some(slot) => merge(slot, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (slot, v) => *slot = v,
    }
}

fn to_json<T: Serialize>(v: &T) -> FfiResult<String> {
    serde_json::to_string(v).map_err(|e| FfiError::internal(format!("serialise: {e}")))
}

fn empty_recipe() -> s::Recipe {
    let now = Millis::now();
    s::Recipe {
        id: d::RecipeId::new(),
        name: String::new(),
        icon: "wand.and.stars".into(),
        queue_id: None,
        category_id: None,
        directory: None,
        options: d::TaskOptions::default(),
        tags: Vec::new(),
        rule_actions: Vec::new(),
        automation_id: None,
        created_at: now,
        updated_at: now,
    }
}

#[uniffi::export]
impl SwoopEngine {
    /// Open the engine: logging, runtime, store, recovery, and (unless disabled) the local
    /// Unix-socket API for the CLI and the browser native host. Blocks until ready.
    #[uniffi::constructor]
    pub fn open(config: FfiEngineConfig) -> Result<Arc<Self>, FfiError> {
        guard(|| Self::open_inner(config))
    }

    pub fn info(&self) -> FfiEngineInfo {
        let i = self.api.info();
        let srv = self.server.info();
        FfiEngineInfo {
            version: i.version,
            build: i.build,
            os: i.os,
            arch: i.arch,
            bundle_id: self.config.bundle_id.clone(),
            data_dir: path_str(&i.data_dir),
            log_dir: path_str(&self.data_dir.join("logs")),
            local_api_port: i.local_api_port,
            remote_enabled: i.remote_enabled,
            remote_port: i.remote_port,
            uptime_seconds: i.uptime_seconds,
            headless: i.headless,
            ffmpeg_available: i.ffmpeg_available,
            socket_path: srv.socket_path,
            local_address: srv.local_address,
            remote_address: srv.remote_address,
            tls_fingerprint: srv.tls_fingerprint,
        }
    }

    // ----- events -----

    /// Register a listener; events arrive in batches (≤ 100 ms) on a background thread.
    pub fn add_listener(&self, listener: Arc<dyn EventListener>) -> u64 {
        let id = self.next_listener.fetch_add(1, Ordering::Relaxed);
        let bus = self.api.subscribe();
        let local = self.local_tx.subscribe();
        let task = self.handle.spawn(events::forward(listener, bus, local));
        self.listeners.lock().insert(id, task);
        id
    }

    pub fn remove_listener(&self, id: u64) {
        if let Some(t) = self.listeners.lock().remove(&id) {
            t.abort();
        }
    }

    // ----- snapshot & tasks -----

    pub async fn snapshot(&self) -> Result<FfiSnapshot, FfiError> {
        let rev = self.snapshot_gen.fetch_add(1, Ordering::Relaxed) + 1;
        ffi_async!(self, |api| {
            let page = api.list_tasks(s::TaskFilter::default()).await?;
            let queues = api.list_queues().await?;
            let summaries = api.queue_summaries().await?;
            let categories = api.list_categories().await?;
            Ok(FfiSnapshot {
                rev,
                rows: page.tasks.iter().map(row).collect(),
                queues: queues.iter().map(Into::into).collect(),
                queue_summaries: summaries.iter().map(Into::into).collect(),
                categories: categories.iter().map(Into::into).collect(),
                stats: (&api.global_stats()).into(),
            })
        })
    }

    pub async fn task_rows(&self, filter: FfiTaskFilter) -> Result<Vec<FfiTaskRow>, FfiError> {
        ffi_async!(self, |api| {
            let page = api.list_tasks(filter.into()).await?;
            Ok(page.tasks.iter().map(row).collect())
        })
    }

    pub async fn count_tasks(&self, filter: FfiTaskFilter) -> Result<u32, FfiError> {
        ffi_async!(self, |api| Ok(api.count_tasks(filter.into()).await?))
    }

    pub async fn task_detail(&self, id: String) -> Result<FfiTaskDetail, FfiError> {
        ffi_async!(self, |api| {
            let id = d::TaskId(id);
            let task = api.get_task(id.clone()).await?;
            let log = api.task_log(id, LOG_TAIL).await.unwrap_or_default();
            Ok(FfiTaskDetail::build(&task, &log))
        })
    }

    pub async fn probe(&self, request: FfiNewTaskRequest) -> Result<FfiProbeResult, FfiError> {
        ffi_async!(self, |api| {
            let p = api.probe(request.try_into()?).await?;
            Ok((&p).into())
        })
    }

    pub async fn add_task(&self, request: FfiNewTaskRequest) -> Result<FfiAddTaskResult, FfiError> {
        ffi_async!(self, |api| {
            let r = api.add_task(request.try_into()?).await?;
            Ok((&r).into())
        })
    }

    pub async fn add_tasks(
        &self,
        requests: Vec<FfiNewTaskRequest>,
    ) -> Result<Vec<FfiAddTaskResult>, FfiError> {
        ffi_async!(self, |api| {
            let reqs = requests
                .into_iter()
                .map(TryInto::try_into)
                .collect::<FfiResult<Vec<d::NewTaskRequest>>>()?;
            let r = api.add_tasks(reqs).await?;
            Ok(r.iter().map(Into::into).collect())
        })
    }

    pub async fn task_action(
        &self,
        id: String,
        action: FfiTaskAction,
    ) -> Result<FfiTaskRow, FfiError> {
        ffi_async!(self, |api| {
            let t = apply_action(&api, d::TaskId(id), action).await?;
            Ok(row(&t))
        })
    }

    /// Apply one action to many tasks; returns how many succeeded (errors only if none did).
    pub async fn tasks_action(
        &self,
        ids: Vec<String>,
        action: FfiTaskAction,
    ) -> Result<u32, FfiError> {
        ffi_async!(self, |api| {
            let mut ok = 0u32;
            let mut first_err = None;
            for id in ids {
                match apply_action(&api, d::TaskId(id), action.clone()).await {
                    Ok(_) => ok += 1,
                    Err(e) => {
                        tracing::debug!(error = %e, "bulk task action failed for one task");
                        first_err.get_or_insert(e);
                    }
                }
            }
            match first_err {
                Some(e) if ok == 0 => Err(e),
                _ => Ok(ok),
            }
        })
    }

    pub async fn remove_tasks(&self, ids: Vec<String>, delete_file: bool) -> Result<u32, FfiError> {
        ffi_async!(self, |api| Ok(api
            .remove_tasks(ids.into_iter().map(d::TaskId).collect(), delete_file)
            .await?))
    }

    pub async fn update_task(
        &self,
        id: String,
        patch: FfiTaskPatch,
    ) -> Result<FfiTaskRow, FfiError> {
        ffi_async!(self, |api| {
            let t = api.update_task(d::TaskId(id), patch.try_into()?).await?;
            Ok(row(&t))
        })
    }

    pub async fn resolve_duplicate(
        &self,
        id: String,
        policy: FfiConflictPolicy,
    ) -> Result<FfiTaskRow, FfiError> {
        ffi_async!(self, |api| {
            let t = api.resolve_duplicate(d::TaskId(id), policy.into()).await?;
            Ok(row(&t))
        })
    }

    /// Move `ids` after `after` (`nil` = top) within their queue.
    pub async fn reorder_tasks(
        &self,
        ids: Vec<String>,
        after: Option<String>,
    ) -> Result<(), FfiError> {
        ffi_async!(self, |api| Ok(api
            .reorder_tasks(
                ids.into_iter().map(d::TaskId).collect(),
                after.map(d::TaskId)
            )
            .await?))
    }

    /// Bytes/s; `nil` inherits, `0` = unlimited.
    pub async fn set_task_limit(
        &self,
        id: String,
        download: Option<u64>,
        upload: Option<u64>,
    ) -> Result<FfiTaskRow, FfiError> {
        ffi_async!(self, |api| Ok(row(&api
            .set_task_limit(d::TaskId(id), download, upload)
            .await?)))
    }

    pub async fn set_task_connections(
        &self,
        id: String,
        connections: u8,
    ) -> Result<FfiTaskRow, FfiError> {
        ffi_async!(self, |api| Ok(row(&api
            .set_task_connections(d::TaskId(id), connections)
            .await?)))
    }

    pub async fn set_task_priority(
        &self,
        id: String,
        priority: FfiPriority,
    ) -> Result<FfiTaskRow, FfiError> {
        ffi_async!(self, |api| Ok(row(&api
            .set_task_priority(d::TaskId(id), priority.into())
            .await?)))
    }

    /// Newest `limit` log lines of a task (0 = all kept).
    pub async fn task_log(&self, id: String, limit: u32) -> Result<Vec<FfiLogEntry>, FfiError> {
        ffi_async!(self, |api| Ok(api
            .task_log(d::TaskId(id), limit)
            .await?
            .iter()
            .map(Into::into)
            .collect()))
    }

    pub async fn diagnostics_text(&self, id: String) -> Result<String, FfiError> {
        ffi_async!(self, |api| Ok(api.diagnostics_text(d::TaskId(id)).await?))
    }

    pub async fn pause_all(&self) -> Result<u32, FfiError> {
        ffi_async!(self, |api| Ok(api.pause_all().await?))
    }

    pub async fn resume_all(&self) -> Result<u32, FfiError> {
        ffi_async!(self, |api| Ok(api.resume_all().await?))
    }

    pub async fn retry_all_failed(&self) -> Result<u32, FfiError> {
        ffi_async!(self, |api| Ok(api.retry_all_failed().await?))
    }

    pub async fn clear_completed(&self) -> Result<u32, FfiError> {
        ffi_async!(self, |api| Ok(api.clear_completed().await?))
    }

    // ----- torrents -----

    pub async fn set_torrent_files(
        &self,
        id: String,
        selection: Vec<FfiFileSelection>,
    ) -> Result<FfiTaskRow, FfiError> {
        ffi_async!(self, |api| Ok(row(&api
            .set_torrent_files(
                d::TaskId(id),
                selection.into_iter().map(Into::into).collect()
            )
            .await?)))
    }

    pub async fn set_torrent_sequential(
        &self,
        id: String,
        sequential: bool,
    ) -> Result<FfiTaskRow, FfiError> {
        ffi_async!(self, |api| Ok(row(&api
            .set_torrent_sequential(d::TaskId(id), sequential)
            .await?)))
    }

    pub async fn set_seeding_limits(
        &self,
        id: String,
        limits: FfiSeedingLimits,
    ) -> Result<FfiTaskRow, FfiError> {
        ffi_async!(self, |api| Ok(row(&api
            .set_seeding_limits(d::TaskId(id), limits.into())
            .await?)))
    }

    pub async fn torrent_peers(&self, id: String) -> Result<Vec<FfiPeer>, FfiError> {
        ffi_async!(self, |api| Ok(api
            .torrent_peers(d::TaskId(id))
            .await?
            .iter()
            .map(Into::into)
            .collect()))
    }

    pub async fn add_trackers(
        &self,
        id: String,
        trackers: Vec<String>,
    ) -> Result<FfiTaskRow, FfiError> {
        ffi_async!(self, |api| Ok(row(&api
            .add_trackers(d::TaskId(id), trackers)
            .await?)))
    }

    pub async fn remove_tracker(
        &self,
        id: String,
        tracker: String,
    ) -> Result<FfiTaskRow, FfiError> {
        ffi_async!(self, |api| Ok(row(&api
            .remove_tracker(d::TaskId(id), tracker)
            .await?)))
    }

    pub async fn set_tracker_enabled(
        &self,
        id: String,
        tracker: String,
        enabled: bool,
    ) -> Result<FfiTaskRow, FfiError> {
        ffi_async!(self, |api| Ok(row(&api
            .set_tracker_enabled(d::TaskId(id), tracker, enabled)
            .await?)))
    }

    pub async fn reannounce(&self, id: String) -> Result<(), FfiError> {
        ffi_async!(self, |api| Ok(api.reannounce(d::TaskId(id)).await?))
    }

    pub async fn refresh_tracker_list(&self) -> Result<u32, FfiError> {
        ffi_async!(self, |api| Ok(api.refresh_tracker_list().await?))
    }

    // ----- media -----

    pub async fn detect_media(
        &self,
        url: String,
        page_url: Option<String>,
    ) -> Result<FfiDetectedMedia, FfiError> {
        ffi_async!(self, |api| Ok(
            (&api.detect_media(url, page_url).await?).into()
        ))
    }

    // ----- queues -----

    pub async fn queues(&self) -> Result<Vec<FfiQueue>, FfiError> {
        ffi_async!(self, |api| Ok(api
            .list_queues()
            .await?
            .iter()
            .map(Into::into)
            .collect()))
    }

    pub async fn queue_summaries(&self) -> Result<Vec<FfiQueueSummary>, FfiError> {
        ffi_async!(self, |api| Ok(api
            .queue_summaries()
            .await?
            .iter()
            .map(Into::into)
            .collect()))
    }

    /// Create (empty or unknown `id`) or update a queue.
    pub async fn save_queue(&self, queue: FfiQueue) -> Result<FfiQueue, FfiError> {
        ffi_async!(self, |api| {
            let exists =
                !queue.id.is_empty() && api.list_queues().await?.iter().any(|q| q.id.0 == queue.id);
            let q: d::queue::Queue = queue.into();
            let saved = if exists {
                api.update_queue(q).await?
            } else {
                api.create_queue(q).await?
            };
            Ok((&saved).into())
        })
    }

    pub async fn delete_queue(&self, id: String, move_to: Option<String>) -> Result<(), FfiError> {
        ffi_async!(self, |api| Ok(api
            .delete_queue(d::QueueId(id), move_to.map(d::QueueId))
            .await?))
    }

    pub async fn pause_queue(&self, id: String) -> Result<FfiQueue, FfiError> {
        ffi_async!(self, |api| Ok(
            (&api.pause_queue(d::QueueId(id)).await?).into()
        ))
    }

    pub async fn resume_queue(&self, id: String) -> Result<FfiQueue, FfiError> {
        ffi_async!(self, |api| Ok(
            (&api.resume_queue(d::QueueId(id)).await?).into()
        ))
    }

    pub async fn reorder_queues(&self, ids: Vec<String>) -> Result<(), FfiError> {
        ffi_async!(self, |api| Ok(api
            .reorder_queues(ids.into_iter().map(d::QueueId).collect())
            .await?))
    }

    // ----- categories -----

    pub async fn categories(&self) -> Result<Vec<FfiCategory>, FfiError> {
        ffi_async!(self, |api| Ok(api
            .list_categories()
            .await?
            .iter()
            .map(Into::into)
            .collect()))
    }

    /// Create (empty or unknown `id`) or update a category.
    pub async fn save_category(&self, category: FfiCategory) -> Result<FfiCategory, FfiError> {
        ffi_async!(self, |api| {
            let exists = !category.id.is_empty()
                && api
                    .list_categories()
                    .await?
                    .iter()
                    .any(|c| c.id.0 == category.id);
            let c: d::category::Category = category.into();
            let saved = if exists {
                api.update_category(c).await?
            } else {
                api.create_category(c).await?
            };
            Ok((&saved).into())
        })
    }

    pub async fn delete_category(&self, id: String) -> Result<(), FfiError> {
        ffi_async!(self, |api| Ok(api
            .delete_category(d::CategoryId(id))
            .await?))
    }

    // ----- rules (JSON: `swoop_domain::rules::Rule`) -----

    pub async fn rules_json(&self) -> Result<String, FfiError> {
        ffi_async!(self, |api| to_json(&api.list_rules().await?))
    }

    /// Create or update a rule; missing fields take defaults. Returns the saved rule JSON.
    pub async fn save_rule_json(&self, json: String) -> Result<String, FfiError> {
        ffi_async!(self, |api| {
            let rule: Rule = serde_json::from_value(overlay(&Rule::new(""), &json)?)?;
            let exists = api.list_rules().await?.iter().any(|r| r.id == rule.id);
            let saved = if exists {
                api.update_rule(rule).await?
            } else {
                api.create_rule(rule).await?
            };
            to_json(&saved)
        })
    }

    pub async fn delete_rule(&self, id: String) -> Result<(), FfiError> {
        ffi_async!(self, |api| Ok(api.delete_rule(d::RuleId(id)).await?))
    }

    /// Dry run. `subject_json` = `{name, url, domain, mime?, size?, origin, kind}`; returns
    /// `[{"rule": Rule, "actions": [RuleAction]}]`.
    pub async fn test_rules_json(&self, subject_json: String) -> Result<String, FfiError> {
        ffi_async!(self, |api| {
            let mut base = serde_json::to_value(RuleSubject::default())?;
            merge(&mut base, serde_json::from_str(&subject_json)?);
            let subject: RuleSubject = serde_json::from_value(base)?;
            let out: Vec<Value> = api
                .test_rules(subject)
                .await?
                .into_iter()
                .map(|(rule, actions)| serde_json::json!({ "rule": rule, "actions": actions }))
                .collect();
            to_json(&out)
        })
    }

    // ----- schedules (JSON: `swoop_domain::schedule::Schedule`) -----

    pub async fn schedules_json(&self) -> Result<String, FfiError> {
        ffi_async!(self, |api| to_json(&api.list_schedules().await?))
    }

    pub async fn save_schedule_json(&self, json: String) -> Result<String, FfiError> {
        ffi_async!(self, |api| {
            let template = Schedule::new("", Recurrence::Always);
            let sched: Schedule = serde_json::from_value(overlay(&template, &json)?)?;
            let exists = api.list_schedules().await?.iter().any(|x| x.id == sched.id);
            let saved = if exists {
                api.update_schedule(sched).await?
            } else {
                api.create_schedule(sched).await?
            };
            to_json(&saved)
        })
    }

    pub async fn delete_schedule(&self, id: String) -> Result<(), FfiError> {
        ffi_async!(self, |api| Ok(api
            .delete_schedule(d::ScheduleId(id))
            .await?))
    }

    // ----- automation (JSON: `swoop_domain::automation::AutomationRule`) -----

    pub async fn automations_json(&self) -> Result<String, FfiError> {
        ffi_async!(self, |api| to_json(&api.list_automations().await?))
    }

    pub async fn save_automation_json(&self, json: String) -> Result<String, FfiError> {
        ffi_async!(self, |api| {
            let a: AutomationRule =
                serde_json::from_value(overlay(&AutomationRule::new(""), &json)?)?;
            let exists = api.list_automations().await?.iter().any(|x| x.id == a.id);
            let saved = if exists {
                api.update_automation(a).await?
            } else {
                api.create_automation(a).await?
            };
            to_json(&saved)
        })
    }

    pub async fn delete_automation(&self, id: String) -> Result<(), FfiError> {
        ffi_async!(self, |api| Ok(api
            .delete_automation(d::AutomationId(id))
            .await?))
    }

    /// Record the user's consent for the rule's code-executing actions; returns the rule JSON.
    pub async fn grant_automation_consent(&self, id: String) -> Result<String, FfiError> {
        ffi_async!(self, |api| to_json(
            &api.grant_automation_consent(d::AutomationId(id)).await?
        ))
    }

    pub async fn automation_runs_json(
        &self,
        id: Option<String>,
        limit: u32,
    ) -> Result<String, FfiError> {
        ffi_async!(self, |api| to_json(
            &api.automation_runs(id.map(d::AutomationId), limit).await?
        ))
    }

    /// Run an automation now against a task; returns the `AutomationRun` JSON.
    pub async fn run_automation(&self, id: String, task_id: String) -> Result<String, FfiError> {
        ffi_async!(self, |api| to_json(
            &api.run_automation(d::AutomationId(id), d::TaskId(task_id))
                .await?
        ))
    }

    // ----- recipes (JSON: `swoop_services::Recipe`) -----

    pub async fn recipes_json(&self) -> Result<String, FfiError> {
        ffi_async!(self, |api| to_json(&api.list_recipes().await?))
    }

    pub async fn save_recipe_json(&self, json: String) -> Result<String, FfiError> {
        let out = ffi_async!(self, |api| {
            let r: s::Recipe = serde_json::from_value(overlay(&empty_recipe(), &json)?)?;
            to_json(&api.save_recipe(r).await?)
        })?;
        self.emit_local(FfiEvent::RecipesChanged);
        Ok(out)
    }

    pub async fn delete_recipe(&self, id: String) -> Result<(), FfiError> {
        ffi_async!(self, |api| Ok(api.delete_recipe(d::RecipeId(id)).await?))?;
        self.emit_local(FfiEvent::RecipesChanged);
        Ok(())
    }

    pub async fn apply_recipe(
        &self,
        id: String,
        request: FfiNewTaskRequest,
    ) -> Result<FfiAddTaskResult, FfiError> {
        ffi_async!(self, |api| Ok((&api
            .apply_recipe(d::RecipeId(id), request.try_into()?)
            .await?)
            .into()))
    }

    // ----- history -----

    pub async fn history(&self, query: FfiHistoryQuery) -> Result<Vec<FfiHistoryEntry>, FfiError> {
        ffi_async!(self, |api| Ok(api
            .history(query.into())
            .await?
            .iter()
            .map(Into::into)
            .collect()))
    }

    pub async fn history_count(&self, query: FfiHistoryQuery) -> Result<u32, FfiError> {
        ffi_async!(self, |api| Ok(api.history_count(query.into()).await?))
    }

    pub async fn delete_history(&self, ids: Vec<String>) -> Result<u32, FfiError> {
        ffi_async!(self, |api| Ok(api
            .delete_history(ids.into_iter().map(d::TaskId).collect())
            .await?))
    }

    pub async fn clear_history(&self) -> Result<u32, FfiError> {
        ffi_async!(self, |api| Ok(api.clear_history().await?))
    }

    // ----- bandwidth & settings -----

    pub async fn set_traffic_mode(&self, mode: FfiTrafficMode) -> Result<(), FfiError> {
        ffi_async!(self, |api| Ok(api.set_traffic_mode(mode.into()).await?))
    }

    /// Bytes/s; 0 = unlimited.
    pub async fn set_global_limits(&self, download: u64, upload: u64) -> Result<(), FfiError> {
        ffi_async!(self, |api| Ok(api
            .set_global_limits(download, upload)
            .await?))
    }

    /// Measure the link and tune connections; returns the new settings JSON.
    pub async fn optimize(&self) -> Result<String, FfiError> {
        ffi_async!(self, |api| to_json(&api.optimize().await?))
    }

    /// Current settings (`swoop_domain::settings::Settings`) as JSON.
    pub fn settings_json(&self) -> String {
        serde_json::to_string(&*self.api.settings()).unwrap_or_else(|_| "{}".into())
    }

    /// Deep-merge `json` (full settings or any subset of sections/fields) into the current
    /// settings, validate and apply. Returns the stored settings JSON.
    pub async fn update_settings_json(&self, json: String) -> Result<String, FfiError> {
        ffi_async!(self, |api| {
            let mut current = serde_json::to_value(&*api.settings())?;
            merge(&mut current, serde_json::from_str(&json)?);
            let settings: Settings = serde_json::from_value(current)?;
            to_json(&api.update_settings(settings).await?)
        })
    }

    /// Store a secret in the macOS keychain; returns the credential id for `FfiTaskOptions`.
    pub async fn store_credential(
        &self,
        name: String,
        username: Option<String>,
        secret: String,
    ) -> Result<String, FfiError> {
        ffi_async!(self, |api| Ok(api
            .store_credential(name, username, secret)
            .await?
            .0))
    }

    pub async fn credentials(&self) -> Result<Vec<FfiCredential>, FfiError> {
        ffi_async!(self, |api| Ok(api
            .list_credentials()
            .await?
            .into_iter()
            .map(|(id, name, username)| FfiCredential {
                id: id.0,
                name,
                username,
            })
            .collect()))
    }

    pub async fn delete_credential(&self, id: String) -> Result<(), FfiError> {
        ffi_async!(self, |api| Ok(api
            .delete_credential(d::CredentialId(id))
            .await?))
    }

    // ----- stats / dashboard / disk -----

    pub fn global_stats(&self) -> FfiGlobalStats {
        (&self.api.global_stats()).into()
    }

    pub async fn dashboard(&self) -> Result<FfiDashboard, FfiError> {
        ffi_async!(self, |api| {
            let db = api.dashboard().await?;
            let mut recent = Vec::with_capacity(db.recent.len());
            for r in &db.recent {
                match api.get_task(r.id.clone()).await {
                    Ok(t) => recent.push(row(&t)),
                    Err(_) => recent.push(r.into()),
                }
            }
            Ok(FfiDashboard {
                stats: (&db.stats).into(),
                queues: db.queues.iter().map(Into::into).collect(),
                recent,
                speed_history: db
                    .speed_history
                    .iter()
                    .map(|x| FfiSpeedSample {
                        at: x.at.0,
                        download: x.download,
                        upload: x.upload,
                    })
                    .collect(),
                disks: db.disks.iter().map(Into::into).collect(),
                scheduled_next: db
                    .scheduled_next
                    .iter()
                    .map(|(id, at)| FfiScheduleBoundary {
                        schedule_id: id.0.clone(),
                        at: at.0,
                    })
                    .collect(),
            })
        })
    }

    /// Free space for `path` (`nil` = the default download directory).
    pub async fn disk_info(&self, path: Option<String>) -> Result<FfiDiskInfo, FfiError> {
        ffi_async!(self, |api| Ok((&api
            .disk_info(path.map(PathBuf::from))
            .await?)
            .into()))
    }

    // ----- devices -----

    pub async fn devices(&self) -> Result<Vec<FfiDevice>, FfiError> {
        ffi_async!(self, |api| Ok(api
            .list_devices()
            .await?
            .iter()
            .map(Into::into)
            .collect()))
    }

    pub async fn start_pairing(&self, scopes: Vec<FfiScope>) -> Result<FfiPairingInfo, FfiError> {
        let mut info: FfiPairingInfo = ffi_async!(self, |api| Ok((&api
            .start_pairing(scopes.into_iter().map(Into::into).collect())
            .await?)
            .into()))?;
        if info.tls_fingerprint.is_none() {
            info.tls_fingerprint = self.server.info().tls_fingerprint;
        }
        Ok(info)
    }

    pub async fn cancel_pairing(&self) -> Result<(), FfiError> {
        ffi_async!(self, |api| Ok(api.cancel_pairing().await?))
    }

    pub async fn revoke_device(&self, id: String) -> Result<(), FfiError> {
        ffi_async!(self, |api| Ok(api.revoke_device(d::DeviceId(id)).await?))
    }

    pub async fn rename_device(&self, id: String, name: String) -> Result<FfiDevice, FfiError> {
        ffi_async!(self, |api| Ok((&api
            .rename_device(d::DeviceId(id), name)
            .await?)
            .into()))
    }

    pub async fn audit_log(&self, limit: u32) -> Result<Vec<FfiAuditEntry>, FfiError> {
        ffi_async!(self, |api| Ok(api
            .audit_log(limit)
            .await?
            .iter()
            .map(Into::into)
            .collect()))
    }

    // ----- site grabber -----

    pub async fn grabber_start(
        &self,
        options: FfiGrabberOptions,
    ) -> Result<FfiGrabberSession, FfiError> {
        ffi_async!(self, |api| Ok(
            (&api.grabber_start(options.into()).await?).into()
        ))
    }

    pub async fn grabber_status(&self, id: String) -> Result<FfiGrabberSession, FfiError> {
        ffi_async!(self, |api| Ok((&api.grabber_status(id).await?).into()))
    }

    pub async fn grabber_cancel(&self, id: String) -> Result<(), FfiError> {
        ffi_async!(self, |api| Ok(api.grabber_cancel(id).await?))
    }

    pub async fn grabber_add(
        &self,
        id: String,
        urls: Vec<String>,
        request: FfiNewTaskRequest,
    ) -> Result<Vec<FfiAddTaskResult>, FfiError> {
        ffi_async!(self, |api| Ok(api
            .grabber_add(id, urls, request.try_into()?)
            .await?
            .iter()
            .map(Into::into)
            .collect()))
    }

    pub async fn grabber_list(&self) -> Result<Vec<FfiGrabberSession>, FfiError> {
        ffi_async!(self, |api| Ok(api
            .grabber_list()
            .await?
            .iter()
            .map(Into::into)
            .collect()))
    }

    // ----- archives -----

    pub async fn archive_list(&self, path: String) -> Result<FfiArchiveListing, FfiError> {
        ffi_async!(self, |api| Ok((&api
            .archive_list(PathBuf::from(path))
            .await?)
            .into()))
    }

    /// Extract `entries` (`nil` = everything) into `destination`; returns the entry count.
    pub async fn archive_extract(
        &self,
        path: String,
        entries: Option<Vec<String>>,
        destination: String,
    ) -> Result<u32, FfiError> {
        ffi_async!(self, |api| Ok(api
            .archive_extract(PathBuf::from(path), entries, PathBuf::from(destination))
            .await?))
    }

    // ----- import / export -----

    /// Export settings, queues, categories, rules, schedules, automations, recipes (and
    /// optionally tasks/history) as a JSON bundle.
    pub async fn export_json(
        &self,
        include_tasks: bool,
        include_history: bool,
    ) -> Result<String, FfiError> {
        ffi_async!(self, |api| {
            let bundle = api.export(include_tasks, include_history).await?;
            serde_json::to_string_pretty(&bundle).map_err(|e| FfiError::internal(e.to_string()))
        })
    }

    pub async fn import_json(
        &self,
        json: String,
        options: FfiImportOptions,
    ) -> Result<FfiImportReport, FfiError> {
        let report = ffi_async!(self, |api| {
            let bundle: s::ExportBundle = serde_json::from_str(&json)?;
            Ok((&api.import(bundle, options.into()).await?).into())
        })?;
        self.emit_local(FfiEvent::RecipesChanged);
        Ok(report)
    }

    // ----- updates / plugins / logs -----

    pub async fn check_for_updates(&self) -> Result<FfiUpdateInfo, FfiError> {
        ffi_async!(self, |api| Ok((&api.check_for_updates().await?).into()))
    }

    /// Download + verify the update; returns the local path of the verified archive.
    pub async fn download_update(&self) -> Result<String, FfiError> {
        ffi_async!(self, |api| Ok(path_str(&api.download_update().await?)))
    }

    /// The app's own update check: quiet (no engine notification). `manual` shows a release the
    /// user chose to skip.
    pub async fn check_app_update(&self, manual: bool) -> Result<FfiUpdateInfo, FfiError> {
        let engine = self.engine.clone();
        join(
            self.handle
                .spawn(async move { Ok((&engine.check_app_update(manual).await?).into()) })
                .await,
        )
    }

    /// The GitHub releases page updates come from.
    pub fn releases_page_url(&self) -> String {
        swoop_update::releases_page()
    }

    /// Progress of the update download / verification / staging.
    pub fn update_progress(&self) -> FfiUpdateProgress {
        self.engine.update_progress().into()
    }

    /// Re-verify the downloaded update, mount it, validate the app inside and stage a copy next
    /// to `bundle_path` (the running Swoop.app). Returns the staged bundle's path.
    pub async fn stage_update(&self, bundle_path: String) -> Result<String, FfiError> {
        let engine = self.engine.clone();
        join(
            self.handle
                .spawn(async move {
                    Ok(path_str(
                        &engine.stage_update(PathBuf::from(bundle_path)).await?,
                    ))
                })
                .await,
        )
    }

    pub async fn plugins(&self) -> Result<Vec<FfiPluginInfo>, FfiError> {
        ffi_async!(self, |api| Ok(api
            .list_plugins()
            .await?
            .iter()
            .map(Into::into)
            .collect()))
    }

    pub async fn set_plugin_enabled(
        &self,
        id: String,
        enabled: bool,
        granted_permissions: Vec<String>,
    ) -> Result<FfiPluginInfo, FfiError> {
        ffi_async!(self, |api| Ok((&api
            .set_plugin_enabled(d::PluginId(id), enabled, granted_permissions)
            .await?)
            .into()))
    }

    pub async fn uninstall_plugin(&self, id: String) -> Result<(), FfiError> {
        ffi_async!(self, |api| Ok(api
            .uninstall_plugin(d::PluginId(id))
            .await?))
    }

    /// Recent process log lines (redacted), oldest first; `level` filters at-or-above.
    pub async fn recent_logs(
        &self,
        limit: u32,
        level: Option<String>,
    ) -> Result<Vec<String>, FfiError> {
        ffi_async!(self, |api| Ok(api.recent_logs(limit, level).await?))
    }

    // ----- platform inputs -----

    /// Push platform measurements (network, power, VPN). Non-blocking.
    pub fn update_environment(&self, env: FfiEnvironment) {
        let api = self.api.clone();
        self.handle.spawn(async move {
            api.update_environment(env.into()).await;
        });
    }

    /// Whether the main window is frontmost (quiet notifications while active).
    pub fn set_active_window(&self, active: bool) {
        self.engine.set_active_window(active);
    }

    /// Graceful shutdown: stop listeners and the API servers, pause transfers, flush state.
    /// Idempotent. The object must not be used afterwards (calls fail with `Unavailable`).
    pub async fn shutdown(&self) -> Result<(), FfiError> {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        if let Some(w) = self.watcher.lock().take() {
            w.abort();
        }
        let server = self.server.clone();
        let listeners: Vec<JoinHandle<()>> =
            self.listeners.lock().drain().map(|(_, h)| h).collect();
        ffi_async!(self, |api| {
            server.shutdown().await;
            api.shutdown().await;
            // Let forwarders deliver `EngineStopping` before they are stopped.
            tokio::time::sleep(events::BATCH_WINDOW * 2).await;
            for l in listeners {
                l.abort();
            }
            Ok(())
        })
    }
}

async fn apply_action(
    api: &SharedEngine,
    id: d::TaskId,
    action: FfiTaskAction,
) -> FfiResult<d::Task> {
    let t = match action {
        FfiTaskAction::Start => api.start_task(id).await?,
        FfiTaskAction::Pause => api.pause_task(id).await?,
        FfiTaskAction::Resume => api.resume_task(id).await?,
        FfiTaskAction::Restart => api.restart_task(id).await?,
        FfiTaskAction::Retry => api.retry_task(id).await?,
        FfiTaskAction::RetryFromSource { new_url } => api.retry_from_source(id, new_url).await?,
        FfiTaskAction::Cancel => api.cancel_task(id).await?,
        FfiTaskAction::Redownload => api.redownload(id).await?,
        FfiTaskAction::Verify { checksum } => {
            let checksum =
                match checksum.filter(|c| !c.trim().is_empty()) {
                    None => None,
                    Some(c) => Some(d::Checksum::parse(&c).ok_or_else(|| {
                        FfiError::validation(format!("unrecognised checksum `{c}`"))
                    })?),
                };
            api.verify_task(id, checksum).await?
        }
        FfiTaskAction::RetrySegments => api.retry_failed_segments(id).await?,
        FfiTaskAction::Duplicate => api.duplicate_task(id).await?,
    };
    Ok(t)
}

impl Drop for SwoopEngine {
    fn drop(&mut self) {
        for (_, l) in self.listeners.lock().drain() {
            l.abort();
        }
        if let Some(rt) = self.rt.lock().take() {
            // Safe from any thread, including a runtime worker.
            rt.shutdown_background();
        }
    }
}
