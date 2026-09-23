//! History queries and duplicate detection.

use crate::api::DuplicateInfo;
use crate::engine::Engine;
use osprey_domain::history::{HistoryEntry, HistoryQuery};
use osprey_domain::{DomainResult, Millis, Task, TaskId, TaskState};

impl Engine {
    /// Look for an existing copy of what `task` would download. Order: exact final path on
    /// disk → active/queued task with the same URL → history by URL → history by name+size.
    pub(crate) async fn detect_duplicate(&self, task: &Task) -> Option<DuplicateInfo> {
        if !self.settings().storage.duplicate_detection {
            return None;
        }
        let final_path = task.directory.join(&task.name);
        if let Ok(meta) = tokio::fs::metadata(&final_path).await {
            if meta.is_file() {
                return Some(DuplicateInfo {
                    matched_by: "path".into(),
                    existing_path: Some(final_path),
                    existing_task_id: None,
                    existing_size: Some(meta.len()),
                    existing_checksum: None,
                    existing_completed_at: None,
                });
            }
        }
        let url = task.source.primary_url().map(str::to_owned);
        if let Some(url) = &url {
            for other in self.tasks.all() {
                if other.id == task.id || other.state.is_terminal() {
                    continue;
                }
                if other.source.primary_url() == Some(url.as_str()) {
                    return Some(DuplicateInfo {
                        matched_by: "url_history".into(),
                        existing_path: other.file_path.clone(),
                        existing_task_id: Some(other.id.clone()),
                        existing_size: other.progress.total,
                        existing_checksum: None,
                        existing_completed_at: None,
                    });
                }
            }
            if let Ok(hits) = self.store.find_history_by_url(url).await {
                if let Some(h) = hits.into_iter().find(|h| h.state == TaskState::Completed) {
                    return Some(from_history("url_history", h));
                }
            }
        }
        if let Some(size) = task.progress.total {
            if let Ok(hits) = self.store.find_history_by_name_size(&task.name, size).await {
                if let Some(h) = hits.into_iter().find(|h| h.state == TaskState::Completed) {
                    return Some(from_history("size", h));
                }
            }
        }
        None
    }

    /// Build the history row for a finished task.
    pub(crate) fn history_entry_for(&self, task: &Task) -> HistoryEntry {
        let finished_at = task.completed_at.unwrap_or_else(Millis::now);
        let duration = task
            .started_at
            .map(|s| (finished_at.0 - s.0).max(0) as u64 / 1000)
            .unwrap_or(0);
        let average = if duration > 0 {
            task.progress.downloaded / duration
        } else {
            task.stats.average_speed
        };
        HistoryEntry {
            task_id: task.id.clone(),
            kind: task.kind,
            name: task.name.clone(),
            original_url: task.source.primary_url().unwrap_or("").to_owned(),
            final_url: task.stats.final_url.clone(),
            domain: task.domain().unwrap_or_default(),
            size: task.progress.total.or(Some(task.progress.downloaded)),
            checksum: task.verified_checksum.clone(),
            state: task.state,
            destination: task.target_path(),
            category_id: task.category_id.clone(),
            queue_id: task.queue_id.clone(),
            started_at: task.started_at,
            finished_at,
            duration_seconds: duration,
            average_speed: average,
            peak_speed: task.stats.peak_speed,
            error: task.error.as_ref().map(|e| e.message.clone()),
            tags: task.tags.clone(),
            origin: task.origin.clone(),
        }
    }

    pub(crate) async fn history_inner(
        &self,
        query: HistoryQuery,
    ) -> DomainResult<Vec<HistoryEntry>> {
        Ok(self.store.query_history(&query).await?)
    }

    pub(crate) async fn history_count_inner(&self, query: HistoryQuery) -> DomainResult<u32> {
        Ok(self.store.count_history(&query).await? as u32)
    }

    pub(crate) async fn delete_history_inner(&self, ids: Vec<TaskId>) -> DomainResult<u32> {
        Ok(self.store.delete_history(&ids).await? as u32)
    }

    pub(crate) async fn clear_history_inner(&self) -> DomainResult<u32> {
        Ok(self.store.clear_history().await? as u32)
    }

    /// Completed/failed counts and bytes since local midnight (for `GlobalStats`).
    pub(crate) async fn today_counters(&self) -> (u32, u32, u64) {
        let midnight = local_midnight();
        let q = HistoryQuery {
            since: Some(midnight),
            limit: 0,
            ..Default::default()
        };
        let Ok(entries) = self.store.query_history(&q).await else {
            return (0, 0, 0);
        };
        let mut completed = 0;
        let mut failed = 0;
        let mut bytes = 0;
        for e in entries {
            match e.state {
                TaskState::Completed => {
                    completed += 1;
                    bytes += e.size.unwrap_or(0);
                }
                TaskState::Failed => failed += 1,
                _ => {}
            }
        }
        (completed, failed, bytes)
    }
}

fn from_history(matched_by: &str, h: HistoryEntry) -> DuplicateInfo {
    DuplicateInfo {
        matched_by: matched_by.to_owned(),
        existing_path: Some(h.destination),
        existing_task_id: Some(h.task_id),
        existing_size: h.size,
        existing_checksum: h.checksum,
        existing_completed_at: Some(h.finished_at),
    }
}

/// Start of the current local day in unix millis.
pub fn local_midnight() -> Millis {
    use chrono::{Local, TimeZone};
    let now = Local::now();
    let date = now.date_naive();
    Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap_or_default())
        .earliest()
        .map(|d| Millis(d.timestamp_millis()))
        .unwrap_or_else(|| Millis(now.timestamp_millis() - 24 * 3600 * 1000))
}
