//! Torrent controls: file selection, sequential mode, seeding limits, trackers, peers — all
//! delegated to the torrent engine, mirrored into the task's `torrent` info and checkpoint.

use crate::api::FileSelection;
use crate::engine::Engine;
use crate::persist::PersistOp;
use osprey_domain::checkpoint::{Checkpoint, TorrentCheckpoint};
use osprey_domain::torrent::{PeerInfo, SeedingLimits};
use osprey_domain::{DomainError, DomainResult, Event, Task, TaskId};
use osprey_engine_torrent::TorrentBlobProvider;
use osprey_runtime::engine::FileSelectionUpdate;
use osprey_store::Store;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// `TorrentBlobProvider` over the store's `torrent_blobs` table.
pub struct StoreBlobs {
    pub store: Arc<Store>,
}

#[async_trait::async_trait]
impl TorrentBlobProvider for StoreBlobs {
    async fn torrent_bytes(&self, info_hash: &str) -> Option<Vec<u8>> {
        self.store.get_torrent_blob(info_hash).await.ok().flatten()
    }
}

impl Engine {
    fn torrent_task(&self, id: &TaskId) -> DomainResult<Task> {
        let t = self.snapshot(id)?;
        if !t.kind.is_torrent() {
            return Err(DomainError::validation(format!(
                "task {id} is not a torrent"
            )));
        }
        Ok(t)
    }

    /// Merge the engine's live view into the task and persist it.
    async fn refresh_torrent_info(&self, id: &TaskId) -> DomainResult<Task> {
        let live = self.torrent.torrent_info(id);
        let s = self.mutate(id, |t| {
            if let Some(info) = live {
                t.torrent = Some(info);
            }
            Ok(())
        })?;
        self.persist
            .commit(PersistOp::Update(Box::new(s.clone())))
            .await?;
        self.bus.publish(Event::TaskUpdated(Box::new(s.clone())));
        Ok(s)
    }

    /// Upsert the torrent checkpoint from the task's current view.
    async fn save_torrent_checkpoint(&self, task: &Task) {
        let Some(info) = &task.torrent else {
            return;
        };
        let existing = self
            .store
            .get_checkpoint(&task.id)
            .await
            .ok()
            .flatten()
            .and_then(|c| c.as_torrent().cloned())
            .unwrap_or_default();
        let selected: Vec<u32> = info
            .files
            .iter()
            .filter(|f| f.selected)
            .map(|f| f.index)
            .collect();
        let all_selected = selected.len() == info.files.len();
        let cp = TorrentCheckpoint {
            info_hash: info.info_hash.clone(),
            selected_files: if all_selected { None } else { Some(selected) },
            priorities: info.files.iter().map(|f| (f.index, f.priority)).collect(),
            uploaded: info.uploaded.max(existing.uploaded),
            downloaded: task.progress.downloaded,
            seeding_since: info.seeding_since.or(existing.seeding_since),
            sequential: task.options.sequential.unwrap_or(existing.sequential),
            output_folder: task
                .file_path
                .clone()
                .unwrap_or_else(|| task.directory.clone()),
            disabled_trackers: info
                .trackers
                .iter()
                .filter(|t| !t.enabled)
                .map(|t| t.url.clone())
                .collect(),
            extra_trackers: existing.extra_trackers,
        };
        let _ = self
            .persist
            .commit(PersistOp::Checkpoint(
                task.id.clone(),
                Box::new(Checkpoint::Torrent(cp)),
            ))
            .await;
    }

    pub(crate) async fn set_torrent_files_inner(
        &self,
        id: TaskId,
        selection: Vec<FileSelection>,
    ) -> DomainResult<Task> {
        self.torrent_task(&id)?;
        let s = self.mutate(&id, |t| {
            if let Some(info) = &mut t.torrent {
                for sel in &selection {
                    if let Some(f) = info.files.iter_mut().find(|f| f.index == sel.index) {
                        f.selected = sel.selected;
                        f.priority = sel.priority.min(2);
                    }
                }
            }
            Ok(())
        })?;
        if let Some(run) = self.tasks.run(&id) {
            run.control.set_file_selection(
                selection
                    .iter()
                    .map(|s| FileSelectionUpdate {
                        index: s.index,
                        selected: s.selected,
                        priority: s.priority.min(2),
                    })
                    .collect(),
            );
        }
        self.persist
            .commit(PersistOp::Update(Box::new(s.clone())))
            .await?;
        self.save_torrent_checkpoint(&s).await;
        self.bus.publish(Event::TaskUpdated(Box::new(s.clone())));
        Ok(s)
    }

    pub(crate) async fn set_torrent_sequential_inner(
        &self,
        id: TaskId,
        sequential: bool,
    ) -> DomainResult<Task> {
        self.torrent_task(&id)?;
        let s = self.mutate(&id, |t| {
            t.options.sequential = Some(sequential);
            Ok(())
        })?;
        if let Some(run) = self.tasks.run(&id) {
            run.control.sequential.store(sequential, Ordering::Relaxed);
        }
        self.persist
            .commit(PersistOp::Update(Box::new(s.clone())))
            .await?;
        self.save_torrent_checkpoint(&s).await;
        self.bus.publish(Event::TaskUpdated(Box::new(s.clone())));
        Ok(s)
    }

    pub(crate) async fn set_seeding_limits_inner(
        &self,
        id: TaskId,
        limits: SeedingLimits,
    ) -> DomainResult<Task> {
        self.torrent_task(&id)?;
        self.torrent.set_seeding_limits(&id, limits.clone());
        let s = self.mutate(&id, |t| {
            if limits.ratio_limit.is_some() {
                t.options.seed_ratio_limit = limits.ratio_limit;
            }
            if limits.time_limit_minutes.is_some() {
                t.options.seed_time_limit_minutes = limits.time_limit_minutes;
            }
            if limits.upload_limit.is_some() {
                t.options.upload_limit = limits.upload_limit;
            }
            Ok(())
        })?;
        if let (Some(run), Some(up)) = (self.tasks.run(&id), limits.upload_limit) {
            run.control.upload_limit.store(up, Ordering::Relaxed);
            run.upload_limiter.set_limit(up);
        }
        self.persist
            .commit(PersistOp::Update(Box::new(s.clone())))
            .await?;
        self.bus.publish(Event::TaskUpdated(Box::new(s.clone())));
        Ok(s)
    }

    pub(crate) async fn torrent_peers_inner(&self, id: TaskId) -> DomainResult<Vec<PeerInfo>> {
        self.torrent_task(&id)?;
        Ok(self.torrent.peers(&id))
    }

    fn map_torrent_err(e: osprey_domain::TaskError) -> DomainError {
        match e.kind {
            osprey_domain::ErrorKind::NotFound => DomainError::Unavailable(e.message),
            osprey_domain::ErrorKind::PermissionDenied => DomainError::PermissionDenied(e.message),
            _ => DomainError::Engine(e.message),
        }
    }

    pub(crate) async fn add_trackers_inner(
        &self,
        id: TaskId,
        trackers: Vec<String>,
    ) -> DomainResult<Task> {
        self.torrent_task(&id)?;
        self.torrent
            .add_trackers(&id, trackers.clone())
            .map_err(Self::map_torrent_err)?;
        let s = self.refresh_torrent_info(&id).await?;
        let mut existing = self
            .store
            .get_checkpoint(&id)
            .await
            .ok()
            .flatten()
            .and_then(|c| c.as_torrent().cloned())
            .unwrap_or_default();
        for t in trackers {
            if !existing.extra_trackers.contains(&t) {
                existing.extra_trackers.push(t);
            }
        }
        let _ = self
            .persist
            .commit(PersistOp::Checkpoint(
                id.clone(),
                Box::new(Checkpoint::Torrent(existing)),
            ))
            .await;
        self.save_torrent_checkpoint(&s).await;
        Ok(s)
    }

    pub(crate) async fn remove_tracker_inner(
        &self,
        id: TaskId,
        tracker: String,
    ) -> DomainResult<Task> {
        self.torrent_task(&id)?;
        self.torrent
            .remove_tracker(&id, &tracker)
            .map_err(Self::map_torrent_err)?;
        let s = self.refresh_torrent_info(&id).await?;
        self.save_torrent_checkpoint(&s).await;
        Ok(s)
    }

    pub(crate) async fn set_tracker_enabled_inner(
        &self,
        id: TaskId,
        tracker: String,
        enabled: bool,
    ) -> DomainResult<Task> {
        self.torrent_task(&id)?;
        self.torrent
            .set_tracker_enabled(&id, &tracker, enabled)
            .map_err(Self::map_torrent_err)?;
        let s = self.refresh_torrent_info(&id).await?;
        self.save_torrent_checkpoint(&s).await;
        Ok(s)
    }

    pub(crate) async fn reannounce_inner(&self, id: TaskId) -> DomainResult<()> {
        self.torrent_task(&id)?;
        self.torrent.reannounce(&id).map_err(Self::map_torrent_err)
    }

    pub(crate) async fn refresh_tracker_list_inner(&self) -> DomainResult<u32> {
        let settings = self.settings();
        let Some(url) = settings.torrent.tracker_source_url.clone() else {
            return Err(DomainError::validation("no tracker list URL configured"));
        };
        self.torrent
            .refresh_tracker_list(&url)
            .await
            .map_err(Self::map_torrent_err)
    }
}
