//! Task lifecycle: building tasks from requests, admission, starting runs and handling their
//! outcomes, plus the sink handlers engines report into.

use crate::api::{AddTaskResult, DuplicateInfo};
use crate::engine::Engine;
use crate::persist::PersistOp;
use crate::tasks::{RunHandle, TaskSink, CHECKPOINT_DEBOUNCE};
use osprey_domain::automation::AutomationEvent;
use osprey_domain::checkpoint::Checkpoint;
use osprey_domain::queue::QueueCompletionAction;
use osprey_domain::rules::RuleAction;
use osprey_domain::state::PauseReason;
use osprey_domain::torrent::TorrentInfo;
use osprey_domain::{
    ConflictPolicy, DomainError, DomainResult, ErrorKind, Event, FailureClass, Millis,
    NewTaskRequest, Notification, Progress, QueueId, Source, Task, TaskError, TaskId, TaskKind,
    TaskState,
};
use osprey_runtime::backoff::BackoffPolicy;
use osprey_runtime::engine::{
    EngineStat, ResolvedMetadata, Transfer, TransferContext, TransferControl, TransferOutcome,
};
use osprey_runtime::paths::AppPaths;
use osprey_runtime::redact::redact;
use osprey_runtime::safety::{
    ensure_within, sanitize_filename, unique_path, validate_destination_dir,
};
use osprey_runtime::{filename, RateLimiter};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// Maximum automatic restarts from scratch (checksum mismatch / source changed).
const MAX_RESTARTS: u32 = 2;

/// What `detect_source` learned about a request.
struct Detected {
    kind: TaskKind,
    source: Source,
    torrent: Option<TorrentInfo>,
}

impl Engine {
    // ---------------------------------------------------------------------------------------
    // add
    // ---------------------------------------------------------------------------------------

    async fn detect_source(&self, req: &NewTaskRequest) -> DomainResult<Detected> {
        let url = req.url.as_deref().map(str::trim).filter(|u| !u.is_empty());
        if let Some(m) = req
            .magnet
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
        {
            return Ok(Detected {
                kind: TaskKind::Magnet,
                source: Source::Magnet { uri: m.to_owned() },
                torrent: None,
            });
        }
        if let Some(u) = url {
            if u.to_ascii_lowercase().starts_with("magnet:") {
                return Ok(Detected {
                    kind: TaskKind::Magnet,
                    source: Source::Magnet { uri: u.to_owned() },
                    torrent: None,
                });
            }
        }
        if let Some(b64) = req.torrent_base64.as_deref().filter(|b| !b.is_empty()) {
            use base64::Engine as _;
            let cleaned: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&cleaned)
                .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&cleaned))
                .map_err(|e| DomainError::validation(format!("torrent_base64: {e}")))?;
            let info = osprey_engine_torrent::TorrentEngine::parse_torrent(&bytes)
                .map_err(|e| DomainError::validation(e.message))?;
            self.store.put_torrent_blob(&info.info_hash, bytes).await?;
            return Ok(Detected {
                kind: TaskKind::Torrent,
                source: Source::TorrentFile {
                    info_hash: info.info_hash.clone(),
                    name: info.name.clone(),
                },
                torrent: Some(info),
            });
        }
        if let Some(m) = req
            .metalink_url
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
        {
            check_http_url(m)?;
            return Ok(Detected {
                kind: TaskKind::Metalink,
                source: Source::Metalink {
                    url: Some(m.to_owned()),
                    document: None,
                },
                torrent: None,
            });
        }
        if let Some(h) = req
            .hls_playlist_url
            .as_deref()
            .map(str::trim)
            .filter(|h| !h.is_empty())
        {
            check_http_url(h)?;
            return Ok(Detected {
                kind: TaskKind::Hls,
                source: Source::Hls {
                    playlist_url: h.to_owned(),
                    variant: req.options.media_variant.clone(),
                },
                torrent: None,
            });
        }
        let Some(u) = url else {
            return Err(DomainError::validation(
                "a URL, magnet link or torrent is required",
            ));
        };
        let parsed =
            url::Url::parse(u).map_err(|e| DomainError::validation(format!("invalid URL: {e}")))?;
        if u.bytes().any(|b| b.is_ascii_control()) {
            return Err(DomainError::validation("URL contains control characters"));
        }
        let path_lower = parsed.path().to_ascii_lowercase();
        match parsed.scheme() {
            "http" | "https" => {
                if path_lower.ends_with(".metalink") || path_lower.ends_with(".meta4") {
                    Ok(Detected {
                        kind: TaskKind::Metalink,
                        source: Source::Metalink {
                            url: Some(u.to_owned()),
                            document: None,
                        },
                        torrent: None,
                    })
                } else if path_lower.ends_with(".m3u8") || path_lower.ends_with(".m3u") {
                    Ok(Detected {
                        kind: TaskKind::Hls,
                        source: Source::Hls {
                            playlist_url: u.to_owned(),
                            variant: req.options.media_variant.clone(),
                        },
                        torrent: None,
                    })
                } else {
                    let mut urls = vec![u.to_owned()];
                    for m in &req.mirrors {
                        let m = m.trim();
                        if !m.is_empty()
                            && check_http_url(m).is_ok()
                            && !urls.iter().any(|x| x == m)
                        {
                            urls.push(m.to_owned());
                        }
                    }
                    Ok(Detected {
                        kind: TaskKind::Http,
                        source: Source::Urls { urls },
                        torrent: None,
                    })
                }
            }
            "ftp" | "ftps" | "ftpes" => Ok(Detected {
                kind: TaskKind::Ftp,
                source: Source::Urls {
                    urls: vec![u.to_owned()],
                },
                torrent: None,
            }),
            other => Err(DomainError::validation(format!(
                "unsupported URL scheme {other:?}"
            ))),
        }
    }

    /// Build a task from a request without persisting it (shared by `probe` and `add_task`).
    pub(crate) async fn build_task(
        &self,
        req: &NewTaskRequest,
        extra: Option<&[RuleAction]>,
    ) -> DomainResult<(Task, crate::rules::RuleOutcome)> {
        let settings = self.settings();
        let detected = self.detect_source(req).await?;
        let raw_name = req
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
            .or_else(|| detected.torrent.as_ref().map(|t| t.name.clone()))
            .or_else(|| detected.source.primary_url().and_then(filename::from_url))
            .unwrap_or_else(|| match &detected.source {
                Source::Magnet { uri } => magnet_display_name(uri),
                _ => "download".to_owned(),
            });
        let name = sanitize_filename(&raw_name);

        let queue_id = req
            .queue_id
            .clone()
            .filter(|q| self.queues.contains(q))
            .or_else(|| {
                detected
                    .kind
                    .is_torrent()
                    .then(|| QueueId("queue-torrent".into()))
                    .filter(|q| self.queues.contains(q))
            })
            .unwrap_or_else(QueueId::default_queue);
        let queue = self.queues.get(&queue_id);

        let mut task = Task::new(
            detected.kind,
            detected.source,
            name,
            settings.storage.download_directory.clone(),
            queue_id,
        );
        task.name_locked = req
            .name
            .as_deref()
            .map(|n| !n.trim().is_empty())
            .unwrap_or(false);
        task.priority = req.priority.unwrap_or_default();
        task.tags = req
            .tags
            .iter()
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect();
        task.options = req.options.clone();
        task.origin = if req.origin.trim().is_empty() {
            "app".into()
        } else {
            req.origin.trim().to_owned()
        };
        task.mime = filename::mime_for_name(&task.name);
        task.torrent = detected.torrent;
        if let Some(t) = &task.torrent {
            task.progress.total = Some(t.total_size);
            task.mime = Some("application/x-bittorrent".into());
        }
        if let Some(c) = &req.category_id {
            if self.categories.read().iter().any(|x| &x.id == c) {
                task.category_id = Some(c.clone());
            }
        }
        if let Some(s) = &req.schedule_id {
            if self.scheduler.contains(s) {
                task.schedule_id = Some(s.clone());
            }
        }
        if task.category_id.is_none() {
            task.category_id = self.auto_category(&task);
        }
        osprey_runtime::net::validate_headers(
            task.options
                .headers
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        )
        .map_err(|e| DomainError::validation(e.message))?;
        if let Some(c) = &task.options.checksum {
            if !c.is_well_formed() {
                return Err(DomainError::validation("checksum is not well formed"));
            }
        }

        // Directory: request → category (organise_by_category) → queue → settings.
        task.directory = match &req.directory {
            Some(d) if !d.as_os_str().is_empty() => {
                task.directory_locked = true;
                AppPaths::expand_home(&d.to_string_lossy())
            }
            _ => {
                let base = queue
                    .as_ref()
                    .and_then(|q| q.directory.clone())
                    .unwrap_or_else(|| settings.storage.download_directory.clone());
                match task
                    .category_id
                    .as_ref()
                    .filter(|_| settings.storage.organise_by_category)
                    .and_then(|c| self.category_dir(c))
                {
                    Some(cd) if cd.is_absolute() => cd,
                    Some(cd) => base.join(cd),
                    None => base,
                }
            }
        };

        let mut outcome = self.apply_rules(&mut task);
        if let Some(extra) = extra {
            let mut post = Vec::new();
            self.apply_actions(&mut task, extra, 1, &mut post);
            outcome.post.extend(post);
        }
        if task.category_id.is_none() {
            task.category_id = self.auto_category(&task);
        }
        task.directory = AppPaths::expand_home(&task.directory.to_string_lossy());
        validate_destination_dir(&task.directory)
            .map_err(|e| DomainError::validation(e.message))?;
        if task.kind.is_torrent() {
            // librqbit writes into `<dir>/<torrent name>/`
            task.file_path = Some(task.directory.join(&task.name));
        }
        Ok((task, outcome))
    }

    /// The full add path: build → duplicates → persist → events → enqueue.
    pub(crate) async fn add_task_inner(
        &self,
        req: NewTaskRequest,
        extra: Option<Vec<RuleAction>>,
    ) -> DomainResult<AddTaskResult> {
        if self.shutting_down.load(Ordering::Relaxed) {
            return Err(DomainError::Unavailable("engine is shutting down".into()));
        }
        let settings = self.settings();
        let (mut task, outcome) = self.build_task(&req, extra.as_deref()).await?;
        let policy = if req.options.conflict_policy == ConflictPolicy::Ask {
            settings.storage.default_conflict_policy
        } else {
            req.options.conflict_policy
        };
        let mut duplicate: Option<DuplicateInfo> = None;
        let mut ask = false;
        if let Some(dup) = self.detect_duplicate(&task).await {
            match policy {
                ConflictPolicy::Skip => {
                    if let Some(existing) = dup
                        .existing_task_id
                        .as_ref()
                        .and_then(|id| self.tasks.snapshot(id))
                    {
                        return Ok(AddTaskResult {
                            task: existing,
                            duplicate: Some(dup),
                        });
                    }
                    return Err(DomainError::Conflict(format!(
                        "already downloaded ({}): {}",
                        dup.matched_by,
                        dup.existing_path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default()
                    )));
                }
                ConflictPolicy::Replace => task.options.conflict_policy = ConflictPolicy::Replace,
                ConflictPolicy::Rename | ConflictPolicy::KeepBoth => {
                    task.name = unique_name(&task.directory, &task.name);
                    task.options.conflict_policy = ConflictPolicy::Rename;
                }
                ConflictPolicy::Ask => {
                    ask = true;
                    duplicate = Some(dup);
                }
            }
        }
        task.touch();
        self.persist
            .commit(PersistOp::Insert(Box::new(task.clone())))
            .await?;
        self.tasks.insert(task.clone());
        self.record_rule_hits(&outcome.rule_ids).await;
        self.bus.publish(Event::TaskAdded(Box::new(task.clone())));
        if !outcome.applied.is_empty() {
            self.task_log_line(
                &task.id,
                osprey_domain::events::LogLevel::Info,
                "rules.applied",
                format!("rules: {}", outcome.applied.join(", ")),
            );
        }
        if ask {
            if let Some(d) = &duplicate {
                self.notify(Notification::DuplicateDetected {
                    task_id: task.id.clone(),
                    name: task.name.clone(),
                    existing_path: d
                        .existing_path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                });
            }
        } else if req.start {
            self.enqueue(&task.id).await?;
        }
        let task = self.tasks.snapshot(&task.id).unwrap_or(task);
        Ok(AddTaskResult { task, duplicate })
    }

    pub(crate) async fn resolve_duplicate_inner(
        &self,
        id: TaskId,
        policy: ConflictPolicy,
    ) -> DomainResult<Task> {
        let cell = self.task_cell(&id)?;
        let state = cell.lock().state;
        if state != TaskState::Pending {
            return Err(DomainError::InvalidTransition(format!(
                "task {id} is {state}, not pending"
            )));
        }
        match policy {
            ConflictPolicy::Ask => {}
            ConflictPolicy::Skip => {
                let snap = self.mutate(&id, |t| t.transition(TaskState::Cancelled).map(|_| ()))?;
                self.persist
                    .commit(PersistOp::State(Box::new(snap.clone())))
                    .await?;
                self.publish_state(&snap, TaskState::Pending);
                return Ok(snap);
            }
            ConflictPolicy::Replace => {
                self.mutate(&id, |t| {
                    t.options.conflict_policy = ConflictPolicy::Replace;
                    Ok(())
                })?;
                self.enqueue(&id).await?;
            }
            ConflictPolicy::Rename | ConflictPolicy::KeepBoth => {
                self.mutate(&id, |t| {
                    t.name = unique_name(&t.directory, &t.name);
                    t.options.conflict_policy = ConflictPolicy::Rename;
                    Ok(())
                })?;
                self.enqueue(&id).await?;
            }
        }
        self.snapshot(&id)
    }

    // ---------------------------------------------------------------------------------------
    // helpers
    // ---------------------------------------------------------------------------------------

    pub(crate) fn task_cell(&self, id: &TaskId) -> DomainResult<Arc<parking_lot::Mutex<Task>>> {
        self.tasks
            .get(id)
            .ok_or_else(|| DomainError::not_found(format!("task {id}")))
    }

    pub(crate) fn snapshot(&self, id: &TaskId) -> DomainResult<Task> {
        self.tasks
            .snapshot(id)
            .ok_or_else(|| DomainError::not_found(format!("task {id}")))
    }

    /// Mutate a task under its lock, bump `rev`, and return the snapshot. The closure must not
    /// block or await.
    pub(crate) fn mutate<F>(&self, id: &TaskId, f: F) -> DomainResult<Task>
    where
        F: FnOnce(&mut Task) -> DomainResult<()>,
    {
        let cell = self.task_cell(id)?;
        let mut t = cell.lock();
        f(&mut t)?;
        t.touch();
        Ok(t.clone())
    }

    /// Publish the state-change pair for a snapshot.
    pub(crate) fn publish_state(&self, task: &Task, from: TaskState) {
        self.bus.publish(Event::TaskStateChanged {
            task_id: task.id.clone(),
            from,
            to: task.state,
            at: task.updated_at,
        });
        self.bus.publish(Event::TaskUpdated(Box::new(task.clone())));
    }

    /// Part file (or directory) of a task.
    pub(crate) fn part_path(&self, task: &Task) -> PathBuf {
        if let Some(map) = &task.segment_map {
            if let Some(p) = &map.part_path {
                return p.clone();
            }
        }
        task.directory.join(format!(
            "{}{}",
            task.name,
            self.settings().storage.temp_suffix
        ))
    }

    /// Delete a task's partial data, never leaving its directory.
    pub(crate) async fn delete_part(&self, task: &Task) {
        let part = self.part_path(task);
        if ensure_within(&task.directory, &part).is_err() {
            return;
        }
        let meta = tokio::fs::symlink_metadata(&part).await;
        match meta {
            Ok(m) if m.is_dir() => {
                let _ = tokio::fs::remove_dir_all(&part).await;
            }
            Ok(_) => {
                let _ = tokio::fs::remove_file(&part).await;
            }
            Err(_) => {}
        }
    }

    /// Move a task into `Queued` (or `Scheduled` when gated).
    pub(crate) async fn enqueue(&self, id: &TaskId) -> DomainResult<()> {
        let queue_block = {
            let cell = self.task_cell(id)?;
            let t = cell.lock();
            self.queues
                .get(&t.queue_id)
                .filter(|q| q.paused)
                .map(|q| PauseReason::Queue(q.name))
        };
        let schedule_block = {
            let t = self.snapshot(id)?;
            self.schedule_block_for(&t)
        };
        let (snap, from) = {
            let cell = self.task_cell(id)?;
            let mut t = cell.lock();
            let from = t.state;
            match t.state {
                TaskState::Pending
                | TaskState::Paused
                | TaskState::Failed
                | TaskState::Cancelled
                | TaskState::Completed
                | TaskState::Scheduled => {
                    if t.state == TaskState::Paused && t.is_user_paused() {
                        t.unblock(&PauseReason::User);
                    }
                    if !t.blocked_by.is_empty() && t.state == TaskState::Paused {
                        // still blocked by something automatic; stay paused
                        t.touch();
                        let snap = t.clone();
                        drop(t);
                        self.persist.send(PersistOp::State(Box::new(snap.clone())));
                        self.bus.publish(Event::TaskUpdated(Box::new(snap)));
                        return Ok(());
                    }
                    let target = if schedule_block.is_some() {
                        TaskState::Scheduled
                    } else {
                        TaskState::Queued
                    };
                    if t.state != target {
                        t.transition(target)?;
                    }
                    if let Some(b) = &schedule_block {
                        t.block(b.clone());
                    }
                    if let Some(b) = &queue_block {
                        t.block(b.clone());
                    }
                    t.touch();
                    (t.clone(), from)
                }
                TaskState::Queued | TaskState::Retrying => {
                    if let Some(b) = &queue_block {
                        t.block(b.clone());
                    }
                    (t.clone(), from)
                }
                other => {
                    return Err(DomainError::InvalidTransition(format!(
                        "task {id} cannot be started from {other}"
                    )))
                }
            }
        };
        self.persist
            .commit(PersistOp::State(Box::new(snap.clone())))
            .await?;
        if from != snap.state {
            self.publish_state(&snap, from);
            match snap.state {
                TaskState::Queued => self.notify(Notification::Queued {
                    task_id: snap.id.clone(),
                    name: snap.name.clone(),
                }),
                TaskState::Scheduled => {
                    let at = self
                        .scheduler
                        .next_boundary(Millis::now())
                        .into_iter()
                        .map(|(_, m)| m)
                        .min()
                        .unwrap_or_else(Millis::now);
                    self.notify(Notification::Scheduled {
                        task_id: snap.id.clone(),
                        name: snap.name.clone(),
                        at,
                    });
                }
                _ => {}
            }
        } else {
            self.bus.publish(Event::TaskUpdated(Box::new(snap)));
        }
        self.admission.notify_one();
        Ok(())
    }

    /// Add a block reason. Running tasks are asked to pause; waiting tasks change state.
    pub(crate) async fn block_task(&self, id: &TaskId, reason: PauseReason) {
        let Some(cell) = self.tasks.get(id) else {
            return;
        };
        let running = self.tasks.run(id);
        let result = {
            let mut t = cell.lock();
            if t.state.is_terminal() {
                return;
            }
            let newly = t.block(reason.clone());
            let from = t.state;
            if let Some(run) = &running {
                if newly && t.state.is_active() {
                    run.control.pause.cancel();
                }
                Some((t.clone(), from, false))
            } else {
                let target = match (&reason, t.state) {
                    (PauseReason::Queue(_), TaskState::Queued) => None,
                    (PauseReason::Schedule(_), TaskState::Queued | TaskState::Pending) => {
                        Some(TaskState::Scheduled)
                    }
                    (_, s) if s.can_transition_to(TaskState::Paused) => Some(TaskState::Paused),
                    _ => None,
                };
                let mut changed = false;
                if let Some(target) = target {
                    if t.state != target && t.transition(target).is_ok() {
                        // transition() clears blocks for non-paused states; re-add ours
                        t.block(reason.clone());
                        changed = true;
                    }
                }
                if newly || changed {
                    Some((t.clone(), from, changed))
                } else {
                    None
                }
            }
        };
        if let Some((snap, from, changed)) = result {
            self.persist.send(PersistOp::State(Box::new(snap.clone())));
            if changed {
                self.publish_state(&snap, from);
            } else {
                self.bus.publish(Event::TaskUpdated(Box::new(snap)));
            }
        }
    }

    /// Remove a block reason; a task with no remaining reasons goes back to `Queued`.
    pub(crate) async fn unblock_task(&self, id: &TaskId, reason: &PauseReason) {
        let Some(cell) = self.tasks.get(id) else {
            return;
        };
        let result = {
            let mut t = cell.lock();
            if !t.unblock(reason) {
                return;
            }
            let from = t.state;
            let mut changed = false;
            if t.blocked_by.is_empty()
                && matches!(t.state, TaskState::Paused | TaskState::Scheduled)
                && t.transition(TaskState::Queued).is_ok()
            {
                changed = true;
            }
            (t.clone(), from, changed)
        };
        let (snap, from, changed) = result;
        self.persist.send(PersistOp::State(Box::new(snap.clone())));
        if changed {
            self.publish_state(&snap, from);
        } else {
            self.bus.publish(Event::TaskUpdated(Box::new(snap)));
        }
        self.admission.notify_one();
    }

    // ---------------------------------------------------------------------------------------
    // admission
    // ---------------------------------------------------------------------------------------

    /// One admission pass: fill free slots in queue order.
    pub(crate) async fn admit_once(&self) {
        if self.shutting_down.load(Ordering::Relaxed) {
            return;
        }
        let settings = self.settings();
        let env_ok = self.environment_snapshot().network_available;
        let now = Millis::now();
        let all = self.tasks.all();
        let running: Vec<(TaskId, Arc<RunHandle>)> = self.tasks.running();
        let running_ids: Vec<&TaskId> = running.iter().map(|(id, _)| id).collect();
        // Seeding torrents keep their run but do not occupy a download slot.
        let slot_holder = |t: &Task| running_ids.contains(&&t.id) && t.state != TaskState::Seeding;
        let mut active_downloads = all
            .iter()
            .filter(|t| slot_holder(t) && !t.kind.is_torrent())
            .count() as u32;
        let mut active_torrents = all
            .iter()
            .filter(|t| slot_holder(t) && t.kind.is_torrent())
            .count() as u32;
        let mut to_start: Vec<TaskId> = Vec::new();
        for queue in self.queues.admission_order() {
            if queue.paused {
                continue;
            }
            let mut queue_active = all
                .iter()
                .filter(|t| t.queue_id == queue.id && slot_holder(t))
                .count() as u32;
            let mut candidates: Vec<&Task> = all
                .iter()
                .filter(|t| t.queue_id == queue.id)
                .filter(|t| !running_ids.contains(&&t.id))
                .filter(|t| t.blocked_by.is_empty())
                .filter(|t| match t.state {
                    TaskState::Queued => true,
                    TaskState::Retrying => t.next_retry_at.map(|at| at.0 <= now.0).unwrap_or(true),
                    _ => false,
                })
                .collect();
            candidates.sort_by(|a, b| {
                b.priority
                    .cmp(&a.priority)
                    .then(a.position.cmp(&b.position))
            });
            for t in candidates {
                if queue.max_concurrent > 0 && queue_active >= queue.max_concurrent {
                    break;
                }
                if !env_ok && !t.kind.is_torrent() {
                    continue;
                }
                if t.kind.is_torrent() {
                    if active_torrents >= settings.bandwidth.max_active_torrents {
                        continue;
                    }
                    active_torrents += 1;
                } else {
                    if active_downloads >= settings.bandwidth.max_active_downloads {
                        continue;
                    }
                    active_downloads += 1;
                }
                queue_active += 1;
                to_start.push(t.id.clone());
            }
        }
        for id in to_start {
            if let Err(e) = self.start_run(&id).await {
                tracing::warn!(task = %id, error = %e, "could not start run");
            }
        }
    }

    /// Spawn the engine run for a task.
    pub(crate) async fn start_run(&self, id: &TaskId) -> DomainResult<()> {
        if self.tasks.run(id).is_some() {
            return Ok(());
        }
        let task = self.snapshot(id)?;
        let settings = self.settings();
        let queue = self.queues.get(&task.queue_id);
        let engine: Arc<dyn Transfer> = match task.kind {
            TaskKind::Http | TaskKind::Metalink => self.http.clone(),
            TaskKind::Ftp => self.ftp.clone(),
            TaskKind::Torrent | TaskKind::Magnet => self.torrent.clone(),
            TaskKind::Hls => self.hls.clone(),
        };
        if let Err(e) = tokio::fs::create_dir_all(&task.directory).await {
            let err = TaskError::from_io(&e, "create destination directory");
            self.on_failed(id, err).await;
            return Ok(());
        }
        let control = TransferControl::new();
        control
            .download_limit
            .store(task.options.download_limit.unwrap_or(0), Ordering::Relaxed);
        control
            .upload_limit
            .store(task.options.upload_limit.unwrap_or(0), Ordering::Relaxed);
        let conns = task
            .options
            .max_connections
            .or_else(|| {
                queue
                    .as_ref()
                    .map(|q| q.bandwidth.connections_per_task)
                    .filter(|c| *c > 0)
            })
            .unwrap_or(0);
        control.max_connections.store(conns, Ordering::Relaxed);
        control.sequential.store(
            task.options
                .sequential
                .unwrap_or(settings.torrent.sequential_by_default),
            Ordering::Relaxed,
        );
        let (limiter, upload_limiter): (Arc<RateLimiter>, Arc<RateLimiter>) = self
            .bandwidth
            .task_limiters(&task.queue_id, id.as_str(), &task.options);
        limiter.set_burst(settings.bandwidth.burst_bytes);
        let run_id = self.tasks.next_run_id();
        let run = RunHandle::new(
            run_id,
            control.clone(),
            limiter.clone(),
            upload_limiter.clone(),
        );
        self.tasks.set_run(id, run.clone());

        let from = task.state;
        let snap = self.mutate(id, |t| {
            if t.state != TaskState::Resolving {
                t.transition(TaskState::Resolving)?;
            }
            t.status_detail = None;
            Ok(())
        })?;
        self.persist.send(PersistOp::State(Box::new(snap.clone())));
        self.publish_state(&snap, from);

        let checkpoint = match self.store.get_checkpoint(id).await {
            Ok(cp) => cp.or_else(|| snap.segment_map.clone().map(Checkpoint::Segments)),
            Err(e) => {
                tracing::warn!(task = %id, error = %e, "checkpoint load failed; starting fresh");
                None
            }
        };
        let secrets = self.resolve_secrets(&snap).await;
        let sink = TaskSink::new(self.this.clone(), id.clone(), run_id);
        control.counters.set_downloaded(snap.progress.downloaded);
        run.last_downloaded
            .store(snap.progress.downloaded, Ordering::Relaxed);
        let ctx = TransferContext {
            task: snap.clone(),
            checkpoint,
            control: control.clone(),
            sink,
            settings: settings.clone(),
            secrets,
            limiter,
            upload_limiter,
            clients: self.clients.clone(),
            started_at: Millis::now(),
        };
        self.task_log_line(
            id,
            osprey_domain::events::LogLevel::Info,
            "run.start",
            format!("attempt {} via {}", snap.attempt + 1, snap.kind.as_str()),
        );
        let weak = self.this.clone();
        let tid = id.clone();
        let join = tokio::spawn(async move {
            let outcome = engine.run(ctx).await;
            if let Some(e) = weak.upgrade() {
                e.handle_outcome(&tid, run_id, outcome).await;
            }
        });
        *run.join.lock() = Some(join);
        Ok(())
    }

    // ---------------------------------------------------------------------------------------
    // outcomes
    // ---------------------------------------------------------------------------------------

    /// Process how a run ended.
    pub(crate) async fn handle_outcome(&self, id: &TaskId, run_id: u64, outcome: TransferOutcome) {
        let Some(run) = self.tasks.clear_run(id, run_id) else {
            return;
        };
        self.flush_checkpoint(id, &run).await;
        // Run-level stats.
        if self.tasks.contains(id) {
            let _ = self.mutate(id, |t| {
                let meter = run.meter.lock();
                t.stats.peak_speed = t.stats.peak_speed.max(meter.peak());
                let secs = run.started.elapsed().as_secs();
                t.stats.active_seconds += secs;
                let downloaded = t.progress.downloaded;
                if t.stats.active_seconds > 0 {
                    t.stats.average_speed = downloaded / t.stats.active_seconds;
                }
                t.progress.speed = 0;
                t.progress.instant_speed = 0;
                t.progress.upload_speed = 0;
                t.progress.eta_seconds = None;
                t.progress.active_connections = 0;
                Ok(())
            });
        }
        let pause_requested = run.control.pause.is_cancelled();
        let cancel_requested = run.control.cancel.is_cancelled();
        match outcome {
            TransferOutcome::Completed { file_path, bytes } => {
                self.finish_completed(id, &run, file_path, bytes).await;
            }
            TransferOutcome::Seeding => self.finish_seeding(id).await,
            TransferOutcome::Cancelled => self.finish_cancelled(id).await,
            TransferOutcome::Paused => {
                if cancel_requested {
                    self.finish_cancelled(id).await;
                } else {
                    self.finish_paused(id).await;
                }
            }
            TransferOutcome::Failed(e) => {
                if cancel_requested {
                    self.finish_cancelled(id).await;
                } else if pause_requested {
                    self.finish_paused(id).await;
                } else {
                    self.on_failed(id, e).await;
                }
            }
        }
        let _ = run.finished.send(true);
        self.admission.notify_one();
    }

    async fn finish_paused(&self, id: &TaskId) {
        let Ok((snap, from)) = self.mutate_from(id, |t| {
            if t.blocked_by.is_empty() {
                t.blocked_by.push(PauseReason::User);
            }
            if t.state != TaskState::Paused {
                let blocks = t.blocked_by.clone();
                t.transition(TaskState::Paused)?;
                t.blocked_by = blocks;
            }
            t.status_detail = None;
            Ok(())
        }) else {
            return;
        };
        let _ = self
            .persist
            .commit(PersistOp::State(Box::new(snap.clone())))
            .await;
        if from != snap.state {
            self.publish_state(&snap, from);
        } else {
            self.bus.publish(Event::TaskUpdated(Box::new(snap.clone())));
        }
        self.task_log_line(
            id,
            osprey_domain::events::LogLevel::Info,
            "run.paused",
            format!("paused ({:?})", snap.blocked_by),
        );
        self.automation
            .fire(self, AutomationEvent::DownloadPaused, &snap)
            .await;
    }

    async fn finish_cancelled(&self, id: &TaskId) {
        let Ok((snap, from)) = self.mutate_from(id, |t| {
            if t.state != TaskState::Cancelled {
                t.transition(TaskState::Cancelled)?;
            }
            t.error = None;
            Ok(())
        }) else {
            return;
        };
        let _ = self
            .persist
            .commit(PersistOp::State(Box::new(snap.clone())))
            .await;
        self.publish_state(&snap, from);
        self.task_log_line(
            id,
            osprey_domain::events::LogLevel::Info,
            "run.cancelled",
            "cancelled".into(),
        );
    }

    async fn finish_seeding(&self, id: &TaskId) {
        let Ok((snap, from)) = self.mutate_from(id, |t| {
            if t.state != TaskState::Seeding {
                t.transition(TaskState::Seeding)?;
            }
            Ok(())
        }) else {
            return;
        };
        let _ = self
            .persist
            .commit(PersistOp::State(Box::new(snap.clone())))
            .await;
        if from != snap.state {
            self.publish_state(&snap, from);
        }
    }

    /// Mutate and also return the state before the closure ran.
    pub(crate) fn mutate_from<F>(&self, id: &TaskId, f: F) -> DomainResult<(Task, TaskState)>
    where
        F: FnOnce(&mut Task) -> DomainResult<()>,
    {
        let cell = self.task_cell(id)?;
        let mut t = cell.lock();
        let from = t.state;
        f(&mut t)?;
        t.touch();
        Ok((t.clone(), from))
    }

    /// Verify → process → complete.
    async fn finish_completed(&self, id: &TaskId, run: &RunHandle, file_path: PathBuf, bytes: u64) {
        let settings = self.settings();
        // 1. Verifying
        let Ok((snap, from)) = self.mutate_from(id, |t| {
            if t.state.can_transition_to(TaskState::Verifying) {
                t.transition(TaskState::Verifying)?;
            }
            t.file_path = Some(file_path.clone());
            t.progress.downloaded = bytes.max(t.progress.downloaded);
            if t.progress.total.is_none() || t.progress.total == Some(0) {
                t.progress.total = Some(bytes);
            }
            if !t.kind.is_torrent() {
                t.progress.total = Some(bytes);
            }
            t.progress.fraction = 1.0;
            t.status_detail = None;
            Ok(())
        }) else {
            return;
        };
        self.persist.send(PersistOp::State(Box::new(snap.clone())));
        if from != snap.state {
            self.publish_state(&snap, from);
        }
        let hashable = !snap.kind.is_torrent() && file_path.is_file();
        let mut verified = None;
        if hashable {
            if let Some(expected) = snap.options.checksum.clone() {
                match osprey_runtime::checksum::verify_file(
                    &file_path,
                    &expected,
                    run.control.cancel.clone(),
                )
                .await
                {
                    Ok(c) => verified = Some(c),
                    Err(e) => {
                        self.task_log_line(
                            id,
                            osprey_domain::events::LogLevel::Error,
                            "verify.mismatch",
                            e.message.clone(),
                        );
                        self.notify(Notification::ChecksumMismatch {
                            task_id: id.clone(),
                            name: snap.name.clone(),
                        });
                        if ensure_within(&snap.directory, &file_path).is_ok() {
                            let _ = tokio::fs::remove_file(&file_path).await;
                        }
                        let _ = self.mutate(id, |t| {
                            t.file_path = None;
                            Ok(())
                        });
                        self.on_failed(id, e).await;
                        return;
                    }
                }
            } else if settings.storage.verify_checksum_on_complete {
                match osprey_runtime::checksum::hash_file(
                    &file_path,
                    settings.storage.default_checksum_algorithm,
                    run.control.cancel.clone(),
                    |_| {},
                )
                .await
                {
                    Ok(c) => verified = Some(c),
                    Err(e) => self.task_log_line(
                        id,
                        osprey_domain::events::LogLevel::Warn,
                        "verify.skipped",
                        e.message,
                    ),
                }
            }
        }
        // 2. Processing
        let Ok((mut snap, from)) = self.mutate_from(id, |t| {
            if t.state.can_transition_to(TaskState::Processing) {
                t.transition(TaskState::Processing)?;
            }
            t.verified_checksum = verified.clone();
            Ok(())
        }) else {
            return;
        };
        self.persist.send(PersistOp::State(Box::new(snap.clone())));
        if from != snap.state {
            self.publish_state(&snap, from);
        }
        let mut current_path = file_path.clone();
        // Replace policy: swap the existing file now.
        if snap.options.conflict_policy == ConflictPolicy::Replace && !snap.kind.is_torrent() {
            let target = snap.directory.join(&snap.name);
            if target != current_path && ensure_within(&snap.directory, &target).is_ok() {
                let _ = tokio::fs::remove_file(&target).await;
                if tokio::fs::rename(&current_path, &target).await.is_ok() {
                    current_path = target;
                }
            }
        }
        let mut post = self.post_actions_for(&snap);
        let (recipe_post, recipe_automation) = self.recipe_post_actions(&snap).await;
        post.extend(recipe_post);
        let mut automations = Vec::new();
        let mut platform = Vec::new();
        for a in post {
            match a {
                RuleAction::MoveAfterCompletion { directory } => {
                    let dest_dir = crate::rules::resolve_dir(&snap.directory, &directory);
                    match self.move_completed_file(&current_path, &dest_dir).await {
                        Ok(p) => current_path = p,
                        Err(e) => self.task_log_line(
                            id,
                            osprey_domain::events::LogLevel::Warn,
                            "post.move_failed",
                            e.message,
                        ),
                    }
                }
                RuleAction::FinderTags { tags } => {
                    platform.push(osprey_domain::automation::AutomationAction::FinderTag { tags })
                }
                RuleAction::RevealInFinder => {
                    platform.push(osprey_domain::automation::AutomationAction::RevealInFinder)
                }
                RuleAction::RunAutomation { automation_id } => automations.push(automation_id),
                _ => {}
            }
        }
        if let Some(a) = recipe_automation {
            automations.push(a);
        }
        if snap.options.open_when_done {
            platform.push(osprey_domain::automation::AutomationAction::Open);
        }
        // 3. Completed (transaction)
        let Ok((done, from)) = self.mutate_from(id, |t| {
            t.file_path = Some(current_path.clone());
            if let Some(parent) = current_path.parent() {
                if !t.kind.is_torrent() {
                    t.directory = parent.to_path_buf();
                }
            }
            if let Some(name) = current_path.file_name() {
                if !t.kind.is_torrent() {
                    t.name = name.to_string_lossy().to_string();
                }
            }
            t.transition(TaskState::Completed)?;
            t.error = None;
            t.status_detail = None;
            t.health.remaining_risk = 0;
            Ok(())
        }) else {
            return;
        };
        snap = done;
        let history = self.history_entry_for(&snap);
        if let Err(e) = self
            .persist
            .commit(PersistOp::Complete(
                Box::new(snap.clone()),
                Box::new(history),
            ))
            .await
        {
            tracing::error!(task = %id, error = %e, "completion transaction failed");
        }
        self.publish_state(&snap, from);
        self.task_log_line(
            id,
            osprey_domain::events::LogLevel::Info,
            "run.completed",
            format!("{} bytes -> {}", bytes, current_path.display()),
        );
        self.notify(Notification::Completed {
            task_id: id.clone(),
            name: snap.name.clone(),
            path: current_path.display().to_string(),
        });
        if !self.config.headless {
            for action in platform {
                self.bus.publish(Event::PlatformAction {
                    task_id: Some(id.clone()),
                    action,
                    context: self.automation_context(&snap, AutomationEvent::DownloadCompleted),
                });
            }
        }
        self.automation
            .fire(self, AutomationEvent::DownloadCompleted, &snap)
            .await;
        if snap.verified_checksum.is_some() {
            self.automation
                .fire(self, AutomationEvent::DownloadVerified, &snap)
                .await;
        }
        if snap.kind.is_torrent() {
            self.notify(Notification::TorrentFinished {
                task_id: id.clone(),
                name: snap.name.clone(),
            });
            self.automation
                .fire(self, AutomationEvent::TorrentFinished, &snap)
                .await;
        }
        for aid in automations {
            if let Err(e) = self
                .automation
                .run_by_id(self, &aid, Some(&snap), AutomationEvent::DownloadCompleted)
                .await
            {
                self.task_log_line(
                    id,
                    osprey_domain::events::LogLevel::Warn,
                    "post.automation",
                    e.to_string(),
                );
            }
        }
        self.check_queue_drained(&snap).await;
    }

    /// Move a finished file into `dest_dir` (same-volume rename, copy+delete otherwise).
    async fn move_completed_file(
        &self,
        from: &Path,
        dest_dir: &Path,
    ) -> Result<PathBuf, TaskError> {
        validate_destination_dir(dest_dir)?;
        tokio::fs::create_dir_all(dest_dir)
            .await
            .map_err(|e| TaskError::from_io(&e, "create move destination"))?;
        let name = from
            .file_name()
            .ok_or_else(|| TaskError::new(ErrorKind::InvalidFilename, "file has no name"))?;
        let mut target = dest_dir.join(name);
        if target == from {
            return Ok(target);
        }
        if target.exists() {
            target = unique_path(&target);
        }
        match tokio::fs::rename(from, &target).await {
            Ok(()) => Ok(target),
            Err(_) => {
                tokio::fs::copy(from, &target)
                    .await
                    .map_err(|e| TaskError::from_io(&e, "copy to move destination"))?;
                let _ = tokio::fs::remove_file(from).await;
                Ok(target)
            }
        }
    }

    /// When the last active task of a queue finishes, run the queue's completion action.
    async fn check_queue_drained(&self, last: &Task) {
        let remaining = self.tasks.all().iter().any(|t| {
            t.queue_id == last.queue_id && !t.state.is_terminal() && t.state != TaskState::Pending
        });
        if remaining {
            return;
        }
        let Some(queue) = self.queues.get(&last.queue_id) else {
            return;
        };
        self.automation
            .fire(self, AutomationEvent::QueueFinished, last)
            .await;
        match &queue.completion_action {
            QueueCompletionAction::Nothing => {}
            QueueCompletionAction::Notify => self.notify(Notification::QueueFinished {
                queue_id: queue.id.clone(),
                name: queue.name.clone(),
            }),
            QueueCompletionAction::RunAutomation { automation_id } => {
                let _ = self
                    .automation
                    .run_by_id(
                        self,
                        automation_id,
                        Some(last),
                        AutomationEvent::QueueFinished,
                    )
                    .await;
            }
            QueueCompletionAction::Sleep => {
                self.pause_all_with(PauseReason::Condition("sleep".into()))
                    .await;
                self.bus.publish(Event::ReadyForSleep {
                    reason: format!("queue:{}", queue.name),
                });
            }
            QueueCompletionAction::QuitApplication => {
                self.bus.publish(Event::ReadyForSleep {
                    reason: format!("quit:{}", queue.name),
                });
            }
        }
    }

    /// Classify a failure and decide what happens next.
    pub(crate) async fn on_failed(&self, id: &TaskId, mut err: TaskError) {
        err.message = redact(&err.message);
        err.detail = err.detail.map(|d| redact(&d));
        err.source_url = err.source_url.map(|u| redact(&u));
        let settings = self.settings();
        let class = err.class();
        let Ok(snap) = self.snapshot(id) else {
            return;
        };
        let backoff = {
            let mut b = BackoffPolicy::from_settings(&settings.network);
            if let Some(n) = snap.options.max_retries {
                b.max_retries = n;
            }
            b
        };
        enum Next {
            Wait(PauseReason),
            Restart,
            Degrade,
            Retry(Duration),
            Fail,
        }
        let next = match class {
            FailureClass::WaitForCondition => Next::Wait(match err.kind {
                ErrorKind::DiskFull => PauseReason::DiskSpace,
                ErrorKind::VolumeUnavailable => PauseReason::VolumeUnavailable,
                _ => PauseReason::NetworkUnavailable,
            }),
            FailureClass::RestartFromScratch if snap.attempt < MAX_RESTARTS => Next::Restart,
            FailureClass::Degrade => Next::Degrade,
            FailureClass::Transient | FailureClass::Throttled | FailureClass::SourceProblem => {
                let attempt = snap.attempt + 1;
                match backoff.delay_for_with_hint(
                    attempt,
                    class,
                    err.retry_after_ms.map(Duration::from_millis),
                ) {
                    Some(d) => Next::Retry(d),
                    None => Next::Fail,
                }
            }
            _ => Next::Fail,
        };
        let log_level = osprey_domain::events::LogLevel::Warn;
        match next {
            Next::Wait(reason) => {
                let Ok((s, from)) = self.mutate_from(id, |t| {
                    t.error = Some(err.clone());
                    let blocks = t.blocked_by.clone();
                    if t.state != TaskState::Paused {
                        t.transition(TaskState::Paused)?;
                    }
                    t.blocked_by = blocks;
                    t.block(reason.clone());
                    Ok(())
                }) else {
                    return;
                };
                let _ = self
                    .persist
                    .commit(PersistOp::State(Box::new(s.clone())))
                    .await;
                self.publish_state(&s, from);
                self.task_log_line(
                    id,
                    log_level,
                    "run.waiting",
                    format!("{}: waiting for {reason:?}", err.message),
                );
            }
            Next::Restart => {
                self.delete_part(&snap).await;
                let _ = self
                    .persist
                    .commit(PersistOp::DeleteCheckpoint(id.clone()))
                    .await;
                let Ok((s, from)) = self.mutate_from(id, |t| {
                    t.segment_map = None;
                    if let Some(m) = &mut t.media {
                        m.segments_done = 0;
                    }
                    t.progress = Progress {
                        total: t.progress.total,
                        ..Progress::default()
                    };
                    t.attempt += 1;
                    t.transition(TaskState::Queued)?;
                    t.error = Some(err.clone());
                    Ok(())
                }) else {
                    return;
                };
                let _ = self
                    .persist
                    .commit(PersistOp::Update(Box::new(s.clone())))
                    .await;
                self.publish_state(&s, from);
                self.task_log_line(
                    id,
                    log_level,
                    "run.restart",
                    format!("{}: restarting from scratch", err.message),
                );
            }
            Next::Degrade => {
                let Ok((s, from)) = self.mutate_from(id, |t| {
                    t.options.max_connections = Some(1);
                    t.transition(TaskState::Queued)?;
                    t.error = Some(err.clone());
                    Ok(())
                }) else {
                    return;
                };
                let _ = self
                    .persist
                    .commit(PersistOp::Update(Box::new(s.clone())))
                    .await;
                self.publish_state(&s, from);
                self.task_log_line(
                    id,
                    log_level,
                    "run.degrade",
                    format!("{}: continuing with one connection", err.message),
                );
            }
            Next::Retry(delay) => {
                let at = Millis::now().saturating_add_ms(delay.as_millis() as i64);
                let Ok((s, from)) = self.mutate_from(id, |t| {
                    t.attempt += 1;
                    t.stats.retries += 1;
                    if t.state != TaskState::Retrying {
                        t.transition(TaskState::Retrying)?;
                    }
                    t.next_retry_at = Some(at);
                    t.error = Some(err.clone());
                    t.status_detail = Some(err.message.clone());
                    Ok(())
                }) else {
                    return;
                };
                let _ = self
                    .persist
                    .commit(PersistOp::State(Box::new(s.clone())))
                    .await;
                if from != s.state {
                    self.publish_state(&s, from);
                } else {
                    self.bus.publish(Event::TaskUpdated(Box::new(s.clone())));
                }
                self.task_log_line(
                    id,
                    log_level,
                    "retry.scheduled",
                    format!(
                        "{} — retry {} in {:.0} s",
                        err.message,
                        s.attempt,
                        delay.as_secs_f64()
                    ),
                );
                let weak = self.this.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(delay).await;
                    if let Some(e) = weak.upgrade() {
                        e.admission.notify_one();
                    }
                });
            }
            Next::Fail => {
                let Ok((s, from)) = self.mutate_from(id, |t| {
                    if t.state != TaskState::Failed {
                        t.transition(TaskState::Failed)?;
                    }
                    t.error = Some(err.clone());
                    t.status_detail = None;
                    Ok(())
                }) else {
                    return;
                };
                let _ = self
                    .persist
                    .commit(PersistOp::State(Box::new(s.clone())))
                    .await;
                self.publish_state(&s, from);
                self.task_log_line(
                    id,
                    osprey_domain::events::LogLevel::Error,
                    "run.failed",
                    format!("{:?}: {}", err.kind, err.message),
                );
                self.notify(Notification::Failed {
                    task_id: id.clone(),
                    name: s.name.clone(),
                    reason: err.message.clone(),
                });
                self.automation
                    .fire(self, AutomationEvent::DownloadFailed, &s)
                    .await;
            }
        }
    }

    // ---------------------------------------------------------------------------------------
    // sink handlers
    // ---------------------------------------------------------------------------------------

    pub(crate) fn sink_progress(&self, id: &TaskId, p: Progress) {
        let Some(cell) = self.tasks.get(id) else {
            return;
        };
        let update = {
            let mut t = cell.lock();
            if t.kind.is_torrent() || p.downloaded >= t.progress.downloaded {
                t.progress.downloaded = p.downloaded;
            }
            if p.uploaded > 0 {
                t.progress.uploaded = p.uploaded;
            }
            if p.total.is_some() {
                t.progress.total = p.total;
            }
            t.progress.peers = p.peers;
            t.progress.seeds = p.seeds;
            t.progress.ratio = p.ratio;
            if p.fraction > 0.0 {
                t.progress.fraction = p.fraction;
            }
            if p.active_connections > 0 {
                t.progress.active_connections = p.active_connections;
            }
            if p.eta_seconds.is_some() && t.progress.eta_seconds.is_none() {
                t.progress.eta_seconds = p.eta_seconds;
            }
            osprey_domain::ProgressUpdate {
                task_id: id.clone(),
                progress: t.progress.clone(),
                rev: t.rev,
            }
        };
        self.bus.progress(update);
    }

    pub(crate) fn sink_state(
        &self,
        id: &TaskId,
        run_id: u64,
        state: TaskState,
        detail: Option<String>,
    ) {
        let Some(cell) = self.tasks.get(id) else {
            return;
        };
        let result = {
            let mut t = cell.lock();
            if t.state == state {
                if t.status_detail != detail {
                    t.status_detail = detail;
                    t.touch();
                    Some((t.clone(), None))
                } else {
                    None
                }
            } else if t.state.can_transition_to(state) {
                let from = t.state;
                if t.transition(state).is_err() {
                    None
                } else {
                    t.status_detail = detail;
                    Some((t.clone(), Some(from)))
                }
            } else {
                tracing::debug!(task = %id, from = %t.state, to = %state, "engine state ignored");
                None
            }
        };
        let Some((snap, from)) = result else {
            return;
        };
        self.persist.send(PersistOp::State(Box::new(snap.clone())));
        match from {
            Some(from) => self.publish_state(&snap, from),
            None => self.bus.publish(Event::TaskUpdated(Box::new(snap.clone()))),
        }
        if let Some(from) = from {
            let weak = self.this.clone();
            let event = match (state, snap.kind.is_torrent()) {
                (TaskState::Downloading, true) => Some(AutomationEvent::TorrentStarted),
                (TaskState::Downloading, false) => Some(AutomationEvent::DownloadStarted),
                _ => None,
            };
            if from == TaskState::Retrying || from == TaskState::Paused {
                // resumed after a retry/pause
                if state == TaskState::Downloading {
                    let s2 = snap.clone();
                    let w2 = weak.clone();
                    tokio::spawn(async move {
                        if let Some(e) = w2.upgrade() {
                            if e.tasks.is_current_run(&s2.id, run_id) {
                                e.automation
                                    .fire(&e, AutomationEvent::DownloadResumed, &s2)
                                    .await;
                            }
                        }
                    });
                }
            }
            if let Some(ev) = event {
                let s = snap.clone();
                tokio::spawn(async move {
                    if let Some(e) = weak.upgrade() {
                        e.automation.fire(&e, ev, &s).await;
                    }
                });
            }
            if state == TaskState::Seeding {
                self.notify(Notification::TorrentFinished {
                    task_id: id.clone(),
                    name: snap.name.clone(),
                });
                let weak = self.this.clone();
                let s = snap.clone();
                tokio::spawn(async move {
                    if let Some(e) = weak.upgrade() {
                        e.automation
                            .fire(&e, AutomationEvent::TorrentFinished, &s)
                            .await;
                    }
                });
            }
        }
    }

    pub(crate) fn sink_metadata(&self, id: &TaskId, m: ResolvedMetadata) {
        let Some(cell) = self.tasks.get(id) else {
            return;
        };
        let snap = {
            let mut t = cell.lock();
            if let Some(name) = m.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                if !t.name_locked {
                    t.name = sanitize_filename(name);
                }
            }
            if let Some(total) = m.total {
                t.progress.total = Some(total);
            }
            if let Some(mime) = &m.mime {
                t.mime = Some(mime.split(';').next().unwrap_or("").trim().to_owned());
            }
            if let Some(r) = m.resumable {
                t.stats.range_supported = Some(r);
            }
            if m.final_url.is_some() {
                t.stats.final_url = m.final_url.clone().map(|u| redact(&u));
            }
            if m.etag.is_some() {
                t.stats.etag = m.etag.clone();
            }
            if m.last_modified.is_some() {
                t.stats.last_modified = m.last_modified.clone();
            }
            if m.server.is_some() {
                t.stats.server = m.server.clone();
            }
            if m.content_disposition.is_some() {
                t.stats.content_disposition = m.content_disposition.clone();
            }
            if m.http_version.is_some() {
                t.stats.http_version = m.http_version.clone();
            }
            if m.remote_addr.is_some() {
                t.stats.remote_addr = m.remote_addr.clone();
            }
            if let Some(ct) = &m.mime {
                t.stats.content_type = Some(ct.clone());
            }
            if let Some(tor) = m.torrent {
                if !tor.name.is_empty() && !t.name_locked {
                    t.name = sanitize_filename(&tor.name);
                }
                if tor.total_size > 0 {
                    t.progress.total = Some(tor.total_size);
                }
                t.torrent = Some(tor);
            }
            if let Some(media) = m.media {
                if let Some(title) = media.title.as_deref().filter(|s| !s.is_empty()) {
                    if !t.name_locked && t.kind == TaskKind::Hls {
                        let ext = t.extension().unwrap_or_else(|| "mp4".into());
                        let stem = sanitize_filename(title);
                        if Path::new(&stem).extension().is_none() {
                            t.name = format!("{stem}.{ext}");
                        }
                    }
                }
                t.media = Some(media);
            }
            if let Some(p) = m.file_path {
                if ensure_within(&t.directory, &p).is_ok() {
                    t.file_path = Some(p);
                }
            }
            if t.category_id.is_none() {
                drop(t);
                let auto = {
                    let t2 = cell.lock();
                    self.auto_category(&t2)
                };
                let mut t = cell.lock();
                t.category_id = auto;
                t.touch();
                t.clone()
            } else {
                t.touch();
                t.clone()
            }
        };
        self.persist.send(PersistOp::Update(Box::new(snap.clone())));
        self.bus.publish(Event::TaskUpdated(Box::new(snap)));
    }

    pub(crate) fn sink_checkpoint(&self, id: &TaskId, run_id: u64, cp: Checkpoint) {
        let Some(run) = self.tasks.run(id) else {
            return;
        };
        if run.run_id != run_id {
            return;
        }
        if let Checkpoint::Segments(map) = &cp {
            if let Some(cell) = self.tasks.get(id) {
                cell.lock().segment_map = Some(map.clone());
            }
        }
        if let Checkpoint::Torrent(tc) = &cp {
            if let Some(cell) = self.tasks.get(id) {
                let mut t = cell.lock();
                t.progress.uploaded = tc.uploaded.max(t.progress.uploaded);
                if let Some(info) = &mut t.torrent {
                    info.uploaded = tc.uploaded;
                    info.seeding_since = tc.seeding_since;
                }
            }
        }
        *run.pending_checkpoint.lock() = Some(cp);
        if !run.checkpoint_armed.swap(true, Ordering::Relaxed) {
            let weak = self.this.clone();
            let tid = id.clone();
            tokio::spawn(async move {
                tokio::time::sleep(CHECKPOINT_DEBOUNCE).await;
                if let Some(e) = weak.upgrade() {
                    if let Some(run) = e.tasks.run(&tid).filter(|r| r.run_id == run_id) {
                        e.flush_checkpoint(&tid, &run).await;
                    }
                }
            });
        }
    }

    /// Persist the pending checkpoint now.
    pub(crate) async fn flush_checkpoint(&self, id: &TaskId, run: &RunHandle) {
        run.checkpoint_armed.store(false, Ordering::Relaxed);
        let pending = run.pending_checkpoint.lock().take();
        if let Some(cp) = pending {
            if self.tasks.contains(id) {
                let _ = self
                    .persist
                    .commit(PersistOp::Checkpoint(id.clone(), Box::new(cp)))
                    .await;
            }
        }
    }

    pub(crate) fn sink_stat(&self, id: &TaskId, stat: EngineStat) {
        let Some(cell) = self.tasks.get(id) else {
            return;
        };
        let run = self.tasks.run(id);
        let mut t = cell.lock();
        let mut accum = run.as_ref().map(|r| r.health.lock());
        match stat {
            EngineStat::ConnectionOpened => {
                if let Some(a) = accum.as_deref_mut() {
                    a.successful_connections += 1;
                    a.samples += 1;
                }
            }
            EngineStat::ConnectionFailed => {
                t.stats.failed_connections += 1;
                if let Some(a) = accum.as_deref_mut() {
                    a.failed_connections += 1;
                }
            }
            EngineStat::Retry => {
                t.stats.retries += 1;
                if let Some(a) = accum.as_deref_mut() {
                    a.retries += 1;
                }
            }
            EngineStat::SegmentReassigned => t.stats.segments_reassigned += 1,
            EngineStat::MirrorSwitched { .. } => {
                t.stats.mirrors_switched += 1;
                if let Some(a) = accum.as_deref_mut() {
                    a.mirrors_switched += 1;
                }
            }
            EngineStat::Throttled => {
                if let Some(a) = accum.as_deref_mut() {
                    a.throttled_events += 1;
                }
            }
            EngineStat::ThroughputDrop => {
                t.stats.throughput_drops += 1;
                if let Some(a) = accum.as_deref_mut() {
                    a.throughput_drops += 1;
                }
            }
            EngineStat::RangeSupport { supported } => t.stats.range_supported = Some(supported),
            EngineStat::BytesDiscarded { bytes } => t.stats.bytes_discarded += bytes,
            EngineStat::Peak { speed } => t.stats.peak_speed = t.stats.peak_speed.max(speed),
        }
    }
}

/// Reject anything but http(s) for control-plane URLs.
fn check_http_url(u: &str) -> DomainResult<()> {
    let parsed =
        url::Url::parse(u).map_err(|e| DomainError::validation(format!("invalid URL: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(DomainError::validation(format!(
            "unsupported URL scheme {:?}",
            parsed.scheme()
        )));
    }
    Ok(())
}

/// `dn=` display name of a magnet, or its info hash.
fn magnet_display_name(uri: &str) -> String {
    url::Url::parse(uri)
        .ok()
        .and_then(|u| {
            u.query_pairs()
                .find(|(k, _)| k == "dn")
                .map(|(_, v)| v.to_string())
                .or_else(|| {
                    u.query_pairs()
                        .find(|(k, _)| k == "xt")
                        .map(|(_, v)| v.rsplit(':').next().unwrap_or("magnet").to_owned())
                })
        })
        .unwrap_or_else(|| "magnet".to_owned())
}

/// ` (2)`-style unique name inside `dir`.
pub(crate) fn unique_name(dir: &Path, name: &str) -> String {
    let p = unique_path(&dir.join(name));
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| name.to_owned())
}
