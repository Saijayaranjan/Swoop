//! The event bus. Publishers push [`Event`]s; subscribers receive them over a broadcast
//! channel. Progress updates are coalesced: engines report progress at any rate and the bus
//! emits at most one `Event::Progress` batch per `interval` containing the latest value per task.
//!
//! Ordering: a state change for a task first flushes that task's pending progress so
//! subscribers never see `Completed` before the final byte count; every update carries the
//! task `rev` so consumers can drop stale data after a resync.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use swoop_domain::{Event, ProgressUpdate, TaskId};
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Arc<Event>>,
    pending: Arc<Mutex<HashMap<TaskId, ProgressUpdate>>>,
    interval: Duration,
}

pub type EventSubscription = broadcast::Receiver<Arc<Event>>;

impl EventBus {
    /// Create a bus and start its flusher on `handle`. The flusher is what turns queued progress
    /// into events, so a bus without one never delivers progress — hence the explicit handle.
    pub fn new(handle: &tokio::runtime::Handle, capacity: usize, interval: Duration) -> Self {
        let (tx, _rx) = broadcast::channel(capacity.max(64));
        let bus = Self {
            tx,
            pending: Arc::new(Mutex::new(HashMap::new())),
            interval,
        };
        let flusher = bus.clone();
        handle.spawn(async move {
            let mut ticker = tokio::time::interval(flusher.interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                flusher.flush();
            }
        });
        bus
    }

    /// A bus for unit tests that never flushes on its own (call [`EventBus::flush`]).
    pub fn manual(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity.max(64));
        Self {
            tx,
            pending: Arc::new(Mutex::new(HashMap::new())),
            interval: Duration::from_secs(3600),
        }
    }

    pub fn subscribe(&self) -> EventSubscription {
        self.tx.subscribe()
    }

    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// Publish immediately (state changes, additions, removals…). Any pending progress for the
    /// task the event concerns is flushed first so ordering is preserved.
    pub fn publish(&self, event: Event) {
        if let Some(id) = event.task_id() {
            if let Some(p) = self.pending.lock().remove(id) {
                let _ = self.tx.send(Arc::new(Event::Progress(vec![p])));
            }
        }
        // A lagging subscriber is dropped by tokio; that is intended for the UI (it re-syncs
        // from a snapshot) and for slow remote clients.
        let _ = self.tx.send(Arc::new(event));
    }

    /// Queue a progress update; coalesced with any earlier update for the same task.
    pub fn progress(&self, update: ProgressUpdate) {
        self.pending.lock().insert(update.task_id.clone(), update);
    }

    /// Emit the pending progress batch now.
    pub fn flush(&self) {
        let batch: Vec<ProgressUpdate> = {
            let mut p = self.pending.lock();
            if p.is_empty() {
                return;
            }
            p.drain().map(|(_, v)| v).collect()
        };
        let _ = self.tx.send(Arc::new(Event::Progress(batch)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swoop_domain::Progress;

    #[tokio::test]
    async fn coalesces_progress_and_orders_before_state() {
        let bus = EventBus::manual(64);
        let mut rx = bus.subscribe();
        let id = TaskId::new();
        for i in 0..10u64 {
            bus.progress(ProgressUpdate {
                task_id: id.clone(),
                progress: Progress {
                    downloaded: i,
                    ..Default::default()
                },
                rev: i,
            });
        }
        bus.publish(Event::TaskStateChanged {
            task_id: id.clone(),
            from: swoop_domain::TaskState::Downloading,
            to: swoop_domain::TaskState::Completed,
            at: swoop_domain::Millis::now(),
        });
        let first = rx.recv().await.unwrap();
        match &*first {
            Event::Progress(batch) => {
                assert_eq!(batch.len(), 1);
                assert_eq!(batch[0].progress.downloaded, 9);
                assert_eq!(batch[0].rev, 9);
            }
            other => panic!("unexpected {other:?}"),
        }
        let second = rx.recv().await.unwrap();
        assert!(matches!(&*second, Event::TaskStateChanged { .. }));
    }

    #[tokio::test]
    async fn flusher_runs_on_handle() {
        let bus = EventBus::new(
            &tokio::runtime::Handle::current(),
            64,
            Duration::from_millis(10),
        );
        let mut rx = bus.subscribe();
        bus.progress(ProgressUpdate {
            task_id: TaskId::new(),
            progress: Progress::default(),
            rev: 1,
        });
        let ev = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(&*ev, Event::Progress(_)));
    }
}
