//! Disk monitoring: free space per destination, low-space notifications, volume presence
//! (ejected drives block tasks with `VolumeUnavailable` until they reappear).

use crate::api::DiskInfo;
use crate::engine::Engine;
use osprey_domain::state::PauseReason;
use osprey_domain::{DomainResult, Event, Millis, Notification, TaskState};
use osprey_runtime::disk::volume_stats;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Minimum interval between low-space notifications for the same path.
const LOW_SPACE_NOTIFY_INTERVAL_MS: i64 = 10 * 60_000;

/// State of the disk monitor.
#[derive(Default)]
pub struct DiskMonitor {
    last_notified: Mutex<HashMap<PathBuf, Millis>>,
    /// Directories currently considered unavailable.
    missing: Mutex<Vec<PathBuf>>,
}

/// Does the destination volume exist (its nearest existing ancestor is not the root)?
fn volume_present(dir: &Path) -> bool {
    if dir.exists() {
        return true;
    }
    // A directory that does not exist yet is fine as long as some non-root ancestor does
    // (we create it on start). `/Volumes/Name/...` with `/Volumes/Name` gone is not.
    let mut p = dir.parent();
    let mut depth = 0;
    while let Some(parent) = p {
        if parent.exists() {
            return depth < 3 || !parent.to_string_lossy().ends_with("/Volumes");
        }
        p = parent.parent();
        depth += 1;
    }
    false
}

impl Engine {
    /// Free-space and volume information for `path` (default download directory when `None`).
    pub(crate) fn disk_info_for(&self, path: Option<PathBuf>) -> DiskInfo {
        let path = path.unwrap_or_else(|| self.settings().storage.download_directory.clone());
        let (free, total) = volume_stats(&path)
            .map(|(f, t)| (Some(f), Some(t)))
            .unwrap_or((None, None));
        let required_by_active = self
            .tasks
            .all()
            .iter()
            .filter(|t| {
                t.state.is_active() && t.directory.starts_with(&path) || t.directory == path
            })
            .map(|t| {
                t.progress
                    .total
                    .unwrap_or(0)
                    .saturating_sub(t.progress.downloaded)
            })
            .sum();
        DiskInfo {
            volume_available: volume_present(&path),
            path,
            free,
            total,
            reserved: self.settings().storage.reserved_free_space,
            required_by_active,
        }
    }

    pub(crate) async fn disk_info_inner(&self, path: Option<PathBuf>) -> DomainResult<DiskInfo> {
        let this = self.this.clone();
        tokio::task::spawn_blocking(move || {
            this.upgrade()
                .map(|e| e.disk_info_for(path))
                .unwrap_or_default()
        })
        .await
        .map_err(osprey_domain::DomainError::internal)
    }

    /// The 30 s disk tick.
    pub(crate) async fn disk_tick(&self) {
        let settings = self.settings();
        let mut dirs: Vec<PathBuf> = vec![settings.storage.download_directory.clone()];
        let tasks = self.tasks.all();
        for t in &tasks {
            if !t.state.is_terminal() && !dirs.contains(&t.directory) {
                dirs.push(t.directory.clone());
            }
        }
        let reserved = settings.storage.reserved_free_space;
        let now = Millis::now();
        let probe_dirs = dirs.clone();
        let probes: Vec<(PathBuf, Option<u64>, bool)> = tokio::task::spawn_blocking(move || {
            probe_dirs
                .into_iter()
                .map(|d| {
                    let free = volume_stats(&d).map(|(f, _)| f);
                    (d.clone(), free, volume_present(&d))
                })
                .collect()
        })
        .await
        .unwrap_or_default();

        for (dir, free, present) in probes {
            if let Some(free) = free {
                self.bus.publish(Event::DiskSpace {
                    path: dir.to_string_lossy().to_string(),
                    free,
                });
                let required: u64 = tasks
                    .iter()
                    .filter(|t| t.directory == dir && t.state.is_active())
                    .map(|t| {
                        t.progress
                            .total
                            .unwrap_or(0)
                            .saturating_sub(t.progress.downloaded)
                    })
                    .sum();
                if free < reserved.saturating_add(required) && (required > 0 || free < reserved) {
                    let due = {
                        let mut last = self.disk.last_notified.lock();
                        let due = last
                            .get(&dir)
                            .map(|at| now.0 - at.0 >= LOW_SPACE_NOTIFY_INTERVAL_MS)
                            .unwrap_or(true);
                        if due {
                            last.insert(dir.clone(), now);
                        }
                        due
                    };
                    if due {
                        self.notify(Notification::LowDiskSpace {
                            path: dir.to_string_lossy().to_string(),
                            free,
                            required: reserved.saturating_add(required),
                        });
                    }
                }
            }
            // Volume presence → block / unblock.
            let was_missing = self.disk.missing.lock().contains(&dir);
            if !present && !was_missing {
                self.disk.missing.lock().push(dir.clone());
                for t in tasks.iter().filter(|t| t.directory == dir) {
                    if t.state.is_active() || t.state == TaskState::Queued {
                        if let Some(run) = self.tasks.run(&t.id) {
                            run.control
                                .volume_lost
                                .store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        self.block_task(&t.id, PauseReason::VolumeUnavailable).await;
                    }
                }
            } else if present && was_missing {
                self.disk.missing.lock().retain(|d| d != &dir);
                for t in tasks.iter().filter(|t| t.directory == dir) {
                    self.unblock_task(&t.id, &PauseReason::VolumeUnavailable)
                        .await;
                }
            }
            // DiskSpace blocks lift once there is room again.
            if let Some(free) = free {
                if free > reserved {
                    for t in tasks.iter().filter(|t| {
                        t.directory == dir && t.blocked_by.contains(&PauseReason::DiskSpace)
                    }) {
                        self.unblock_task(&t.id, &PauseReason::DiskSpace).await;
                    }
                }
            }
        }
    }
}
