//! Queues group tasks and bound their concurrency, bandwidth and schedule.

use crate::{Millis, QueueId, ScheduleId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Named bandwidth profiles selectable from the menu bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrafficMode {
    /// No limit at all.
    #[default]
    Unlimited,
    /// Alias for unlimited but with maximum connection counts (menu label "Full speed").
    FullSpeed,
    /// ~70 % of the measured link capacity.
    Balanced,
    /// Leaves headroom for browsing: ~30 % of link capacity or 2 MB/s, whichever is lower.
    Browsing,
    /// User-defined limits in `Settings::custom_limits`.
    Custom,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BandwidthProfile {
    /// Bytes per second; 0 = unlimited.
    pub download_limit: u64,
    pub upload_limit: u64,
    /// Connections per task inside this queue; 0 = global default.
    pub connections_per_task: u8,
}

/// What to do when the last task in the queue finishes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum QueueCompletionAction {
    #[default]
    Nothing,
    Notify,
    /// Run the named automation rule.
    RunAutomation { automation_id: String },
    /// Sleep the machine (macOS `pmset sleepnow` equivalent, executed by the app layer).
    Sleep,
    QuitApplication,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queue {
    pub id: QueueId,
    pub name: String,
    /// SF Symbol / icon name for the UI.
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub color: Option<String>,
    /// Max concurrently active tasks; 0 = unlimited.
    pub max_concurrent: u32,
    #[serde(default)]
    pub bandwidth: BandwidthProfile,
    #[serde(default)]
    pub schedule_id: Option<ScheduleId>,
    /// Default destination for tasks added to this queue.
    #[serde(default)]
    pub directory: Option<PathBuf>,
    /// Higher-priority queues get their slots first when the global limit is hit.
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub completion_action: QueueCompletionAction,
    /// A paused queue keeps its tasks `Queued` but never starts them.
    #[serde(default)]
    pub paused: bool,
    /// Built-in queues cannot be deleted.
    #[serde(default)]
    pub builtin: bool,
    /// Ordering in the sidebar.
    #[serde(default)]
    pub position: i32,
    pub created_at: Millis,
    pub updated_at: Millis,
}

impl Queue {
    pub fn new(name: impl Into<String>, max_concurrent: u32) -> Self {
        let now = Millis::now();
        Self {
            id: QueueId::new(),
            name: name.into(),
            icon: "tray.full".into(),
            color: None,
            max_concurrent,
            bandwidth: BandwidthProfile::default(),
            schedule_id: None,
            directory: None,
            priority: 0,
            completion_action: QueueCompletionAction::Nothing,
            paused: false,
            builtin: false,
            position: 0,
            created_at: now,
            updated_at: now,
        }
    }

    /// The queues created on first launch.
    pub fn builtin_defaults() -> Vec<Queue> {
        let mk = |id: &str, name: &str, icon: &str, max: u32, pos: i32, prio: i32| {
            let mut q = Queue::new(name, max);
            q.id = QueueId(id.to_owned());
            q.icon = icon.to_owned();
            q.builtin = true;
            q.position = pos;
            q.priority = prio;
            q
        };
        vec![
            mk("queue-default", "Default", "tray.full", 3, 0, 0),
            mk("queue-high", "High priority", "bolt", 2, 1, 10),
            mk("queue-video", "Video", "film", 2, 2, 0),
            mk("queue-torrent", "Torrents", "network", 5, 3, -5),
            mk("queue-night", "Night", "moon.stars", 4, 4, 0),
        ]
    }

    /// Aggregate counts kept in memory by the services layer, sent to the UI with the queue.
    pub fn summary(&self) -> QueueSummary {
        QueueSummary { queue_id: self.id.clone(), active: 0, waiting: 0, completed: 0, failed: 0, download_speed: 0, upload_speed: 0 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct QueueSummary {
    pub queue_id: QueueId,
    pub active: u32,
    pub waiting: u32,
    pub completed: u32,
    pub failed: u32,
    pub download_speed: u64,
    pub upload_speed: u64,
}
