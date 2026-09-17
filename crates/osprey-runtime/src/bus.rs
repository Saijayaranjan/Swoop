//! The event bus. Publishers push [`Event`]s; subscribers receive them over a broadcast
//! channel. Progress updates are coalesced: engines report progress at any rate and the bus
//! emits at most one `Event::Progress` batch per `interval` containing the latest value per task.

use osprey_domain::{Event, ProgressUpdate, TaskId};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Arc<Event>>,
    pending: Arc<Mutex<HashMap<TaskId, ProgressUpdate>>>,
    interval: Duration,
}

pub type EventSubscription = broadcast::Receiver<Arc<Event>>;

impl EventBus {
    pub fn new(capacity: usize, interval: Duration) -> Self {
        let (tx, _rx) = broadcast::channel(capacity.max(64));
        let bus = Self { tx, pending: Arc::new(Mutex::new(HashMap::new())), interval };
        bus.spawn_flusher();
        bus
    }

    fn spawn_flusher(&self) {
        let bus = self.clone();
        // Only spawn when a runtime is present; tests without one call `flush()` directly.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut ticker = tokio::time::interval(bus.interval);
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    ticker.tick().await;
                    bus.flush();
                }
            });
        }
    }

    pub fn subscribe(&self) -> EventSubscription {
        self.tx.subscribe()
    }

    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// Publish immediately (state changes, additions, removals…).
    pub fn publish(&self, event: Event) {
        // A lagging subscriber is dropped by tokio; that is the intended behaviour for the UI
        // (it re-syncs from a snapshot) and for slow remote clients.
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
        self.publish(Event::Progress(batch));
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(4096, Duration::from_millis(250))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use osprey_domain::Progress;

    #[tokio::test]
    async fn coalesces_progress() {
        let bus = EventBus::new(64, Duration::from_secs(3600));
        let mut rx = bus.subscribe();
        let id = TaskId::new();
        for i in 0..10u64 {
            bus.progress(ProgressUpdate { task_id: id.clone(), progress: Progress { downloaded: i, ..Default::default() }, state: None });
        }
        bus.flush();
        let ev = rx.recv().await.unwrap();
        match &*ev {
            Event::Progress(batch) => {
                assert_eq!(batch.len(), 1);
                assert_eq!(batch[0].progress.downloaded, 9);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
