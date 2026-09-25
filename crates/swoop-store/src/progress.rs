//! Latest-wins progress batching so the services layer does not have to debounce hot rows.

use crate::error::StoreResult;
use crate::tasks::upsert_progress_row;
use crate::Store;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Duration;
use swoop_domain::{Millis, Progress, TaskId};

/// How often pending progress updates are written.
pub const PROGRESS_FLUSH_INTERVAL: Duration = Duration::from_secs(2);

struct Inner {
    store: Arc<Store>,
    pending: Mutex<HashMap<TaskId, Progress>>,
}

impl Inner {
    /// Enqueue everything pending as one write op. `sync` waits for the commit.
    async fn drain(&self, sync: bool) -> StoreResult<()> {
        let batch: Vec<(TaskId, Progress)> = {
            let mut guard = self.pending.lock();
            if guard.is_empty() {
                return Ok(());
            }
            guard.drain().collect()
        };
        let write = move |conn: &rusqlite::Connection| {
            let now = Millis::now();
            for (id, p) in &batch {
                upsert_progress_row(conn, id, p, now)?;
            }
            Ok(())
        };
        if sync {
            self.store.write("progress_batch", write).await
        } else {
            self.store.write_deferred("progress_batch", write)
        }
    }
}

/// Accepts `(task_id, Progress)` at any rate and writes the latest value per task every
/// [`PROGRESS_FLUSH_INTERVAL`] (and on [`ProgressBatcher::flush`]). Clones share one buffer;
/// the background task stops when the last clone is dropped.
#[derive(Clone)]
pub struct ProgressBatcher {
    inner: Arc<Inner>,
}

impl ProgressBatcher {
    pub(crate) fn new(store: Arc<Store>) -> Self {
        let inner = Arc::new(Inner {
            store,
            pending: Mutex::new(HashMap::new()),
        });
        let weak: Weak<Inner> = Arc::downgrade(&inner);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(PROGRESS_FLUSH_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let Some(inner) = weak.upgrade() else { break };
                if let Err(e) = inner.drain(false).await {
                    tracing::warn!(error = %e, "progress batch enqueue failed");
                    if matches!(e, crate::StoreError::Closed | crate::StoreError::WriterGone) {
                        break;
                    }
                }
            }
        });
        Self { inner }
    }

    /// Record the latest progress of a task (cheap; no I/O).
    pub fn update(&self, task_id: &TaskId, progress: &Progress) {
        self.inner
            .pending
            .lock()
            .insert(task_id.clone(), progress.clone());
    }

    /// Number of tasks with an unwritten update.
    pub fn pending(&self) -> usize {
        self.inner.pending.lock().len()
    }

    /// Write every pending update now and wait for the commit.
    pub async fn flush(&self) -> StoreResult<()> {
        self.inner.drain(true).await
    }
}

impl Store {
    /// A [`ProgressBatcher`] bound to this store. Requires a Tokio runtime.
    pub fn progress_writer(self: &Arc<Self>) -> ProgressBatcher {
        ProgressBatcher::new(Arc::clone(self))
    }
}
