//! Queue management: the cached queue list, CRUD with persistence, summaries and queue-level
//! pause gating.

use crate::engine::Engine;
use osprey_domain::queue::{Queue, QueueSummary};
use osprey_domain::state::PauseReason;
use osprey_domain::{DomainError, DomainResult, Event, Millis, QueueId, TaskState};
use parking_lot::RwLock;
use std::collections::HashMap;

/// In-memory queue registry (the store is the source of truth; this is the read cache).
#[derive(Default)]
pub struct QueueManager {
    queues: RwLock<Vec<Queue>>,
}

impl QueueManager {
    pub fn replace_all(&self, queues: Vec<Queue>) {
        let mut q = queues;
        q.sort_by_key(|x| (x.position, x.created_at));
        *self.queues.write() = q;
    }
    pub fn all(&self) -> Vec<Queue> {
        self.queues.read().clone()
    }
    pub fn get(&self, id: &QueueId) -> Option<Queue> {
        self.queues.read().iter().find(|q| &q.id == id).cloned()
    }
    pub fn contains(&self, id: &QueueId) -> bool {
        self.queues.read().iter().any(|q| &q.id == id)
    }
    pub fn upsert(&self, queue: Queue) {
        let mut q = self.queues.write();
        match q.iter_mut().find(|x| x.id == queue.id) {
            Some(slot) => *slot = queue,
            None => q.push(queue),
        }
        q.sort_by_key(|x| (x.position, x.created_at));
    }
    pub fn remove(&self, id: &QueueId) -> Option<Queue> {
        let mut q = self.queues.write();
        let pos = q.iter().position(|x| &x.id == id)?;
        Some(q.remove(pos))
    }
    /// Queues in admission order: `priority DESC, position ASC`.
    pub fn admission_order(&self) -> Vec<Queue> {
        let mut q = self.queues.read().clone();
        q.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then(a.position.cmp(&b.position))
        });
        q
    }
}

impl Engine {
    /// Aggregate per-queue counts and speeds from the task table.
    pub(crate) fn queue_summaries_now(&self) -> Vec<QueueSummary> {
        let mut map: HashMap<QueueId, QueueSummary> = self
            .queues
            .all()
            .into_iter()
            .map(|q| (q.id.clone(), q.summary()))
            .collect();
        for t in self.tasks.all() {
            let Some(s) = map.get_mut(&t.queue_id) else {
                continue;
            };
            match t.state {
                TaskState::Completed => s.completed += 1,
                TaskState::Failed => s.failed += 1,
                st if st.is_active() => {
                    s.active += 1;
                    s.download_speed += t.progress.speed;
                    s.upload_speed += t.progress.upload_speed;
                }
                st if st.is_waiting() || st == TaskState::Paused => s.waiting += 1,
                _ => {}
            }
        }
        let mut out: Vec<QueueSummary> = map.into_values().collect();
        out.sort_by(|a, b| a.queue_id.cmp(&b.queue_id));
        out
    }

    fn validate_queue(&self, queue: &mut Queue) -> DomainResult<()> {
        queue.name = queue.name.trim().to_owned();
        if queue.name.is_empty() {
            return Err(DomainError::validation("queue name must not be empty"));
        }
        if let Some(dir) = &queue.directory {
            let expanded = osprey_runtime::paths::AppPaths::expand_home(&dir.to_string_lossy());
            osprey_runtime::safety::validate_destination_dir(&expanded)
                .map_err(|e| DomainError::validation(e.message))?;
            queue.directory = Some(expanded);
        }
        if let Some(s) = &queue.schedule_id {
            if !self.scheduler.contains(s) {
                return Err(DomainError::not_found(format!("schedule {s}")));
            }
        }
        Ok(())
    }

    pub(crate) async fn create_queue_inner(&self, mut queue: Queue) -> DomainResult<Queue> {
        self.validate_queue(&mut queue)?;
        if self.queues.contains(&queue.id) {
            return Err(DomainError::Conflict(format!("queue {} exists", queue.id)));
        }
        queue.builtin = false;
        let now = Millis::now();
        queue.created_at = now;
        queue.updated_at = now;
        if queue.position == 0 {
            queue.position = self.queues.all().len() as i32;
        }
        self.store.upsert_queue(&queue).await?;
        self.bandwidth.apply_queue(&queue);
        self.queues.upsert(queue.clone());
        self.bus.publish(Event::QueueUpdated(queue.clone()));
        Ok(queue)
    }

    pub(crate) async fn update_queue_inner(&self, mut queue: Queue) -> DomainResult<Queue> {
        let existing = self
            .queues
            .get(&queue.id)
            .ok_or_else(|| DomainError::not_found(format!("queue {}", queue.id)))?;
        self.validate_queue(&mut queue)?;
        queue.builtin = existing.builtin;
        queue.created_at = existing.created_at;
        queue.updated_at = Millis::now();
        self.store.upsert_queue(&queue).await?;
        self.bandwidth.apply_queue(&queue);
        self.queues.upsert(queue.clone());
        self.bus.publish(Event::QueueUpdated(queue.clone()));
        if existing.paused != queue.paused {
            self.apply_queue_pause(&queue.id, &queue.name, queue.paused)
                .await;
        }
        if existing.schedule_id != queue.schedule_id {
            self.scheduler_tick_now().await;
        }
        self.admission.notify_one();
        Ok(queue)
    }

    pub(crate) async fn delete_queue_inner(
        &self,
        id: QueueId,
        move_tasks_to: Option<QueueId>,
    ) -> DomainResult<()> {
        let queue = self
            .queues
            .get(&id)
            .ok_or_else(|| DomainError::not_found(format!("queue {id}")))?;
        if queue.builtin {
            return Err(DomainError::PermissionDenied(
                "built-in queues cannot be deleted".into(),
            ));
        }
        let target = move_tasks_to.unwrap_or_else(QueueId::default_queue);
        if !self.queues.contains(&target) {
            return Err(DomainError::not_found(format!("queue {target}")));
        }
        for tid in self.tasks.ids() {
            let Some(cell) = self.tasks.get(&tid) else {
                continue;
            };
            let snapshot = {
                let mut t = cell.lock();
                if t.queue_id != id {
                    continue;
                }
                t.queue_id = target.clone();
                t.unblock(&PauseReason::Queue(queue.name.clone()));
                t.touch();
                t.clone()
            };
            self.persist
                .commit(crate::persist::PersistOp::Update(Box::new(
                    snapshot.clone(),
                )))
                .await?;
            self.bus.publish(Event::TaskUpdated(Box::new(snapshot)));
        }
        self.store.delete_queue(&id).await?;
        self.bandwidth.remove_queue(&id);
        self.queues.remove(&id);
        self.bus.publish(Event::QueueRemoved { queue_id: id });
        self.admission.notify_one();
        Ok(())
    }

    pub(crate) async fn set_queue_paused(&self, id: QueueId, paused: bool) -> DomainResult<Queue> {
        let mut queue = self
            .queues
            .get(&id)
            .ok_or_else(|| DomainError::not_found(format!("queue {id}")))?;
        if queue.paused == paused {
            return Ok(queue);
        }
        queue.paused = paused;
        queue.updated_at = Millis::now();
        self.store.upsert_queue(&queue).await?;
        self.queues.upsert(queue.clone());
        self.bus.publish(Event::QueueUpdated(queue.clone()));
        self.apply_queue_pause(&id, &queue.name, paused).await;
        self.admission.notify_one();
        Ok(queue)
    }

    /// Block (or unblock) every non-terminal task of a queue with `PauseReason::Queue`.
    pub(crate) async fn apply_queue_pause(&self, id: &QueueId, name: &str, paused: bool) {
        let reason = PauseReason::Queue(name.to_owned());
        for tid in self.tasks.ids() {
            let Some(cell) = self.tasks.get(&tid) else {
                continue;
            };
            let belongs = {
                let t = cell.lock();
                &t.queue_id == id && !t.state.is_terminal() && t.state != TaskState::Pending
            };
            if !belongs {
                continue;
            }
            if paused {
                self.block_task(&tid, reason.clone()).await;
            } else {
                self.unblock_task(&tid, &reason).await;
            }
        }
    }

    pub(crate) async fn reorder_queues_inner(&self, ids: Vec<QueueId>) -> DomainResult<()> {
        let mut all = self.queues.all();
        let mut pos = 0i32;
        for id in &ids {
            if let Some(q) = all.iter_mut().find(|q| &q.id == id) {
                q.position = pos;
                pos += 1;
            }
        }
        for q in all.iter_mut() {
            if !ids.contains(&q.id) {
                q.position = pos;
                pos += 1;
            }
        }
        for q in &all {
            self.store.upsert_queue(q).await?;
            self.queues.upsert(q.clone());
            self.bus.publish(Event::QueueUpdated(q.clone()));
        }
        Ok(())
    }
}
