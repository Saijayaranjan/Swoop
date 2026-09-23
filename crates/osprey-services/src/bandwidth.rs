//! Bandwidth management: the limiter tree (global → queue → task), traffic modes and the
//! "Optimize" heuristic.

use crate::engine::Engine;
use osprey_domain::queue::{Queue, TrafficMode};
use osprey_domain::settings::Settings;
use osprey_domain::{DomainResult, Millis, QueueId, TaskOptions};
use osprey_runtime::RateLimiter;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

/// Owns the global and per-queue limiters; task limiters are created per run as children.
/// (download, upload) limiters for one queue.
type QueueLimiters = (Arc<RateLimiter>, Arc<RateLimiter>);

pub struct BandwidthManager {
    global_down: Arc<RateLimiter>,
    global_up: Arc<RateLimiter>,
    queues: Mutex<HashMap<QueueId, QueueLimiters>>,
}

impl Default for BandwidthManager {
    fn default() -> Self {
        Self::new()
    }
}

impl BandwidthManager {
    pub fn new() -> Self {
        Self {
            global_down: RateLimiter::new("global", 0, None),
            global_up: RateLimiter::new("global-up", 0, None),
            queues: Mutex::new(HashMap::new()),
        }
    }

    /// Apply the effective global limits and burst size from settings.
    pub fn apply_settings(&self, settings: &Settings) {
        let (down, up) = settings.bandwidth.effective_limits();
        self.global_down.set_limit(down);
        self.global_up.set_limit(up);
        self.global_down.set_burst(settings.bandwidth.burst_bytes);
        self.global_up.set_burst(settings.bandwidth.burst_bytes);
    }

    /// Current effective global limits (download, upload).
    pub fn global_limits(&self) -> (u64, u64) {
        (self.global_down.limit(), self.global_up.limit())
    }

    /// Ensure the queue's limiters exist and reflect its profile.
    pub fn apply_queue(&self, queue: &Queue) {
        let mut q = self.queues.lock();
        let entry = q.entry(queue.id.clone()).or_insert_with(|| {
            (
                self.global_down.child(
                    format!("queue:{}", queue.id),
                    queue.bandwidth.download_limit,
                ),
                self.global_up.child(
                    format!("queue-up:{}", queue.id),
                    queue.bandwidth.upload_limit,
                ),
            )
        });
        entry.0.set_limit(queue.bandwidth.download_limit);
        entry.1.set_limit(queue.bandwidth.upload_limit);
    }

    pub fn remove_queue(&self, id: &QueueId) {
        self.queues.lock().remove(id);
    }

    /// Fresh per-task limiters parented to the queue (or the global limiter when the queue is
    /// unknown). `options` supplies the task-level caps.
    pub fn task_limiters(
        &self,
        queue_id: &QueueId,
        task_id: &str,
        options: &TaskOptions,
    ) -> (Arc<RateLimiter>, Arc<RateLimiter>) {
        let parents = self.queues.lock().get(queue_id).cloned();
        let (pd, pu) =
            parents.unwrap_or_else(|| (self.global_down.clone(), self.global_up.clone()));
        (
            pd.child(
                format!("task:{task_id}"),
                options.download_limit.unwrap_or(0),
            ),
            pu.child(
                format!("task-up:{task_id}"),
                options.upload_limit.unwrap_or(0),
            ),
        )
    }
}

impl Engine {
    /// Change the traffic mode (persisted) and re-apply the global limiter.
    pub(crate) async fn set_traffic_mode_inner(&self, mode: TrafficMode) -> DomainResult<()> {
        let mut s = (*self.settings()).clone();
        s.bandwidth.mode = mode;
        self.update_settings_inner(s).await.map(|_| ())
    }

    /// Set custom limits and switch to `Custom` mode.
    pub(crate) async fn set_global_limits_inner(
        &self,
        download: u64,
        upload: u64,
    ) -> DomainResult<()> {
        let mut s = (*self.settings()).clone();
        s.bandwidth.custom_download_limit = download;
        s.bandwidth.custom_upload_limit = upload;
        s.bandwidth.mode = TrafficMode::Custom;
        self.update_settings_inner(s).await.map(|_| ())
    }

    /// Measure the link from recent speed samples and derive connection settings.
    pub(crate) async fn optimize_inner(&self) -> DomainResult<Arc<Settings>> {
        let since = Millis::now().saturating_add_ms(-10 * 60_000);
        let samples = self.store.speed_samples(since).await?;
        let peak = samples.iter().map(|s| s.download).max().unwrap_or(0);
        let current = self.global_stats_snapshot().download_speed;
        let capacity = peak.max(current);
        let ranges_supported = self
            .tasks
            .all()
            .iter()
            .filter(|t| t.stats.range_supported.is_some())
            .rev()
            .take(20)
            .all(|t| t.stats.range_supported == Some(true));
        let mut s = (*self.settings()).clone();
        if capacity > 0 {
            s.bandwidth.measured_capacity = capacity;
            if ranges_supported {
                let per = (capacity / (2 * 1024 * 1024)).clamp(4, 16) as u8;
                s.network.connections_per_task = per;
            }
        }
        s.network.adaptive_segmentation = true;
        let applied = self.update_settings_inner(s).await?;
        Ok(applied)
    }
}
