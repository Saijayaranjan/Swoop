//! User-facing task operations: start, pause, resume, restart, retry, cancel, remove, edit,
//! reorder, verify and the bulk variants.

use crate::api::{TaskFilter, TaskPatch, TaskRow, TaskSort};
use crate::engine::Engine;
use crate::persist::PersistOp;
use osprey_domain::state::PauseReason;
use osprey_domain::{
    Checksum, DomainError, DomainResult, ErrorKind, Event, Priority, Source, Task, TaskError,
    TaskId, TaskKind, TaskState,
};
use osprey_runtime::engine::Transfer;
use osprey_runtime::paths::AppPaths;
use osprey_runtime::safety::{is_strictly_within, sanitize_filename, validate_destination_dir};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// How long user operations wait for a running engine to acknowledge a pause/cancel.
const STOP_WAIT: Duration = Duration::from_secs(10);

impl Engine {
    pub(crate) async fn start_task_inner(&self, id: TaskId) -> DomainResult<Task> {
        let state = self.snapshot(&id)?.state;
        match state {
            TaskState::Pending | TaskState::Paused | TaskState::Failed | TaskState::Cancelled => {
                self.enqueue(&id).await?;
            }
            TaskState::Scheduled => {
                // Explicit start overrides the schedule gate for this task.
                let snap = self.mutate(&id, |t| {
                    t.blocked_by
                        .retain(|r| !matches!(r, PauseReason::Schedule(_)));
                    if t.blocked_by.is_empty() {
                        t.transition(TaskState::Queued)?;
                    }
                    Ok(())
                })?;
                self.persist
                    .commit(PersistOp::State(Box::new(snap.clone())))
                    .await?;
                self.publish_state(&snap, TaskState::Scheduled);
                self.admission.notify_one();
            }
            TaskState::Queued | TaskState::Retrying => {
                let snap = self.mutate(&id, |t| {
                    t.next_retry_at = None;
                    Ok(())
                })?;
                self.persist.send(PersistOp::State(Box::new(snap)));
                self.admission.notify_one();
            }
            _ => {}
        }
        self.snapshot(&id)
    }

    pub(crate) async fn pause_task_inner(&self, id: TaskId) -> DomainResult<Task> {
        let snap = self.snapshot(&id)?;
        if !snap.state.can_pause() && snap.state != TaskState::Pending {
            return Err(DomainError::InvalidTransition(format!(
                "task {id} cannot be paused from {}",
                snap.state
            )));
        }
        let run = self.tasks.run(&id);
        self.block_task(&id, PauseReason::User).await;
        if let Some(run) = run {
            run.wait_finished(STOP_WAIT).await;
        }
        self.snapshot(&id)
    }

    pub(crate) async fn resume_task_inner(&self, id: TaskId) -> DomainResult<Task> {
        let snap = self.snapshot(&id)?;
        match snap.state {
            TaskState::Paused => {
                self.unblock_task(&id, &PauseReason::User).await;
                let after = self.snapshot(&id)?;
                if after.state == TaskState::Paused && after.blocked_by.is_empty() {
                    self.enqueue(&id).await?;
                }
            }
            TaskState::Failed | TaskState::Cancelled | TaskState::Pending => {
                self.enqueue(&id).await?;
            }
            _ => {}
        }
        self.snapshot(&id)
    }

    /// Stop a running task (pause its engine) and wait for the outcome.
    async fn stop_run(&self, id: &TaskId, cancel: bool) {
        if let Some(run) = self.tasks.run(id) {
            if cancel {
                run.control.cancel.cancel();
            } else {
                run.control.pause.cancel();
            }
            run.wait_finished(STOP_WAIT).await;
        }
    }

    pub(crate) async fn cancel_task_inner(&self, id: TaskId) -> DomainResult<Task> {
        let snap = self.snapshot(&id)?;
        if snap.state.is_terminal() && snap.state != TaskState::Failed {
            return Ok(snap);
        }
        if self.tasks.run(&id).is_some() {
            self.stop_run(&id, true).await;
        }
        let after = self.snapshot(&id)?;
        if after.state != TaskState::Cancelled {
            let (s, from) = self.mutate_from(&id, |t| {
                if t.state.can_transition_to(TaskState::Cancelled) {
                    t.transition(TaskState::Cancelled)?;
                }
                Ok(())
            })?;
            self.persist
                .commit(PersistOp::State(Box::new(s.clone())))
                .await?;
            if from != s.state {
                self.publish_state(&s, from);
            }
        }
        self.admission.notify_one();
        self.snapshot(&id)
    }

    /// Restart from scratch: stop, discard partial data, queue again.
    pub(crate) async fn restart_task_inner(&self, id: TaskId) -> DomainResult<Task> {
        self.stop_run(&id, true).await;
        let snap = self.snapshot(&id)?;
        self.delete_part(&snap).await;
        if snap.kind.is_torrent() {
            self.torrent.forget(&snap, false).await;
        }
        self.persist
            .commit(PersistOp::DeleteCheckpoint(id.clone()))
            .await?;
        let (s, from) = self.mutate_from(&id, |t| {
            t.reset_for_rerun(false);
            if t.state != TaskState::Queued {
                t.transition(TaskState::Queued)?;
            }
            Ok(())
        })?;
        self.persist
            .commit(PersistOp::Update(Box::new(s.clone())))
            .await?;
        self.publish_state(&s, from);
        self.enqueue(&id).await?;
        self.snapshot(&id)
    }

    /// Retry keeping partial data.
    pub(crate) async fn retry_task_inner(&self, id: TaskId) -> DomainResult<Task> {
        let snap = self.snapshot(&id)?;
        match snap.state {
            TaskState::Failed | TaskState::Cancelled | TaskState::Paused | TaskState::Pending => {
                let (s, from) = self.mutate_from(&id, |t| {
                    t.error = None;
                    t.attempt = 0;
                    t.next_retry_at = None;
                    t.blocked_by.clear();
                    t.transition(TaskState::Queued)?;
                    Ok(())
                })?;
                self.persist
                    .commit(PersistOp::State(Box::new(s.clone())))
                    .await?;
                self.publish_state(&s, from);
                self.enqueue(&id).await?;
            }
            TaskState::Retrying => {
                let s = self.mutate(&id, |t| {
                    t.next_retry_at = None;
                    Ok(())
                })?;
                self.persist.send(PersistOp::State(Box::new(s)));
                self.admission.notify_one();
            }
            _ => {}
        }
        self.snapshot(&id)
    }

    /// Replace the primary URL (optional) and start over from the source.
    pub(crate) async fn retry_from_source_inner(
        &self,
        id: TaskId,
        new_url: Option<String>,
    ) -> DomainResult<Task> {
        self.stop_run(&id, true).await;
        let snap = self.snapshot(&id)?;
        let old_host = snap.domain();
        let mut keep_checkpoint = true;
        if let Some(u) = new_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
            let parsed = url::Url::parse(u)
                .map_err(|e| DomainError::validation(format!("invalid URL: {e}")))?;
            let expected: &[&str] = match snap.kind {
                TaskKind::Http | TaskKind::Metalink => &["http", "https"],
                TaskKind::Ftp => &["ftp", "ftps", "ftpes"],
                TaskKind::Hls => &["http", "https"],
                TaskKind::Magnet => &["magnet"],
                TaskKind::Torrent => &[],
            };
            if !expected.contains(&parsed.scheme()) {
                return Err(DomainError::validation(format!(
                    "URL scheme {:?} does not match a {} task",
                    parsed.scheme(),
                    snap.kind.as_str()
                )));
            }
            let new_host = parsed.host_str().map(|h| h.to_lowercase());
            keep_checkpoint = new_host == old_host;
        }
        if !keep_checkpoint {
            self.delete_part(&snap).await;
            self.persist
                .commit(PersistOp::DeleteCheckpoint(id.clone()))
                .await?;
        }
        let (s, from) = self.mutate_from(&id, |t| {
            if let Some(u) = new_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
                match &mut t.source {
                    Source::Urls { urls } => {
                        if urls.is_empty() {
                            urls.push(u.to_owned());
                        } else {
                            urls[0] = u.to_owned();
                        }
                    }
                    Source::Magnet { uri } => *uri = u.to_owned(),
                    Source::Metalink { url, .. } => *url = Some(u.to_owned()),
                    Source::Hls { playlist_url, .. } => *playlist_url = u.to_owned(),
                    Source::TorrentFile { .. } => {
                        return Err(DomainError::validation(
                            "torrent tasks have no URL to replace",
                        ))
                    }
                }
            }
            t.attempt = 0;
            t.error = None;
            t.next_retry_at = None;
            t.blocked_by.clear();
            t.stats.final_url = None;
            t.stats.etag = None;
            t.stats.last_modified = None;
            if !keep_checkpoint {
                t.segment_map = None;
                t.progress.downloaded = 0;
                t.progress.fraction = 0.0;
            }
            if t.state != TaskState::Queued {
                t.transition(TaskState::Queued)?;
            }
            Ok(())
        })?;
        self.persist
            .commit(PersistOp::Update(Box::new(s.clone())))
            .await?;
        self.publish_state(&s, from);
        self.enqueue(&id).await?;
        self.snapshot(&id)
    }

    /// Clone a task into a fresh one.
    fn clone_task(&self, src: &Task, state_pending: bool) -> Task {
        let mut t = Task::new(
            src.kind,
            src.source.clone(),
            src.name.clone(),
            src.directory.clone(),
            src.queue_id.clone(),
        );
        t.name_locked = src.name_locked;
        t.directory_locked = src.directory_locked;
        t.category_id = src.category_id.clone();
        t.schedule_id = src.schedule_id.clone();
        t.priority = src.priority;
        t.tags = src.tags.clone();
        t.options = src.options.clone();
        t.origin = src.origin.clone();
        t.mime = src.mime.clone();
        t.torrent = src.torrent.clone().map(|mut i| {
            i.uploaded = 0;
            i.ratio = 0.0;
            i.seeding_since = None;
            i
        });
        t.media = src.media.clone().map(|mut m| {
            m.segments_done = 0;
            m
        });
        t.progress.total = src.progress.total;
        t.stats.range_supported = src.stats.range_supported;
        if src.kind.is_torrent() {
            t.file_path = Some(t.directory.join(&t.name));
        }
        let _ = state_pending;
        t
    }

    async fn insert_clone(&self, task: Task, start: bool) -> DomainResult<Task> {
        self.persist
            .commit(PersistOp::Insert(Box::new(task.clone())))
            .await?;
        self.tasks.insert(task.clone());
        self.bus.publish(Event::TaskAdded(Box::new(task.clone())));
        if start {
            self.enqueue(&task.id).await?;
        }
        self.snapshot(&task.id)
    }

    /// Download again into a new task (the completed one is kept).
    pub(crate) async fn redownload_inner(&self, id: TaskId) -> DomainResult<Task> {
        let src = self.snapshot(&id)?;
        let mut t = self.clone_task(&src, false);
        if src.state == TaskState::Completed && !src.kind.is_torrent() {
            t.name = crate::manager::unique_name(&t.directory, &src.name);
        }
        self.insert_clone(t, true).await
    }

    /// Same as redownload but left `Pending` for the user to edit.
    pub(crate) async fn duplicate_task_inner(&self, id: TaskId) -> DomainResult<Task> {
        let src = self.snapshot(&id)?;
        let t = self.clone_task(&src, true);
        self.insert_clone(t, false).await
    }

    /// Remove a task: stop it, drop engine state, delete partial data (and the file when
    /// asked — never anything outside `task.directory`), delete the row.
    pub(crate) async fn remove_task_inner(
        &self,
        id: TaskId,
        delete_file: bool,
    ) -> DomainResult<()> {
        let snap = self.snapshot(&id)?;
        self.stop_run(&id, true).await;
        let snap = self.snapshot(&id).unwrap_or(snap);
        if snap.kind.is_torrent() {
            self.torrent.forget(&snap, delete_file).await;
        }
        self.delete_part(&snap).await;
        let mut deleted = false;
        if delete_file && !snap.kind.is_torrent() {
            // Only a file Osprey itself promoted is deleted: engines write to a part file and
            // rename it into place when the run completes, recording `file_path`. Until then
            // `<directory>/<name>` may be an unrelated file the user already had (the engine
            // would have picked a unique name), so it is never guessed.
            if let Some(p) = snap.file_path.clone().filter(|_| {
                matches!(
                    snap.state,
                    TaskState::Completed | TaskState::Processing | TaskState::Verifying
                )
            }) {
                if is_strictly_within(&snap.directory, &p) {
                    match tokio::fs::symlink_metadata(&p).await {
                        Ok(m) if m.is_dir() => {
                            deleted = tokio::fs::remove_dir_all(&p).await.is_ok()
                        }
                        Ok(m) if m.file_type().is_symlink() => {}
                        Ok(_) => deleted = tokio::fs::remove_file(&p).await.is_ok(),
                        Err(_) => {}
                    }
                }
            }
        } else if delete_file {
            deleted = true;
        }
        self.persist.commit(PersistOp::Delete(id.clone())).await?;
        self.tasks.remove(&id);
        self.log_rings.remove(&id);
        self.bus.publish(Event::TaskRemoved {
            task_id: id,
            deleted_file: deleted,
        });
        self.admission.notify_one();
        Ok(())
    }

    pub(crate) async fn update_task_inner(
        &self,
        id: TaskId,
        patch: TaskPatch,
    ) -> DomainResult<Task> {
        let snap = self.snapshot(&id)?;
        let running = self.tasks.run(&id);
        // Validate outside the lock.
        let directory = match &patch.directory {
            Some(d) => {
                if snap.state.is_active() {
                    return Err(DomainError::Conflict(
                        "pause the task before changing its directory".into(),
                    ));
                }
                let expanded = AppPaths::expand_home(&d.to_string_lossy());
                validate_destination_dir(&expanded)
                    .map_err(|e| DomainError::validation(e.message))?;
                Some(expanded)
            }
            None => None,
        };
        let name = match &patch.name {
            Some(n) => {
                if snap.state.is_active() {
                    return Err(DomainError::Conflict(
                        "pause the task before renaming it".into(),
                    ));
                }
                let s = sanitize_filename(n.trim());
                if s.is_empty() {
                    return Err(DomainError::validation("name must not be empty"));
                }
                Some(s)
            }
            None => None,
        };
        if let Some(q) = &patch.queue_id {
            if !self.queues.contains(q) {
                return Err(DomainError::not_found(format!("queue {q}")));
            }
        }
        if let Some(Some(c)) = &patch.category_id {
            if !self.categories.read().iter().any(|x| &x.id == c) {
                return Err(DomainError::not_found(format!("category {c}")));
            }
        }
        if let Some(Some(s)) = &patch.schedule_id {
            if !self.scheduler.contains(s) {
                return Err(DomainError::not_found(format!("schedule {s}")));
            }
        }
        if let Some(o) = &patch.options {
            osprey_runtime::net::validate_headers(
                o.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            )
            .map_err(|e| DomainError::validation(e.message))?;
            if let Some(c) = &o.checksum {
                if !c.is_well_formed() {
                    return Err(DomainError::validation("checksum is not well formed"));
                }
            }
        }
        if let Some(m) = &patch.mirrors {
            for u in m {
                let p = url::Url::parse(u)
                    .map_err(|e| DomainError::validation(format!("mirror: {e}")))?;
                if !matches!(p.scheme(), "http" | "https") {
                    return Err(DomainError::validation("mirrors must be http(s) URLs"));
                }
            }
        }
        let old_queue = snap.queue_id.clone();
        let old_dir = snap.directory.clone();
        let old_name = snap.name.clone();
        let s = self.mutate(&id, |t| {
            if let Some(n) = name {
                t.name_locked = true;
                t.name = n;
            }
            if let Some(d) = directory {
                t.directory_locked = true;
                t.directory = d;
            }
            if let Some(q) = patch.queue_id.clone() {
                t.queue_id = q;
            }
            if let Some(c) = patch.category_id.clone() {
                t.category_id = c;
            }
            if let Some(sch) = patch.schedule_id.clone() {
                t.schedule_id = sch;
            }
            if let Some(p) = patch.priority {
                t.priority = p;
            }
            if let Some(tags) = patch.tags.clone() {
                t.tags = tags;
            }
            if let Some(o) = patch.options.clone() {
                t.options = o;
            }
            if let Some(m) = patch.mirrors.clone() {
                if let Source::Urls { urls } = &mut t.source {
                    let primary = urls.first().cloned();
                    let mut list = Vec::new();
                    if let Some(p) = primary {
                        list.push(p);
                    }
                    for u in m {
                        if !list.contains(&u) {
                            list.push(u);
                        }
                    }
                    *urls = list;
                }
            }
            if (old_dir != t.directory || old_name != t.name) && !t.state.is_terminal() {
                // partial data lives under the old name; a fresh run starts over
                t.segment_map = None;
                t.progress.downloaded = 0;
                t.progress.fraction = 0.0;
            }
            Ok(())
        })?;
        if old_dir != s.directory || old_name != s.name {
            let mut old = snap.clone();
            old.segment_map = snap.segment_map.clone();
            if !old.state.is_terminal() {
                self.delete_part(&old).await;
                self.persist
                    .commit(PersistOp::DeleteCheckpoint(id.clone()))
                    .await?;
            }
        }
        self.persist
            .commit(PersistOp::Update(Box::new(s.clone())))
            .await?;
        self.bus.publish(Event::TaskUpdated(Box::new(s.clone())));
        if let Some(run) = running {
            run.control
                .download_limit
                .store(s.options.download_limit.unwrap_or(0), Ordering::Relaxed);
            run.control
                .upload_limit
                .store(s.options.upload_limit.unwrap_or(0), Ordering::Relaxed);
            run.control
                .max_connections
                .store(s.options.max_connections.unwrap_or(0), Ordering::Relaxed);
            run.limiter.set_limit(s.options.download_limit.unwrap_or(0));
            run.upload_limiter
                .set_limit(s.options.upload_limit.unwrap_or(0));
        }
        if old_queue != s.queue_id {
            if let Some(q) = self.queues.get(&old_queue) {
                self.unblock_task(&id, &PauseReason::Queue(q.name)).await;
            }
            if let Some(q) = self.queues.get(&s.queue_id) {
                if q.paused {
                    self.block_task(&id, PauseReason::Queue(q.name)).await;
                }
            }
        }
        self.admission.notify_one();
        self.snapshot(&id)
    }

    pub(crate) async fn set_task_limit_inner(
        &self,
        id: TaskId,
        download: Option<u64>,
        upload: Option<u64>,
    ) -> DomainResult<Task> {
        let s = self.mutate(&id, |t| {
            t.options.download_limit = download;
            t.options.upload_limit = upload;
            Ok(())
        })?;
        if let Some(run) = self.tasks.run(&id) {
            run.control
                .download_limit
                .store(download.unwrap_or(0), Ordering::Relaxed);
            run.control
                .upload_limit
                .store(upload.unwrap_or(0), Ordering::Relaxed);
            run.limiter.set_limit(download.unwrap_or(0));
            run.upload_limiter.set_limit(upload.unwrap_or(0));
        }
        self.persist
            .commit(PersistOp::Update(Box::new(s.clone())))
            .await?;
        self.bus.publish(Event::TaskUpdated(Box::new(s.clone())));
        Ok(s)
    }

    pub(crate) async fn set_task_connections_inner(
        &self,
        id: TaskId,
        connections: u8,
    ) -> DomainResult<Task> {
        let s = self.mutate(&id, |t| {
            t.options.max_connections = if connections == 0 {
                None
            } else {
                Some(connections.min(64))
            };
            Ok(())
        })?;
        if let Some(run) = self.tasks.run(&id) {
            run.control
                .max_connections
                .store(connections.min(64), Ordering::Relaxed);
        }
        self.persist
            .commit(PersistOp::Update(Box::new(s.clone())))
            .await?;
        self.bus.publish(Event::TaskUpdated(Box::new(s.clone())));
        Ok(s)
    }

    pub(crate) async fn set_task_priority_inner(
        &self,
        id: TaskId,
        priority: Priority,
    ) -> DomainResult<Task> {
        let s = self.mutate(&id, |t| {
            t.priority = priority;
            Ok(())
        })?;
        self.persist
            .commit(PersistOp::Update(Box::new(s.clone())))
            .await?;
        self.bus.publish(Event::TaskUpdated(Box::new(s.clone())));
        self.admission.notify_one();
        Ok(s)
    }

    /// Move `ids` right after `after` (None = top) within their queue and renumber positions.
    pub(crate) async fn reorder_tasks_inner(
        &self,
        ids: Vec<TaskId>,
        after: Option<TaskId>,
    ) -> DomainResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let first = self.snapshot(&ids[0])?;
        let queue_id = first.queue_id.clone();
        let mut ordered: Vec<Task> = self
            .tasks
            .all()
            .into_iter()
            .filter(|t| t.queue_id == queue_id)
            .collect();
        ordered.sort_by_key(|t| (t.position, t.created_at));
        let moving: Vec<Task> = ordered
            .iter()
            .filter(|t| ids.contains(&t.id))
            .cloned()
            .collect();
        ordered.retain(|t| !ids.contains(&t.id));
        let insert_at = match &after {
            Some(a) => ordered
                .iter()
                .position(|t| &t.id == a)
                .map(|p| p + 1)
                .unwrap_or(0),
            None => 0,
        };
        for (i, m) in moving.into_iter().enumerate() {
            ordered.insert(insert_at + i, m);
        }
        for (i, t) in ordered.iter().enumerate() {
            let pos = (i as i64 + 1) * 1000;
            if t.position == pos {
                continue;
            }
            let s = self.mutate(&t.id, |x| {
                x.position = pos;
                Ok(())
            })?;
            self.persist.send(PersistOp::Update(Box::new(s.clone())));
            self.bus.publish(Event::TaskUpdated(Box::new(s)));
        }
        self.persist.flush().await?;
        self.admission.notify_one();
        Ok(())
    }

    /// Retry the unfinished segments of an HTTP task keeping its checkpoint.
    pub(crate) async fn retry_failed_segments_inner(&self, id: TaskId) -> DomainResult<Task> {
        let snap = self.snapshot(&id)?;
        if !matches!(
            snap.kind,
            TaskKind::Http | TaskKind::Metalink | TaskKind::Ftp
        ) {
            return Err(DomainError::validation("only HTTP/FTP tasks have segments"));
        }
        if !matches!(
            snap.state,
            TaskState::Failed | TaskState::Paused | TaskState::Cancelled
        ) {
            return Err(DomainError::InvalidTransition(format!(
                "task {id} is {}; pause or wait for it to fail first",
                snap.state
            )));
        }
        if snap.segment_map.is_none() {
            return Err(DomainError::validation("task has no resume data"));
        }
        self.retry_task_inner(id).await
    }

    /// Verify a completed file against `checksum` (or the task's own).
    pub(crate) async fn verify_task_inner(
        &self,
        id: TaskId,
        checksum: Option<Checksum>,
    ) -> DomainResult<Task> {
        let snap = self.snapshot(&id)?;
        if snap.state != TaskState::Completed {
            return Err(DomainError::InvalidTransition(
                "only completed tasks can be verified".into(),
            ));
        }
        let path = snap.target_path();
        if !path.is_file() {
            return Err(DomainError::validation("the completed file is missing"));
        }
        let expected = checksum
            .or_else(|| snap.options.checksum.clone())
            .or_else(|| snap.verified_checksum.clone());
        let cancel = tokio_util::sync::CancellationToken::new();
        let result = match &expected {
            Some(exp) => {
                if !exp.is_well_formed() {
                    return Err(DomainError::validation("checksum is not well formed"));
                }
                osprey_runtime::checksum::verify_file(&path, exp, cancel).await
            }
            None => {
                let algo = self.settings().storage.default_checksum_algorithm;
                osprey_runtime::checksum::hash_file(&path, algo, cancel, |_| {}).await
            }
        };
        let s = match result {
            Ok(c) => {
                self.task_log_line(
                    &id,
                    osprey_domain::events::LogLevel::Info,
                    "verify.ok",
                    format!("{} {}", c.algorithm.as_str(), c.value),
                );
                self.mutate(&id, |t| {
                    t.verified_checksum = Some(c);
                    t.error = None;
                    Ok(())
                })?
            }
            Err(e) => {
                self.task_log_line(
                    &id,
                    osprey_domain::events::LogLevel::Error,
                    "verify.mismatch",
                    e.message.clone(),
                );
                self.notify(osprey_domain::Notification::ChecksumMismatch {
                    task_id: id.clone(),
                    name: snap.name.clone(),
                });
                self.mutate(&id, |t| {
                    t.error = Some(TaskError {
                        kind: ErrorKind::ChecksumMismatch,
                        ..e
                    });
                    Ok(())
                })?
            }
        };
        self.persist
            .commit(PersistOp::Update(Box::new(s.clone())))
            .await?;
        self.bus.publish(Event::TaskUpdated(Box::new(s.clone())));
        Ok(s)
    }

    /// Pause every pausable task with `reason` (user pause-all, sleep, shutdown).
    pub(crate) async fn pause_all_with(&self, reason: PauseReason) -> u32 {
        let mut n = 0;
        let mut runs = Vec::new();
        for t in self.tasks.all() {
            if t.state.can_pause() || (t.state == TaskState::Pending && reason == PauseReason::User)
            {
                if let Some(run) = self.tasks.run(&t.id) {
                    runs.push(run);
                }
                self.block_task(&t.id, reason.clone()).await;
                n += 1;
            }
        }
        for run in runs {
            run.wait_finished(STOP_WAIT).await;
        }
        n
    }

    pub(crate) async fn pause_all_inner(&self) -> DomainResult<u32> {
        Ok(self.pause_all_with(PauseReason::User).await)
    }

    pub(crate) async fn resume_all_inner(&self) -> DomainResult<u32> {
        let mut n = 0;
        for t in self.tasks.all() {
            if t.state == TaskState::Paused && t.is_user_paused() {
                self.unblock_task(&t.id, &PauseReason::User).await;
                n += 1;
            }
        }
        Ok(n)
    }

    pub(crate) async fn retry_all_failed_inner(&self) -> DomainResult<u32> {
        let mut n = 0;
        for t in self.tasks.all() {
            if t.state == TaskState::Failed && self.retry_task_inner(t.id.clone()).await.is_ok() {
                n += 1;
            }
        }
        Ok(n)
    }

    pub(crate) async fn clear_completed_inner(&self) -> DomainResult<u32> {
        let mut n = 0;
        for t in self.tasks.all() {
            if t.state == TaskState::Completed
                && self.remove_task_inner(t.id.clone(), false).await.is_ok()
            {
                n += 1;
            }
        }
        Ok(n)
    }

    // ---------------------------------------------------------------------------------------
    // listing
    // ---------------------------------------------------------------------------------------

    /// Filter + sort the in-memory table (paging applied by the caller).
    pub(crate) fn filtered_tasks(&self, f: &TaskFilter) -> Vec<Task> {
        let text = f
            .text
            .as_ref()
            .map(|t| t.to_lowercase())
            .filter(|t| !t.is_empty());
        let mut out: Vec<Task> = self
            .tasks
            .all()
            .into_iter()
            .filter(|t| f.states.is_empty() || f.states.contains(&t.state))
            .filter(|t| f.kinds.is_empty() || f.kinds.contains(&t.kind))
            .filter(|t| {
                f.queue_id
                    .as_ref()
                    .map(|q| &t.queue_id == q)
                    .unwrap_or(true)
            })
            .filter(|t| {
                f.category_id
                    .as_ref()
                    .map(|c| t.category_id.as_ref() == Some(c))
                    .unwrap_or(true)
            })
            .filter(|t| {
                f.domain
                    .as_ref()
                    .map(|d| t.domain().as_deref() == Some(d.as_str()))
                    .unwrap_or(true)
            })
            .filter(|t| {
                f.tag
                    .as_ref()
                    .map(|tag| t.tags.iter().any(|x| x == tag))
                    .unwrap_or(true)
            })
            .filter(|t| f.created_since.map(|s| t.created_at >= s).unwrap_or(true))
            .filter(|t| {
                f.min_size
                    .map(|m| t.progress.total.unwrap_or(0) >= m)
                    .unwrap_or(true)
            })
            .filter(|t| {
                f.max_size
                    .map(|m| t.progress.total.unwrap_or(0) <= m)
                    .unwrap_or(true)
            })
            .filter(|t| match f.smart.as_deref() {
                None | Some("") | Some("all") => true,
                Some("active") => t.state.is_active(),
                Some("queued") => matches!(t.state, TaskState::Queued | TaskState::Pending),
                Some("scheduled") => t.state == TaskState::Scheduled,
                Some("complete") | Some("completed") => t.state == TaskState::Completed,
                Some("failed") => t.state == TaskState::Failed,
                Some("torrent") => t.kind.is_torrent(),
                Some("media") => t.kind == TaskKind::Hls || t.media.is_some(),
                Some("paused") => t.state == TaskState::Paused,
                Some(_) => true,
            })
            .filter(|t| match &text {
                None => true,
                Some(q) => {
                    t.name.to_lowercase().contains(q)
                        || t.source
                            .primary_url()
                            .map(|u| u.to_lowercase().contains(q))
                            .unwrap_or(false)
                        || t.domain().map(|d| d.contains(q)).unwrap_or(false)
                        || t.tags.iter().any(|tag| tag.to_lowercase().contains(q))
                }
            })
            .collect();
        out.sort_by(|a, b| {
            let ord = match f.sort {
                TaskSort::Position => a
                    .position
                    .cmp(&b.position)
                    .then(a.created_at.cmp(&b.created_at)),
                TaskSort::CreatedAt => a.created_at.cmp(&b.created_at),
                TaskSort::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                TaskSort::Size => a.progress.total.cmp(&b.progress.total),
                TaskSort::Progress => a
                    .progress
                    .percent()
                    .unwrap_or(0.0)
                    .partial_cmp(&b.progress.percent().unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal),
                TaskSort::Speed => a.progress.speed.cmp(&b.progress.speed),
                TaskSort::Eta => a.progress.eta_seconds.cmp(&b.progress.eta_seconds),
                TaskSort::State => a.state.as_str().cmp(b.state.as_str()),
                TaskSort::Domain => a.domain().cmp(&b.domain()),
            };
            if f.descending {
                ord.reverse()
            } else {
                ord
            }
        });
        out
    }

    pub(crate) fn page<T: Clone>(items: Vec<T>, limit: u32, offset: u32) -> Vec<T> {
        let it = items.into_iter().skip(offset as usize);
        if limit == 0 {
            it.collect()
        } else {
            it.take(limit as usize).collect()
        }
    }

    pub(crate) fn rows(&self, f: &TaskFilter) -> Vec<TaskRow> {
        Self::page(self.filtered_tasks(f), f.limit, f.offset)
            .iter()
            .map(TaskRow::from)
            .collect()
    }

    /// Absolute path helper for the API layer.
    pub(crate) fn expand(p: &std::path::Path) -> PathBuf {
        AppPaths::expand_home(&p.to_string_lossy())
    }
}
