//! Event delivery: bus `Event` → `FfiEvent`, batched per listener by a dedicated forwarder task
//! (never on the publisher's thread), flushed at most every 100 ms.

use crate::types::*;
use osprey_domain as d;
use osprey_domain::Event;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::{self, error::RecvError};

/// Maximum time events are held before a batch is delivered.
pub const BATCH_WINDOW: Duration = Duration::from_millis(100);
/// A batch is flushed early when it grows this large.
const MAX_BATCH: usize = 512;

/// Implemented in Swift. Called from a Rust background thread; implementations must return
/// quickly and hop to the main actor asynchronously (`Task { @MainActor in … }`).
#[uniffi::export(with_foreign)]
pub trait EventListener: Send + Sync {
    fn on_events(&self, events: Vec<FfiEvent>);
}

/// Notification-centre content (the app localises by case).
#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum FfiNotification {
    Completed {
        task_id: String,
        name: String,
        path: String,
    },
    Failed {
        task_id: String,
        name: String,
        reason: String,
    },
    Queued {
        task_id: String,
        name: String,
    },
    Scheduled {
        task_id: String,
        name: String,
        at: i64,
    },
    ChecksumMismatch {
        task_id: String,
        name: String,
    },
    LowDiskSpace {
        path: String,
        free: u64,
        required: u64,
    },
    TorrentFinished {
        task_id: String,
        name: String,
    },
    DevicePaired {
        device_id: String,
        name: String,
    },
    AutomationFailed {
        automation_id: String,
        name: String,
        error: String,
    },
    DuplicateDetected {
        task_id: String,
        name: String,
        existing_path: String,
    },
    UpdateAvailable {
        version: String,
        notes_url: Option<String>,
    },
    QueueFinished {
        queue_id: String,
        name: String,
    },
    /// `automation.notify` / `schedule.notify` actions.
    Custom {
        title: String,
        body: String,
        task_id: Option<String>,
    },
}

impl From<&d::Notification> for FfiNotification {
    fn from(n: &d::Notification) -> Self {
        use d::Notification as N;
        match n {
            N::Completed {
                task_id,
                name,
                path,
            } => Self::Completed {
                task_id: task_id.0.clone(),
                name: name.clone(),
                path: path.clone(),
            },
            N::Failed {
                task_id,
                name,
                reason,
            } => Self::Failed {
                task_id: task_id.0.clone(),
                name: name.clone(),
                reason: reason.clone(),
            },
            N::Queued { task_id, name } => Self::Queued {
                task_id: task_id.0.clone(),
                name: name.clone(),
            },
            N::Scheduled { task_id, name, at } => Self::Scheduled {
                task_id: task_id.0.clone(),
                name: name.clone(),
                at: at.0,
            },
            N::ChecksumMismatch { task_id, name } => Self::ChecksumMismatch {
                task_id: task_id.0.clone(),
                name: name.clone(),
            },
            N::LowDiskSpace {
                path,
                free,
                required,
            } => Self::LowDiskSpace {
                path: path.clone(),
                free: *free,
                required: *required,
            },
            N::TorrentFinished { task_id, name } => Self::TorrentFinished {
                task_id: task_id.0.clone(),
                name: name.clone(),
            },
            N::DevicePaired { device_id, name } => Self::DevicePaired {
                device_id: device_id.0.clone(),
                name: name.clone(),
            },
            N::AutomationFailed {
                automation_id,
                name,
                error,
            } => Self::AutomationFailed {
                automation_id: automation_id.0.clone(),
                name: name.clone(),
                error: error.clone(),
            },
            N::DuplicateDetected {
                task_id,
                name,
                existing_path,
            } => Self::DuplicateDetected {
                task_id: task_id.0.clone(),
                name: name.clone(),
                existing_path: existing_path.clone(),
            },
            N::UpdateAvailable { version, notes_url } => Self::UpdateAvailable {
                version: version.clone(),
                notes_url: notes_url.clone(),
            },
            N::QueueFinished { queue_id, name } => Self::QueueFinished {
                queue_id: queue_id.0.clone(),
                name: name.clone(),
            },
        }
    }
}

/// Everything the app reacts to. Task payloads are lowered to `FfiTaskRow`; fetch the detail
/// with `task_detail` when the inspector needs it.
#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum FfiEvent {
    TaskAdded {
        row: FfiTaskRow,
    },
    TaskUpdated {
        row: FfiTaskRow,
    },
    TaskRemoved {
        id: String,
        deleted_file: bool,
    },
    TaskStateChanged {
        id: String,
        from: FfiTaskState,
        to: FfiTaskState,
        at: i64,
    },
    Progress {
        updates: Vec<FfiProgress>,
    },
    TaskLog {
        entry: FfiLogEntry,
    },
    QueueUpdated {
        queue: FfiQueue,
    },
    QueueRemoved {
        id: String,
    },
    QueueSummaries {
        summaries: Vec<FfiQueueSummary>,
    },
    CategoriesChanged,
    RulesChanged,
    SchedulesChanged,
    AutomationsChanged,
    AutomationRan {
        automation_id: String,
        task_id: Option<String>,
        success: bool,
        message: String,
    },
    RecipesChanged,
    SettingsChanged,
    GlobalStats {
        stats: FfiGlobalStats,
    },
    Notification {
        notification: FfiNotification,
    },
    DevicesChanged,
    PairingStarted {
        code: String,
        expires_at: i64,
        url: String,
    },
    PairingCompleted {
        device_name: String,
    },
    DiskSpace {
        path: String,
        free: u64,
    },
    NetworkChanged {
        available: bool,
        metered: bool,
    },
    /// Finder tag, reveal, open, AppleScript… — executed by the app.
    PlatformAction {
        task_id: Option<String>,
        action_json: String,
        context_json: String,
    },
    GrabberProgress {
        session_id: String,
        pages: u32,
        files: u32,
        done: bool,
    },
    UpdateCheck {
        available: bool,
        version: Option<String>,
        notes: Option<String>,
    },
    /// Sleep / quit requested by a schedule or queue completion action.
    ReadyForSleep {
        reason: String,
    },
    /// Any other engine-defined event (`schedule.launch_application`, plugin events…).
    Custom {
        name: String,
        payload_json: String,
    },
    /// The forwarder fell behind: reload `snapshot()`.
    Resync,
    EngineStopping,
}

/// Convert one bus event. `None` = not forwarded to the app.
pub fn convert(event: &Event) -> Option<FfiEvent> {
    Some(match event {
        Event::EngineStarted { .. } => return None,
        Event::EngineStopping => FfiEvent::EngineStopping,
        Event::TaskAdded(t) => FfiEvent::TaskAdded {
            row: t.as_ref().into(),
        },
        Event::TaskUpdated(t) => FfiEvent::TaskUpdated {
            row: t.as_ref().into(),
        },
        Event::TaskRemoved {
            task_id,
            deleted_file,
        } => FfiEvent::TaskRemoved {
            id: task_id.0.clone(),
            deleted_file: *deleted_file,
        },
        Event::TaskStateChanged {
            task_id,
            from,
            to,
            at,
        } => FfiEvent::TaskStateChanged {
            id: task_id.0.clone(),
            from: (*from).into(),
            to: (*to).into(),
            at: at.0,
        },
        Event::Progress(batch) => FfiEvent::Progress {
            updates: batch.iter().map(Into::into).collect(),
        },
        Event::TaskLog(l) => FfiEvent::TaskLog { entry: l.into() },
        Event::QueueUpdated(q) => FfiEvent::QueueUpdated { queue: q.into() },
        Event::QueueRemoved { queue_id } => FfiEvent::QueueRemoved {
            id: queue_id.0.clone(),
        },
        Event::QueueSummaries(s) => FfiEvent::QueueSummaries {
            summaries: s.iter().map(Into::into).collect(),
        },
        Event::CategoryUpdated(_) | Event::CategoryRemoved { .. } => FfiEvent::CategoriesChanged,
        Event::RuleUpdated(_) | Event::RuleRemoved { .. } => FfiEvent::RulesChanged,
        Event::ScheduleUpdated(_) | Event::ScheduleRemoved { .. } | Event::ScheduleFired { .. } => {
            FfiEvent::SchedulesChanged
        }
        Event::AutomationUpdated(_) | Event::AutomationRemoved { .. } => {
            FfiEvent::AutomationsChanged
        }
        Event::AutomationRan(r) => FfiEvent::AutomationRan {
            automation_id: r.automation_id.0.clone(),
            task_id: r.task_id.as_ref().map(|t| t.0.clone()),
            success: r.success,
            message: r.message.clone(),
        },
        Event::PlatformAction {
            task_id,
            action,
            context,
        } => FfiEvent::PlatformAction {
            task_id: task_id.as_ref().map(|t| t.0.clone()),
            action_json: serde_json::to_string(action).unwrap_or_default(),
            context_json: serde_json::to_string(context).unwrap_or_default(),
        },
        Event::SettingsChanged(_) => FfiEvent::SettingsChanged,
        Event::GlobalStats(g) => FfiEvent::GlobalStats { stats: g.into() },
        Event::Notification(n) => FfiEvent::Notification {
            notification: n.into(),
        },
        Event::DeviceUpdated(_) | Event::DeviceRemoved { .. } => FfiEvent::DevicesChanged,
        Event::PairingStarted {
            code,
            expires_at,
            url,
        } => FfiEvent::PairingStarted {
            code: code.clone(),
            expires_at: expires_at.0,
            url: url.clone(),
        },
        Event::PairingCompleted { device } => FfiEvent::PairingCompleted {
            device_name: device.name.clone(),
        },
        Event::DiskSpace { path, free } => FfiEvent::DiskSpace {
            path: path.clone(),
            free: *free,
        },
        Event::NetworkChanged { available, metered } => FfiEvent::NetworkChanged {
            available: *available,
            metered: *metered,
        },
        Event::Custom { name, payload } => custom(name, payload),
        Event::GrabberProgress {
            session_id,
            pages_crawled,
            files_found,
            done,
        } => FfiEvent::GrabberProgress {
            session_id: session_id.clone(),
            pages: *pages_crawled,
            files: *files_found,
            done: *done,
        },
        Event::UpdateCheck {
            available,
            version,
            notes,
        } => FfiEvent::UpdateCheck {
            available: *available,
            version: version.clone(),
            notes: notes.clone(),
        },
        Event::ReadyForSleep { reason } => FfiEvent::ReadyForSleep {
            reason: reason.clone(),
        },
    })
}

fn custom(name: &str, payload: &serde_json::Value) -> FfiEvent {
    let str_field = |k: &str| payload.get(k).and_then(|v| v.as_str()).map(str::to_owned);
    match name {
        "automation.notify" => FfiEvent::Notification {
            notification: FfiNotification::Custom {
                title: str_field("title").unwrap_or_default(),
                body: str_field("body").unwrap_or_default(),
                task_id: str_field("task_id"),
            },
        },
        "schedule.notify" => FfiEvent::Notification {
            notification: FfiNotification::Custom {
                title: "Osprey".to_owned(),
                body: str_field("message").unwrap_or_default(),
                task_id: None,
            },
        },
        _ => FfiEvent::Custom {
            name: name.to_owned(),
            payload_json: payload.to_string(),
        },
    }
}

/// Collapse high-volume events inside one batch: progress keeps the newest sample per task
/// (emitted where the last progress event was); stats and queue summaries keep only the last.
pub fn coalesce(batch: Vec<FfiEvent>) -> Vec<FfiEvent> {
    let progress_events = batch
        .iter()
        .filter(|e| matches!(e, FfiEvent::Progress { .. }))
        .count();
    let stats_events = batch
        .iter()
        .filter(|e| matches!(e, FfiEvent::GlobalStats { .. }))
        .count();
    let summary_events = batch
        .iter()
        .filter(|e| matches!(e, FfiEvent::QueueSummaries { .. }))
        .count();
    if progress_events <= 1 && stats_events <= 1 && summary_events <= 1 {
        return batch;
    }
    let mut latest: HashMap<String, FfiProgress> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(batch.len());
    let (mut seen_p, mut seen_s, mut seen_q) = (0, 0, 0);
    for e in batch {
        match e {
            FfiEvent::Progress { updates } => {
                seen_p += 1;
                for u in updates {
                    match latest.get(&u.task_id) {
                        Some(prev) if prev.rev > u.rev => {}
                        Some(_) => {
                            latest.insert(u.task_id.clone(), u);
                        }
                        None => {
                            order.push(u.task_id.clone());
                            latest.insert(u.task_id.clone(), u);
                        }
                    }
                }
                if seen_p == progress_events {
                    let updates = order
                        .drain(..)
                        .filter_map(|id| latest.remove(&id))
                        .collect();
                    out.push(FfiEvent::Progress { updates });
                }
            }
            e @ FfiEvent::GlobalStats { .. } => {
                seen_s += 1;
                if seen_s == stats_events {
                    out.push(e);
                }
            }
            e @ FfiEvent::QueueSummaries { .. } => {
                seen_q += 1;
                if seen_q == summary_events {
                    out.push(e);
                }
            }
            other => out.push(other),
        }
    }
    out
}

enum Incoming {
    Event(Box<FfiEvent>),
    Lagged,
    Closed,
}

async fn next(
    bus: &mut broadcast::Receiver<Arc<Event>>,
    local: &mut Option<broadcast::Receiver<FfiEvent>>,
) -> Incoming {
    loop {
        let from_bus = match local {
            Some(l) => tokio::select! {
                r = bus.recv() => Some(r),
                r = l.recv() => match r {
                    Ok(e) => return Incoming::Event(Box::new(e)),
                    Err(RecvError::Lagged(_)) => return Incoming::Lagged,
                    Err(RecvError::Closed) => {
                        // The engine object is gone; keep draining the bus until it closes.
                        *local = None;
                        None
                    }
                },
            },
            None => Some(bus.recv().await),
        };
        match from_bus {
            None => continue,
            Some(Ok(ev)) => {
                if let Some(e) = convert(&ev) {
                    return Incoming::Event(Box::new(e));
                }
            }
            Some(Err(RecvError::Lagged(_))) => return Incoming::Lagged,
            Some(Err(RecvError::Closed)) => return Incoming::Closed,
        }
    }
}

async fn deliver(listener: &Arc<dyn EventListener>, batch: Vec<FfiEvent>) {
    if batch.is_empty() {
        return;
    }
    let batch = coalesce(batch);
    let l = listener.clone();
    // The foreign callback runs on a blocking thread so a slow listener never stalls a runtime
    // worker; awaiting it keeps batches in order.
    if let Err(e) = tokio::task::spawn_blocking(move || l.on_events(batch)).await {
        tracing::warn!(error = %e, "event listener panicked");
    }
}

/// The per-listener forwarder loop. Ends when the bus closes or the task is aborted.
pub async fn forward(
    listener: Arc<dyn EventListener>,
    mut bus: broadcast::Receiver<Arc<Event>>,
    local: broadcast::Receiver<FfiEvent>,
) {
    let mut local = Some(local);
    loop {
        // Wait (without a deadline) for the first event of the next batch.
        let first = next(&mut bus, &mut local).await;
        let mut batch = Vec::new();
        let mut closed = false;
        match first {
            Incoming::Event(e) => batch.push(*e),
            Incoming::Lagged => batch.push(FfiEvent::Resync),
            Incoming::Closed => closed = true,
        }
        if !closed && !matches!(batch.last(), Some(FfiEvent::Resync)) {
            let deadline = tokio::time::Instant::now() + BATCH_WINDOW;
            while batch.len() < MAX_BATCH {
                match tokio::time::timeout_at(deadline, next(&mut bus, &mut local)).await {
                    Err(_) => break,
                    Ok(Incoming::Event(e)) => {
                        let e = *e;
                        let stop = matches!(e, FfiEvent::EngineStopping);
                        batch.push(e);
                        if stop {
                            break;
                        }
                    }
                    Ok(Incoming::Lagged) => {
                        batch.push(FfiEvent::Resync);
                        break;
                    }
                    Ok(Incoming::Closed) => {
                        closed = true;
                        break;
                    }
                }
            }
        }
        if matches!(batch.last(), Some(FfiEvent::Resync)) {
            // Everything before the gap is superseded by the snapshot the app reloads; drain
            // what is queued so the next batch starts fresh.
            batch.retain(|e| matches!(e, FfiEvent::Resync | FfiEvent::EngineStopping));
            bus = bus.resubscribe();
        }
        deliver(&listener, batch).await;
        if closed {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(id: &str, rev: u64, downloaded: u64) -> FfiProgress {
        FfiProgress {
            task_id: id.into(),
            rev,
            downloaded,
            uploaded: 0,
            total: None,
            fraction: 0.0,
            speed: 0,
            upload_speed: 0,
            eta_seconds: None,
            active_connections: 0,
            peers: 0,
            seeds: 0,
            ratio: 0.0,
        }
    }

    #[test]
    fn coalesces_progress_keeping_newest() {
        let out = coalesce(vec![
            FfiEvent::Progress {
                updates: vec![p("a", 1, 10), p("b", 1, 5)],
            },
            FfiEvent::RulesChanged,
            FfiEvent::Progress {
                updates: vec![p("a", 1, 20)],
            },
        ]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], FfiEvent::RulesChanged);
        match &out[1] {
            FfiEvent::Progress { updates } => {
                assert_eq!(updates.len(), 2);
                assert_eq!(updates[0].task_id, "a");
                assert_eq!(updates[0].downloaded, 20);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
