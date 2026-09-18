//! Startup recovery: reconcile every persisted task with what is actually on disk, following
//! the recovery table in `docs/architecture/002-persistence-and-recovery.md`.
//!
//! | Persisted state | Part file | Action |
//! |---|---|---|
//! | Pending / Queued / Scheduled | any | keep |
//! | Resolving / Connecting / Downloading / Retrying | present | `Queued`, checkpoint kept |
//! | Resolving / Connecting / Downloading / Retrying | missing | `Queued`, checkpoint discarded |
//! | Paused | any | keep (`blocked_by` preserved) |
//! | Verifying | part present | stays `Verifying` (re-hash) |
//! | Verifying / Processing | part missing, final file has `total` bytes | reported in `ready_to_complete` |
//! | Processing | part (dir) present | stays `Processing` (merge is idempotent) |
//! | Completed / Failed / Cancelled | — | keep |
//! | Seeding | — | `Resolving` (via `Paused`, the only legal path) |
//! | Torrent in any active state | — | reported in `torrents_to_readd` |
//!
//! Two situations the table does not name are handled conservatively: a `Verifying` or
//! `Processing` task whose part file *and* final file are both gone is re-queued from scratch
//! (`Verifying → Retrying → Queued`, the legal path), and torrent tasks — whose data librqbit
//! writes directly into the destination — are treated as "part present".

use crate::error::{StoreError, StoreResult};
use crate::tasks::{delete_checkpoint_row, load_all_on, update_state_row, upsert_progress_row};
use crate::Store;
use osprey_domain::state::PauseReason;
use osprey_domain::{Checkpoint, Millis, Progress, Task, TaskId, TaskState};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What recovery decided for one task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    /// Nothing changed.
    Kept,
    /// Moved to `Queued`; the checkpoint is intact and the engine resumes from it.
    RequeuedWithCheckpoint,
    /// Moved to `Queued`; the checkpoint and progress were discarded.
    RequeuedFromScratch,
    /// Left in `Verifying`; the engine re-hashes the part file.
    Reverify,
    /// Left in `Processing`; post-processing runs again.
    Reprocess,
    /// The final file is complete on disk; the services layer must run the completion
    /// transaction (history + checksum).
    ReadyToComplete,
    /// Torrent moved from `Seeding` to `Resolving` for re-adding to the session.
    Reseed,
    /// A transition failed; the task was left untouched.
    Failed(String),
}

/// Outcome of [`Store::recover`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RecoveryReport {
    /// Tasks inspected.
    pub scanned: usize,
    /// Tasks left exactly as persisted.
    pub kept: usize,
    /// Active tasks re-queued with their checkpoint intact.
    pub requeued_with_checkpoint: Vec<TaskId>,
    /// Active tasks re-queued after their partial data was found missing.
    pub requeued_from_scratch: Vec<TaskId>,
    /// Tasks that will verify again.
    pub reverifying: Vec<TaskId>,
    /// Tasks that will post-process again.
    pub reprocessing: Vec<TaskId>,
    /// Tasks whose final file is complete; the caller runs [`Store::complete_task`].
    pub ready_to_complete: Vec<TaskId>,
    /// Seeding torrents moved to `Resolving`.
    pub reseeding: Vec<TaskId>,
    /// Torrent tasks in an active state that must be re-added to the librqbit session (from
    /// `torrent_blobs` when the session entry is missing).
    pub torrents_to_readd: Vec<TaskId>,
    /// Paused tasks still carrying a `Shutdown` block (the services layer decides whether to
    /// lift it).
    pub paused_by_shutdown: Vec<TaskId>,
    /// Tasks whose recovery transition failed, with the reason.
    pub failed: Vec<(TaskId, String)>,
    /// The post-recovery snapshot of every task (progress + segment map merged), so the caller
    /// does not need a second load.
    pub tasks: Vec<Task>,
}

/// What was found on disk for one task.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DiskState {
    /// The part file/directory exists (always `true` for torrents — librqbit owns their data).
    pub part_present: bool,
    /// Size of the final file, if it exists.
    pub final_size: Option<u64>,
}

/// Where a task's partial data lives.
pub(crate) fn part_path(
    task: &Task,
    checkpoint: Option<&Checkpoint>,
    temp_suffix: &str,
) -> PathBuf {
    match checkpoint {
        Some(Checkpoint::Segments(m)) if m.part_path.is_some() => {
            m.part_path.clone().unwrap_or_default()
        }
        Some(Checkpoint::Hls(h)) if !h.part_dir.as_os_str().is_empty() => h.part_dir.clone(),
        _ => task.directory.join(format!("{}{}", task.name, temp_suffix)),
    }
}

fn inspect_disk(task: &Task, checkpoint: Option<&Checkpoint>, temp_suffix: &str) -> DiskState {
    let part_present = task.kind.is_torrent() || part_path(task, checkpoint, temp_suffix).exists();
    let final_size = std::fs::metadata(task.target_path())
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len());
    DiskState {
        part_present,
        final_size,
    }
}

/// Expected final size, from progress or the checkpoint.
fn known_total(task: &Task, checkpoint: Option<&Checkpoint>) -> Option<u64> {
    task.progress.total.or(match checkpoint {
        Some(Checkpoint::Segments(m)) => m.total,
        _ => None,
    })
}

/// Apply the recovery table to one task in memory. Returns the action and whether the
/// checkpoint must be discarded.
pub(crate) fn apply_rules(
    task: &mut Task,
    checkpoint: Option<&Checkpoint>,
    disk: &DiskState,
) -> (RecoveryAction, bool) {
    use TaskState::*;
    let final_complete = matches!(
        (known_total(task, checkpoint), disk.final_size),
        (Some(t), Some(f)) if t == f
    );
    let mut discard = false;
    let requeue = |task: &mut Task, via_retrying: bool| -> Result<(), String> {
        if via_retrying {
            task.transition(Retrying).map_err(|e| e.to_string())?;
        }
        task.transition(Queued).map_err(|e| e.to_string())?;
        Ok(())
    };
    let action = match task.state {
        Pending | Queued | Scheduled | Completed | Failed | Cancelled | Paused => {
            RecoveryAction::Kept
        }
        Resolving | Connecting | Downloading | Retrying => {
            if disk.part_present {
                match requeue(task, false) {
                    Ok(()) => RecoveryAction::RequeuedWithCheckpoint,
                    Err(e) => RecoveryAction::Failed(e),
                }
            } else {
                match requeue(task, false) {
                    Ok(()) => {
                        discard = true;
                        RecoveryAction::RequeuedFromScratch
                    }
                    Err(e) => RecoveryAction::Failed(e),
                }
            }
        }
        Verifying | Processing => {
            if disk.part_present {
                // the engine restarts the step; only the shutdown marker must go
                task.unblock(&PauseReason::Shutdown);
                if task.state == Verifying {
                    RecoveryAction::Reverify
                } else {
                    RecoveryAction::Reprocess
                }
            } else if final_complete {
                RecoveryAction::ReadyToComplete
            } else {
                match requeue(task, true) {
                    Ok(()) => {
                        discard = true;
                        RecoveryAction::RequeuedFromScratch
                    }
                    Err(e) => RecoveryAction::Failed(e),
                }
            }
        }
        Seeding => {
            let r = task
                .transition(Paused)
                .and_then(|_| task.transition(Resolving));
            match r {
                Ok(_) => RecoveryAction::Reseed,
                Err(e) => RecoveryAction::Failed(e.to_string()),
            }
        }
    };
    if discard {
        task.segment_map = None;
        task.progress = Progress {
            total: task.progress.total,
            ..Progress::default()
        };
        if let Some(m) = &mut task.media {
            m.segments_done = 0;
        }
    }
    (action, discard)
}

impl Store {
    /// Reconcile persisted tasks with the filesystem (see the module docs), persist the
    /// resulting states in one transaction and return what was done. `temp_suffix` is
    /// `Settings::storage.temp_suffix` (`.osprey-part`). Run before engines start.
    pub async fn recover(&self, temp_suffix: &str) -> StoreResult<RecoveryReport> {
        let suffix = temp_suffix.to_owned();
        let loaded = self.read(load_all_on).await?;
        // Filesystem probes are blocking; keep them off the async workers.
        let planned = tokio::task::spawn_blocking(move || {
            let mut report = RecoveryReport {
                scanned: loaded.len(),
                ..Default::default()
            };
            let mut changed: Vec<(Task, bool)> = Vec::new();
            let mut tasks = Vec::with_capacity(loaded.len());
            for (mut task, checkpoint) in loaded {
                let disk = inspect_disk(&task, checkpoint.as_ref(), &suffix);
                let before_rev = task.rev;
                let was_active = task.state.is_active();
                let (action, discard) = apply_rules(&mut task, checkpoint.as_ref(), &disk);
                let id = task.id.clone();
                if task.kind.is_torrent() && was_active {
                    report.torrents_to_readd.push(id.clone());
                }
                if task.state == TaskState::Paused
                    && task.blocked_by.contains(&PauseReason::Shutdown)
                {
                    report.paused_by_shutdown.push(id.clone());
                }
                tracing::debug!(task = %id, state = %task.state, ?action, ?disk, "recovery");
                match &action {
                    RecoveryAction::Kept => report.kept += 1,
                    RecoveryAction::RequeuedWithCheckpoint => {
                        report.requeued_with_checkpoint.push(id)
                    }
                    RecoveryAction::RequeuedFromScratch => report.requeued_from_scratch.push(id),
                    RecoveryAction::Reverify => report.reverifying.push(id),
                    RecoveryAction::Reprocess => report.reprocessing.push(id),
                    RecoveryAction::ReadyToComplete => report.ready_to_complete.push(id),
                    RecoveryAction::Reseed => report.reseeding.push(id),
                    RecoveryAction::Failed(e) => report.failed.push((id, e.clone())),
                }
                if task.rev != before_rev || discard {
                    changed.push((task.clone(), discard));
                }
                tasks.push(task);
            }
            report.tasks = tasks;
            (report, changed)
        })
        .await
        .map_err(|e| StoreError::Internal(format!("recovery task failed: {e}")))?;
        let (report, changed) = planned;
        if !changed.is_empty() {
            self.write("recover", move |conn| {
                let now = Millis::now();
                for (task, discard) in &changed {
                    update_state_row(conn, task)?;
                    if *discard {
                        delete_checkpoint_row(conn, &task.id)?;
                        upsert_progress_row(conn, &task.id, &task.progress, now)?;
                    }
                }
                Ok(())
            })
            .await?;
        }
        tracing::info!(
            scanned = report.scanned,
            kept = report.kept,
            requeued = report.requeued_with_checkpoint.len() + report.requeued_from_scratch.len(),
            ready_to_complete = report.ready_to_complete.len(),
            failed = report.failed.len(),
            "recovery applied"
        );
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use osprey_domain::{QueueId, SegmentMap, Source, TaskKind};

    fn task(state: TaskState) -> Task {
        let mut t = Task::new(
            TaskKind::Http,
            Source::Urls {
                urls: vec!["https://example.com/a.bin".into()],
            },
            "a.bin",
            PathBuf::from("/tmp/x"),
            QueueId::default_queue(),
        );
        // walk to the requested state through legal transitions
        let path: &[TaskState] = match state {
            TaskState::Pending => &[],
            TaskState::Queued => &[TaskState::Queued],
            TaskState::Scheduled => &[TaskState::Scheduled],
            TaskState::Resolving => &[TaskState::Queued, TaskState::Resolving],
            TaskState::Connecting => &[TaskState::Queued, TaskState::Connecting],
            TaskState::Downloading => &[
                TaskState::Queued,
                TaskState::Connecting,
                TaskState::Downloading,
            ],
            TaskState::Paused => &[TaskState::Queued, TaskState::Paused],
            TaskState::Retrying => &[
                TaskState::Queued,
                TaskState::Connecting,
                TaskState::Retrying,
            ],
            TaskState::Verifying => &[
                TaskState::Queued,
                TaskState::Connecting,
                TaskState::Downloading,
                TaskState::Verifying,
            ],
            TaskState::Processing => &[
                TaskState::Queued,
                TaskState::Connecting,
                TaskState::Downloading,
                TaskState::Processing,
            ],
            TaskState::Completed => &[
                TaskState::Queued,
                TaskState::Connecting,
                TaskState::Downloading,
                TaskState::Completed,
            ],
            TaskState::Failed => &[TaskState::Failed],
            TaskState::Cancelled => &[TaskState::Cancelled],
            TaskState::Seeding => &[
                TaskState::Queued,
                TaskState::Connecting,
                TaskState::Downloading,
                TaskState::Seeding,
            ],
        };
        for s in path {
            t.transition(*s).expect("legal path");
        }
        t
    }

    #[test]
    fn active_with_part_keeps_checkpoint() {
        for s in [
            TaskState::Resolving,
            TaskState::Connecting,
            TaskState::Downloading,
            TaskState::Retrying,
        ] {
            let mut t = task(s);
            t.block(PauseReason::Shutdown);
            let (a, discard) = apply_rules(
                &mut t,
                None,
                &DiskState {
                    part_present: true,
                    final_size: None,
                },
            );
            assert_eq!(a, RecoveryAction::RequeuedWithCheckpoint);
            assert!(!discard);
            assert_eq!(t.state, TaskState::Queued);
            assert!(t.blocked_by.is_empty());
        }
    }

    #[test]
    fn active_without_part_starts_over() {
        let mut t = task(TaskState::Downloading);
        t.segment_map = Some(SegmentMap::default());
        t.progress.downloaded = 10;
        let (a, discard) = apply_rules(&mut t, None, &DiskState::default());
        assert_eq!(a, RecoveryAction::RequeuedFromScratch);
        assert!(discard && t.segment_map.is_none() && t.progress.downloaded == 0);
    }

    #[test]
    fn verifying_and_processing_rules() {
        let mut t = task(TaskState::Verifying);
        assert_eq!(
            apply_rules(
                &mut t,
                None,
                &DiskState {
                    part_present: true,
                    final_size: None
                }
            )
            .0,
            RecoveryAction::Reverify
        );
        let mut t = task(TaskState::Processing);
        assert_eq!(
            apply_rules(
                &mut t,
                None,
                &DiskState {
                    part_present: true,
                    final_size: None
                }
            )
            .0,
            RecoveryAction::Reprocess
        );
        let mut t = task(TaskState::Verifying);
        t.progress.total = Some(42);
        assert_eq!(
            apply_rules(
                &mut t,
                None,
                &DiskState {
                    part_present: false,
                    final_size: Some(42)
                }
            )
            .0,
            RecoveryAction::ReadyToComplete
        );
        assert_eq!(t.state, TaskState::Verifying);
        let mut t = task(TaskState::Processing);
        let (a, discard) = apply_rules(&mut t, None, &DiskState::default());
        assert_eq!(a, RecoveryAction::RequeuedFromScratch);
        assert!(discard && t.state == TaskState::Queued);
    }

    #[test]
    fn seeding_goes_to_resolving_and_terminal_states_are_kept() {
        let mut t = task(TaskState::Seeding);
        assert_eq!(
            apply_rules(&mut t, None, &DiskState::default()).0,
            RecoveryAction::Reseed
        );
        assert_eq!(t.state, TaskState::Resolving);
        for s in [
            TaskState::Pending,
            TaskState::Queued,
            TaskState::Scheduled,
            TaskState::Paused,
            TaskState::Completed,
            TaskState::Failed,
            TaskState::Cancelled,
        ] {
            let mut t = task(s);
            let rev = t.rev;
            assert_eq!(
                apply_rules(&mut t, None, &DiskState::default()).0,
                RecoveryAction::Kept
            );
            assert_eq!(t.state, s);
            assert_eq!(t.rev, rev);
        }
    }

    #[test]
    fn part_path_prefers_checkpoint() {
        let t = task(TaskState::Downloading);
        assert_eq!(
            part_path(&t, None, ".osprey-part"),
            PathBuf::from("/tmp/x/a.bin.osprey-part")
        );
        let cp = Checkpoint::Segments(SegmentMap {
            part_path: Some(PathBuf::from("/elsewhere/p")),
            ..Default::default()
        });
        assert_eq!(
            part_path(&t, Some(&cp), ".osprey-part"),
            PathBuf::from("/elsewhere/p")
        );
    }
}
