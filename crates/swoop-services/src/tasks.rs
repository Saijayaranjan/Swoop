//! The in-memory task table, per-run handles and the engine-facing progress sink.
//!
//! Every task lives in a `DashMap<TaskId, Arc<Mutex<Task>>>`; the UI is served from here, never
//! from SQLite. A running task additionally has a [`RunHandle`] holding its control knobs,
//! limiters and speed meter. [`TaskSink`] is what engines report into; it applies changes in
//! memory synchronously and queues the durable write.

use crate::engine::Engine;
use dashmap::DashMap;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Instant;
use swoop_domain::checkpoint::Checkpoint;
use swoop_domain::events::LogLevel;
use swoop_domain::health::HealthInputs;
use swoop_domain::{Progress, Task, TaskId, TaskState};
use swoop_runtime::engine::{EngineStat, ProgressSink, ResolvedMetadata, TransferControl};
use swoop_runtime::speed::SpeedMeter;
use swoop_runtime::RateLimiter;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// Debounce for checkpoint persistence (spec: ≤ every 3 s, immediate on pause/stop).
pub const CHECKPOINT_DEBOUNCE: std::time::Duration = std::time::Duration::from_secs(3);

/// Live measurements feeding the health score (reset per run).
#[derive(Debug, Default)]
pub struct HealthAccum {
    pub successful_connections: u32,
    pub failed_connections: u32,
    pub retries: u32,
    pub throttled_events: u32,
    pub mirrors_switched: u32,
    pub throughput_drops: u32,
    pub samples: u32,
}

impl HealthAccum {
    /// Build the score inputs from the accumulated counters plus the speed meter.
    pub fn inputs(&self, meter: &SpeedMeter, task: &Task, active_seconds: u64) -> HealthInputs {
        HealthInputs {
            retries: self.retries + task.stats.retries,
            failed_connections: self.failed_connections,
            successful_connections: self.successful_connections,
            throughput_drops: self.throughput_drops + meter.drops(),
            samples: self.samples,
            range_supported: task.stats.range_supported,
            mirrors_switched: self.mirrors_switched,
            throttled_events: self.throttled_events,
            fraction_done: match task.progress.total {
                Some(t) if t > 0 => (task.progress.downloaded as f32 / t as f32).min(1.0),
                _ => task.progress.fraction,
            },
            speed_cv: meter.coefficient_of_variation(),
            active_seconds,
        }
    }
}

/// Everything attached to one run of a task.
pub struct RunHandle {
    /// Monotonic per-process run number; sinks of stale runs are ignored.
    pub run_id: u64,
    pub control: Arc<TransferControl>,
    pub limiter: Arc<RateLimiter>,
    pub upload_limiter: Arc<RateLimiter>,
    pub meter: Mutex<SpeedMeter>,
    pub upload_meter: Mutex<SpeedMeter>,
    pub health: Mutex<HealthAccum>,
    pub started: Instant,
    pub join: Mutex<Option<JoinHandle<()>>>,
    /// Latest checkpoint not yet persisted.
    pub pending_checkpoint: Mutex<Option<Checkpoint>>,
    /// A debounced checkpoint flush is scheduled.
    pub checkpoint_armed: AtomicBool,
    /// Last sampled counters (for speed deltas).
    pub last_downloaded: AtomicU64,
    pub last_uploaded: AtomicU64,
    /// Flips to `true` once the outcome has been fully handled.
    pub finished: watch::Sender<bool>,
}

impl RunHandle {
    pub(crate) fn new(
        run_id: u64,
        control: Arc<TransferControl>,
        limiter: Arc<RateLimiter>,
        upload_limiter: Arc<RateLimiter>,
    ) -> Arc<Self> {
        let (finished, _) = watch::channel(false);
        Arc::new(Self {
            run_id,
            control,
            limiter,
            upload_limiter,
            meter: Mutex::new(SpeedMeter::new()),
            upload_meter: Mutex::new(SpeedMeter::new()),
            health: Mutex::new(HealthAccum::default()),
            started: Instant::now(),
            join: Mutex::new(None),
            pending_checkpoint: Mutex::new(None),
            checkpoint_armed: AtomicBool::new(false),
            last_downloaded: AtomicU64::new(0),
            last_uploaded: AtomicU64::new(0),
            finished,
        })
    }

    /// Wait (bounded) until the run's outcome has been processed.
    pub async fn wait_finished(&self, timeout: std::time::Duration) -> bool {
        let mut rx = self.finished.subscribe();
        if *rx.borrow() {
            return true;
        }
        tokio::time::timeout(timeout, async {
            while rx.changed().await.is_ok() {
                if *rx.borrow() {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false)
    }
}

/// The task registry.
#[derive(Default)]
pub struct TaskTable {
    tasks: DashMap<TaskId, Arc<Mutex<Task>>>,
    runs: DashMap<TaskId, Arc<RunHandle>>,
    next_run: AtomicU64,
}

impl TaskTable {
    pub fn insert(&self, task: Task) -> Arc<Mutex<Task>> {
        let id = task.id.clone();
        let cell = Arc::new(Mutex::new(task));
        self.tasks.insert(id, cell.clone());
        cell
    }

    pub fn get(&self, id: &TaskId) -> Option<Arc<Mutex<Task>>> {
        self.tasks.get(id).map(|e| e.value().clone())
    }

    pub fn remove(&self, id: &TaskId) -> Option<Task> {
        self.runs.remove(id);
        self.tasks.remove(id).map(|(_, cell)| cell.lock().clone())
    }

    pub fn contains(&self, id: &TaskId) -> bool {
        self.tasks.contains_key(id)
    }

    /// Snapshot of a task.
    pub fn snapshot(&self, id: &TaskId) -> Option<Task> {
        self.get(id).map(|c| c.lock().clone())
    }

    /// Snapshot of every task.
    pub fn all(&self) -> Vec<Task> {
        self.tasks
            .iter()
            .map(|e| e.value().lock().clone())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Ids of every task.
    pub fn ids(&self) -> Vec<TaskId> {
        self.tasks.iter().map(|e| e.key().clone()).collect()
    }

    pub fn next_run_id(&self) -> u64 {
        self.next_run.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn run(&self, id: &TaskId) -> Option<Arc<RunHandle>> {
        self.runs.get(id).map(|e| e.value().clone())
    }

    pub fn set_run(&self, id: &TaskId, run: Arc<RunHandle>) {
        self.runs.insert(id.clone(), run);
    }

    /// Remove the run handle only if it still belongs to `run_id`.
    pub fn clear_run(&self, id: &TaskId, run_id: u64) -> Option<Arc<RunHandle>> {
        let current = self.runs.get(id).map(|e| e.value().run_id);
        if current == Some(run_id) {
            self.runs.remove(id).map(|(_, r)| r)
        } else {
            None
        }
    }

    /// Ids of every task with a live run.
    pub fn running(&self) -> Vec<(TaskId, Arc<RunHandle>)> {
        self.runs
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect()
    }

    pub fn running_count(&self) -> usize {
        self.runs.len()
    }

    /// Whether the run identified by `run_id` is still the current one for the task.
    pub fn is_current_run(&self, id: &TaskId, run_id: u64) -> bool {
        self.runs.get(id).map(|e| e.value().run_id) == Some(run_id)
    }
}

/// Per-task in-memory log ring (last 500 lines).
pub const LOG_RING_CAPACITY: usize = 500;

/// Bounded per-task log rings.
#[derive(Default)]
pub struct LogRings {
    rings: DashMap<TaskId, VecDeque<swoop_domain::TaskLogEntry>>,
}

impl LogRings {
    pub fn push(&self, entry: swoop_domain::TaskLogEntry) {
        let mut ring = self.rings.entry(entry.task_id.clone()).or_default();
        if ring.len() >= LOG_RING_CAPACITY {
            ring.pop_front();
        }
        ring.push_back(entry);
    }
    pub fn tail(&self, id: &TaskId, limit: usize) -> Vec<swoop_domain::TaskLogEntry> {
        let Some(ring) = self.rings.get(id) else {
            return Vec::new();
        };
        let skip = if limit == 0 {
            0
        } else {
            ring.len().saturating_sub(limit)
        };
        ring.iter().skip(skip).cloned().collect()
    }
    pub fn seed(&self, id: &TaskId, entries: Vec<swoop_domain::TaskLogEntry>) {
        let mut ring: VecDeque<_> = entries.into_iter().collect();
        while ring.len() > LOG_RING_CAPACITY {
            ring.pop_front();
        }
        self.rings.insert(id.clone(), ring);
    }
    pub fn remove(&self, id: &TaskId) {
        self.rings.remove(id);
    }
    pub fn has(&self, id: &TaskId) -> bool {
        self.rings.contains_key(id)
    }
}

/// The [`ProgressSink`] handed to an engine for one run.
pub struct TaskSink {
    engine: Weak<Engine>,
    task_id: TaskId,
    run_id: u64,
}

impl TaskSink {
    pub(crate) fn new(engine: Weak<Engine>, task_id: TaskId, run_id: u64) -> Arc<Self> {
        Arc::new(Self {
            engine,
            task_id,
            run_id,
        })
    }
    fn engine(&self) -> Option<Arc<Engine>> {
        let e = self.engine.upgrade()?;
        e.tasks
            .is_current_run(&self.task_id, self.run_id)
            .then_some(e)
    }
}

impl ProgressSink for TaskSink {
    fn progress(&self, progress: Progress) {
        if let Some(e) = self.engine() {
            e.sink_progress(&self.task_id, progress);
        }
    }
    fn state(&self, state: TaskState, detail: Option<String>) {
        if let Some(e) = self.engine() {
            e.sink_state(&self.task_id, self.run_id, state, detail);
        }
    }
    fn metadata(&self, metadata: ResolvedMetadata) {
        if let Some(e) = self.engine() {
            e.sink_metadata(&self.task_id, metadata);
        }
    }
    fn checkpoint(&self, checkpoint: Checkpoint) {
        if let Some(e) = self.engine() {
            e.sink_checkpoint(&self.task_id, self.run_id, checkpoint);
        }
    }
    fn log(&self, level: LogLevel, code: &str, message: String) {
        if let Some(e) = self.engine.upgrade() {
            e.task_log_line(&self.task_id, level, code, message);
        }
    }
    fn stat(&self, stat: EngineStat) {
        if let Some(e) = self.engine() {
            e.sink_stat(&self.task_id, stat);
        }
    }
}
