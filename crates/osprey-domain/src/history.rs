//! Completed/failed task history — separate from live tasks so it survives "clear list".

use crate::{CategoryId, Checksum, Millis, QueueId, TaskId, TaskKind, TaskState};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub task_id: TaskId,
    pub kind: TaskKind,
    pub name: String,
    pub original_url: String,
    #[serde(default)]
    pub final_url: Option<String>,
    pub domain: String,
    pub size: Option<u64>,
    #[serde(default)]
    pub checksum: Option<Checksum>,
    pub state: TaskState,
    pub destination: PathBuf,
    #[serde(default)]
    pub category_id: Option<CategoryId>,
    pub queue_id: QueueId,
    pub started_at: Option<Millis>,
    pub finished_at: Millis,
    pub duration_seconds: u64,
    pub average_speed: u64,
    pub peak_speed: u64,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub origin: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryQuery {
    pub text: Option<String>,
    pub domain: Option<String>,
    pub state: Option<TaskState>,
    pub kind: Option<TaskKind>,
    pub category_id: Option<CategoryId>,
    pub queue_id: Option<QueueId>,
    pub since: Option<Millis>,
    pub until: Option<Millis>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub tag: Option<String>,
    pub sort: HistorySort,
    pub descending: bool,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistorySort {
    #[default]
    FinishedAt,
    Name,
    Size,
    Domain,
    Duration,
    Speed,
}
