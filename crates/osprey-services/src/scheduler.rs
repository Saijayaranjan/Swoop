//! Scheduler: evaluates schedule windows and conditions (with hysteresis), gates attached tasks
//! and queues, and runs schedule actions when a window opens or closes.

use crate::engine::Engine;
use osprey_domain::schedule::{EnvironmentSnapshot, Schedule, ScheduleAction, CONDITION_DWELL_MS};
use osprey_domain::state::PauseReason;
use osprey_domain::{DomainError, DomainResult, Event, Millis, ScheduleId, Task, TaskState};
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use tokio::task::JoinHandle;

#[derive(Clone, Debug, Default)]
struct SchedState {
    /// Last committed activity.
    active: bool,
    /// Whether the time window was open at the last evaluation.
    window: bool,
    /// A condition-driven change waiting for the dwell time: (target, since).
    pending: Option<(bool, Millis)>,
    initialised: bool,
}

/// Cached schedules and their evaluation state.
#[derive(Default)]
pub struct Scheduler {
    schedules: RwLock<Vec<Schedule>>,
    state: Mutex<HashMap<ScheduleId, SchedState>>,
    boundary: Mutex<Option<JoinHandle<()>>>,
}

impl Scheduler {
    pub fn replace_all(&self, schedules: Vec<Schedule>) {
        *self.schedules.write() = schedules;
    }
    pub fn all(&self) -> Vec<Schedule> {
        self.schedules.read().clone()
    }
    pub fn get(&self, id: &ScheduleId) -> Option<Schedule> {
        self.schedules.read().iter().find(|s| &s.id == id).cloned()
    }
    pub fn contains(&self, id: &ScheduleId) -> bool {
        self.schedules.read().iter().any(|s| &s.id == id)
    }
    pub fn upsert(&self, s: Schedule) {
        let mut all = self.schedules.write();
        match all.iter_mut().find(|x| x.id == s.id) {
            Some(slot) => *slot = s,
            None => all.push(s),
        }
    }
    pub fn remove(&self, id: &ScheduleId) {
        self.schedules.write().retain(|s| &s.id != id);
        self.state.lock().remove(id);
    }
    /// Committed activity of a schedule (`true` when never evaluated and the window is open).
    pub fn is_active(&self, id: &ScheduleId, now: Millis, env: &EnvironmentSnapshot) -> bool {
        let Some(s) = self.get(id) else {
            return true;
        };
        let st = self.state.lock().get(id).cloned();
        match st {
            Some(st) if st.initialised => st.active,
            _ => s.is_active(now, env),
        }
    }
    /// Next instant any window may change.
    pub fn next_boundary(&self, now: Millis) -> Vec<(ScheduleId, Millis)> {
        self.schedules
            .read()
            .iter()
            .filter(|s| s.enabled)
            .filter_map(|s| s.next_boundary_after(now).map(|b| (s.id.clone(), b)))
            .collect()
    }
}

impl Engine {
    /// Evaluate every schedule now. Called by the 60 s ticker, boundary timers, environment
    /// updates and (as a test hook) directly.
    pub async fn scheduler_tick_now(&self) {
        let now = Millis::now();
        let env = self.environment_snapshot();
        let schedules = self.scheduler.all();
        for s in schedules {
            let raw = s.is_active(now, &env);
            let window = s.enabled && s.window_open_at(now);
            let decision = {
                let mut states = self.scheduler.state.lock();
                let st = states.entry(s.id.clone()).or_default();
                let window_changed = st.window != window;
                st.window = window;
                if !st.initialised {
                    st.initialised = true;
                    st.active = raw;
                    Some((raw, true))
                } else if raw == st.active {
                    st.pending = None;
                    None
                } else if window_changed || s.conditions.is_empty() {
                    // The time window itself flipped: apply at once.
                    st.pending = None;
                    st.active = raw;
                    Some((raw, false))
                } else {
                    // Only conditions changed: require them to be stable for the dwell time.
                    match st.pending {
                        Some((target, since)) if target == raw => {
                            if now.0 - since.0 >= CONDITION_DWELL_MS {
                                st.pending = None;
                                st.active = raw;
                                Some((raw, false))
                            } else {
                                None
                            }
                        }
                        _ => {
                            st.pending = Some((raw, now));
                            None
                        }
                    }
                }
            };
            if let Some((active, initial)) = decision {
                {
                    if s.gate_attached {
                        self.gate_schedule(&s, active).await;
                    }
                    if !initial {
                        self.bus.publish(Event::ScheduleFired {
                            schedule_id: s.id.clone(),
                            opened: active,
                        });
                        let actions = if active { &s.on_start } else { &s.on_end };
                        for a in actions {
                            self.run_schedule_action(&s, a).await;
                        }
                        let mut updated = s.clone();
                        updated.last_fired_at = Some(now);
                        self.scheduler.upsert(updated.clone());
                        if let Err(e) = self.store.upsert_schedule(&updated).await {
                            tracing::debug!(error = %e, "schedule last_fired_at not persisted");
                        }
                        self.automation.fire_schedule(self, &s, active).await;
                    }
                }
            }
        }
        self.arm_boundary_timer(now);
        self.admission.notify_one();
    }

    fn arm_boundary_timer(&self, now: Millis) {
        let next = self
            .scheduler
            .next_boundary(now)
            .into_iter()
            .map(|(_, b)| b.0)
            .min();
        let mut slot = self.scheduler.boundary.lock();
        if let Some(h) = slot.take() {
            h.abort();
        }
        let Some(at) = next else {
            return;
        };
        let delay = (at - now.0).max(0) as u64 + 500;
        let weak = self.this.clone();
        *slot = Some(tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            if let Some(e) = weak.upgrade() {
                e.scheduler_tick_now().await;
            }
        }));
    }

    /// Does the task (or its queue) hang on a schedule whose window is currently closed?
    pub(crate) fn schedule_block_for(&self, task: &Task) -> Option<PauseReason> {
        let now = Millis::now();
        let env = self.environment_snapshot();
        let mut ids = Vec::new();
        if let Some(s) = &task.schedule_id {
            ids.push(s.clone());
        }
        if let Some(q) = self.queues.get(&task.queue_id) {
            if let Some(s) = q.schedule_id {
                ids.push(s);
            }
        }
        for id in ids {
            let Some(s) = self.scheduler.get(&id) else {
                continue;
            };
            if s.enabled && s.gate_attached && !self.scheduler.is_active(&id, now, &env) {
                return Some(PauseReason::Schedule(s.name.clone()));
            }
        }
        None
    }

    /// Block or unblock every task attached to `schedule` (directly or through its queue).
    async fn gate_schedule(&self, schedule: &Schedule, active: bool) {
        let reason = PauseReason::Schedule(schedule.name.clone());
        let gated_queues: Vec<_> = self
            .queues
            .all()
            .into_iter()
            .filter(|q| q.schedule_id.as_ref() == Some(&schedule.id))
            .map(|q| q.id)
            .collect();
        for id in self.tasks.ids() {
            let Some(cell) = self.tasks.get(&id) else {
                continue;
            };
            let attached = {
                let t = cell.lock();
                (t.schedule_id.as_ref() == Some(&schedule.id) || gated_queues.contains(&t.queue_id))
                    && !t.state.is_terminal()
                    && t.state != TaskState::Pending
            };
            if !attached {
                continue;
            }
            if active {
                self.unblock_task(&id, &reason).await;
            } else {
                self.block_task(&id, reason.clone()).await;
            }
        }
    }

    async fn run_schedule_action(&self, schedule: &Schedule, action: &ScheduleAction) {
        let result: DomainResult<()> = match action {
            ScheduleAction::StartQueue { queue_id } => self
                .set_queue_paused(queue_id.clone(), false)
                .await
                .map(|_| ()),
            ScheduleAction::PauseQueue { queue_id } => self
                .set_queue_paused(queue_id.clone(), true)
                .await
                .map(|_| ()),
            ScheduleAction::SetSpeedLimit { download, upload } => {
                self.set_global_limits_inner(*download, *upload).await
            }
            ScheduleAction::SetConnectionLimit { per_task } => {
                let mut s = (*self.settings()).clone();
                s.network.connections_per_task = (*per_task).clamp(1, 64);
                self.update_settings_inner(s).await.map(|_| ())
            }
            ScheduleAction::SetTrafficMode { mode } => self.set_traffic_mode_inner(*mode).await,
            ScheduleAction::LaunchApplication { path } => {
                self.bus.publish(Event::Custom {
                    name: "schedule.launch_application".into(),
                    payload: serde_json::json!({ "schedule_id": schedule.id, "path": path }),
                });
                Ok(())
            }
            ScheduleAction::RunAutomation { automation_id } => self
                .automation
                .run_by_id(
                    self,
                    &osprey_domain::AutomationId(automation_id.clone()),
                    None,
                    osprey_domain::automation::AutomationEvent::ScheduleFired,
                )
                .await
                .map(|_| ()),
            ScheduleAction::Notify { message } => {
                self.bus.publish(Event::Custom {
                    name: "schedule.notify".into(),
                    payload: serde_json::json!({ "schedule_id": schedule.id, "message": message }),
                });
                Ok(())
            }
            ScheduleAction::SleepComputer => {
                self.pause_all_with(PauseReason::Condition("sleep".into()))
                    .await;
                self.bus.publish(Event::ReadyForSleep {
                    reason: format!("schedule:{}", schedule.name),
                });
                Ok(())
            }
            ScheduleAction::QuitApplication => {
                self.pause_all_with(PauseReason::Condition("quit".into()))
                    .await;
                self.bus.publish(Event::ReadyForSleep {
                    reason: format!("quit:{}", schedule.name),
                });
                Ok(())
            }
        };
        if let Err(e) = result {
            tracing::warn!(schedule = %schedule.name, error = %e, "schedule action failed");
        }
    }

    fn validate_schedule(s: &mut Schedule) -> DomainResult<()> {
        s.name = s.name.trim().to_owned();
        if s.name.is_empty() {
            return Err(DomainError::validation("schedule name must not be empty"));
        }
        use osprey_domain::schedule::Recurrence;
        let check = |t: &str| -> DomainResult<()> {
            let ok = t
                .split_once(':')
                .and_then(|(h, m)| Some((h.parse::<u32>().ok()?, m.parse::<u32>().ok()?)))
                .map(|(h, m)| h < 24 && m < 60)
                .unwrap_or(false);
            if ok {
                Ok(())
            } else {
                Err(DomainError::validation(format!("invalid time {t:?}")))
            }
        };
        match &s.recurrence {
            Recurrence::Daily { start, end } => {
                check(start)?;
                check(end)?;
            }
            Recurrence::Weekly { days, start, end } => {
                check(start)?;
                check(end)?;
                if days.iter().any(|d| *d > 6) {
                    return Err(DomainError::validation("weekday must be 0..=6"));
                }
            }
            Recurrence::Range { from, to } if to.0 <= from.0 => {
                return Err(DomainError::validation("range end must be after start"));
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) async fn create_schedule_inner(&self, mut s: Schedule) -> DomainResult<Schedule> {
        Self::validate_schedule(&mut s)?;
        if self.scheduler.contains(&s.id) {
            return Err(DomainError::Conflict(format!("schedule {} exists", s.id)));
        }
        let now = Millis::now();
        s.created_at = now;
        s.updated_at = now;
        self.store.upsert_schedule(&s).await?;
        self.scheduler.upsert(s.clone());
        self.bus.publish(Event::ScheduleUpdated(s.clone()));
        self.scheduler_tick_now().await;
        Ok(s)
    }

    pub(crate) async fn update_schedule_inner(&self, mut s: Schedule) -> DomainResult<Schedule> {
        Self::validate_schedule(&mut s)?;
        let existing = self
            .scheduler
            .get(&s.id)
            .ok_or_else(|| DomainError::not_found(format!("schedule {}", s.id)))?;
        s.created_at = existing.created_at;
        s.updated_at = Millis::now();
        self.store.upsert_schedule(&s).await?;
        self.scheduler.upsert(s.clone());
        // Re-evaluate from scratch so a changed window is applied immediately.
        self.scheduler.state.lock().remove(&s.id);
        self.bus.publish(Event::ScheduleUpdated(s.clone()));
        self.scheduler_tick_now().await;
        Ok(s)
    }

    pub(crate) async fn delete_schedule_inner(&self, id: ScheduleId) -> DomainResult<()> {
        let existing = self
            .scheduler
            .get(&id)
            .ok_or_else(|| DomainError::not_found(format!("schedule {id}")))?;
        self.store.delete_schedule(&id).await?;
        // Lift the gate from everything attached, then detach.
        self.gate_schedule(&existing, true).await;
        for tid in self.tasks.ids() {
            let Some(cell) = self.tasks.get(&tid) else {
                continue;
            };
            let snap = {
                let mut t = cell.lock();
                if t.schedule_id.as_ref() != Some(&id) {
                    continue;
                }
                t.schedule_id = None;
                t.touch();
                t.clone()
            };
            self.persist
                .send(crate::persist::PersistOp::Update(Box::new(snap.clone())));
            self.bus.publish(Event::TaskUpdated(Box::new(snap)));
        }
        for mut q in self.queues.all() {
            if q.schedule_id.as_ref() == Some(&id) {
                q.schedule_id = None;
                if let Err(e) = self.store.upsert_queue(&q).await {
                    tracing::warn!(error = %e, "queue schedule detach not persisted");
                }
                self.queues.upsert(q.clone());
                self.bus.publish(Event::QueueUpdated(q));
            }
        }
        self.scheduler.remove(&id);
        self.bus.publish(Event::ScheduleRemoved { schedule_id: id });
        Ok(())
    }
}
