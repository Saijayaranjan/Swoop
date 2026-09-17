//! Events emitted by the core. One stream feeds the macOS app (via FFI), the WebSocket API and
//! the automation engine. Progress is coalesced into batches by the runtime before it reaches
//! subscribers.

use crate::automation::AutomationRun;
use crate::category::Category;
use crate::device::Device;
use crate::queue::{Queue, QueueSummary};
use crate::rules::Rule;
use crate::schedule::Schedule;
use crate::{AutomationId, DeviceId, Millis, ProgressUpdate, QueueId, Task, TaskId, TaskState};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// A structured diagnostic line attached to a task (shown in the inspector's Events tab).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskLogEntry {
    pub task_id: TaskId,
    pub at: Millis,
    pub level: LogLevel,
    /// Stable machine-readable code (`http.probe`, `segment.split`, `retry.scheduled`, …).
    pub code: String,
    /// Already redacted.
    pub message: String,
}

/// Aggregate numbers refreshed ~1/s for the dashboard and menu bar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GlobalStats {
    pub download_speed: u64,
    pub upload_speed: u64,
    pub active: u32,
    pub downloading: u32,
    pub seeding: u32,
    pub queued: u32,
    pub scheduled: u32,
    pub paused: u32,
    pub completed_today: u32,
    pub failed_today: u32,
    pub total_tasks: u32,
    pub bytes_today: u64,
    pub free_space: Option<u64>,
    pub network_available: bool,
    pub traffic_mode: crate::queue::TrafficMode,
    pub download_limit: u64,
    pub upload_limit: u64,
    pub at: Millis,
}

/// Something the platform layer may surface as a user notification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Notification {
    Completed { task_id: TaskId, name: String, path: String },
    Failed { task_id: TaskId, name: String, reason: String },
    Queued { task_id: TaskId, name: String },
    Scheduled { task_id: TaskId, name: String, at: Millis },
    ChecksumMismatch { task_id: TaskId, name: String },
    LowDiskSpace { path: String, free: u64, required: u64 },
    TorrentFinished { task_id: TaskId, name: String },
    DevicePaired { device_id: DeviceId, name: String },
    AutomationFailed { automation_id: AutomationId, name: String, error: String },
    DuplicateDetected { task_id: TaskId, name: String, existing_path: String },
    UpdateAvailable { version: String, notes_url: Option<String> },
    QueueFinished { queue_id: QueueId, name: String },
}

/// The core event stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum Event {
    EngineStarted { version: String, at: Millis },
    EngineStopping,
    TaskAdded(Task),
    /// Full snapshot; sent on any non-progress change (state, options, name, path…).
    TaskUpdated(Task),
    TaskRemoved { task_id: TaskId, deleted_file: bool },
    TaskStateChanged { task_id: TaskId, from: TaskState, to: TaskState, at: Millis },
    /// Coalesced progress for many tasks (≤ 4 batches/second).
    Progress(Vec<ProgressUpdate>),
    TaskLog(TaskLogEntry),
    QueueUpdated(Queue),
    QueueRemoved { queue_id: QueueId },
    QueueSummaries(Vec<QueueSummary>),
    CategoryUpdated(Category),
    CategoryRemoved { category_id: crate::CategoryId },
    RuleUpdated(Rule),
    RuleRemoved { rule_id: crate::RuleId },
    ScheduleUpdated(Schedule),
    ScheduleRemoved { schedule_id: crate::ScheduleId },
    ScheduleFired { schedule_id: crate::ScheduleId, opened: bool },
    AutomationUpdated(crate::automation::AutomationRule),
    AutomationRemoved { automation_id: AutomationId },
    AutomationRan(AutomationRun),
    /// The core wants the platform layer to execute a platform-only action.
    PlatformAction { task_id: Option<TaskId>, action: crate::automation::AutomationAction, context: crate::automation::AutomationContext },
    SettingsChanged(Box<crate::settings::Settings>),
    GlobalStats(GlobalStats),
    Notification(Notification),
    DeviceUpdated(Device),
    DeviceRemoved { device_id: DeviceId },
    PairingStarted { code: String, expires_at: Millis, url: String },
    PairingCompleted { device: Device },
    DiskSpace { path: String, free: u64 },
    NetworkChanged { available: bool, metered: bool },
    /// Emitted by automation `EmitEvent` actions for external integrations.
    Custom { name: String, payload: serde_json::Value },
    GrabberProgress { session_id: String, pages_crawled: u32, files_found: u32, done: bool },
    UpdateCheck { available: bool, version: Option<String>, notes: Option<String> },
}

impl Event {
    /// Which task the event concerns, if any (used for per-task subscriptions).
    pub fn task_id(&self) -> Option<&TaskId> {
        match self {
            Event::TaskAdded(t) | Event::TaskUpdated(t) => Some(&t.id),
            Event::TaskRemoved { task_id, .. }
            | Event::TaskStateChanged { task_id, .. } => Some(task_id),
            Event::TaskLog(l) => Some(&l.task_id),
            _ => None,
        }
    }

    /// High-volume events that remote clients may opt out of.
    pub fn is_high_volume(&self) -> bool {
        matches!(self, Event::Progress(_) | Event::TaskLog(_) | Event::GlobalStats(_) | Event::QueueSummaries(_))
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Event::EngineStarted { .. } => "engine_started",
            Event::EngineStopping => "engine_stopping",
            Event::TaskAdded(_) => "task_added",
            Event::TaskUpdated(_) => "task_updated",
            Event::TaskRemoved { .. } => "task_removed",
            Event::TaskStateChanged { .. } => "task_state_changed",
            Event::Progress(_) => "progress",
            Event::TaskLog(_) => "task_log",
            Event::QueueUpdated(_) => "queue_updated",
            Event::QueueRemoved { .. } => "queue_removed",
            Event::QueueSummaries(_) => "queue_summaries",
            Event::CategoryUpdated(_) => "category_updated",
            Event::CategoryRemoved { .. } => "category_removed",
            Event::RuleUpdated(_) => "rule_updated",
            Event::RuleRemoved { .. } => "rule_removed",
            Event::ScheduleUpdated(_) => "schedule_updated",
            Event::ScheduleRemoved { .. } => "schedule_removed",
            Event::ScheduleFired { .. } => "schedule_fired",
            Event::AutomationUpdated(_) => "automation_updated",
            Event::AutomationRemoved { .. } => "automation_removed",
            Event::AutomationRan(_) => "automation_ran",
            Event::PlatformAction { .. } => "platform_action",
            Event::SettingsChanged(_) => "settings_changed",
            Event::GlobalStats(_) => "global_stats",
            Event::Notification(_) => "notification",
            Event::DeviceUpdated(_) => "device_updated",
            Event::DeviceRemoved { .. } => "device_removed",
            Event::PairingStarted { .. } => "pairing_started",
            Event::PairingCompleted { .. } => "pairing_completed",
            Event::DiskSpace { .. } => "disk_space",
            Event::NetworkChanged { .. } => "network_changed",
            Event::Custom { .. } => "custom",
            Event::GrabberProgress { .. } => "grabber_progress",
            Event::UpdateCheck { .. } => "update_check",
        }
    }
}
