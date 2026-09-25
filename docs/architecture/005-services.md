# Services layer design (`swoop-services`)

`Engine` implements `EngineApi` and owns every long-lived component:

```
Engine
├── paths: AppPaths, instance lock (InstanceLock)
├── store: Arc<swoop_store::Store>          (recover() at start, per 002-persistence-and-recovery.md)
├── settings: ArcSwap-like RwLock<Arc<Settings>>   (validated; persisted; SettingsChanged event)
├── bus: EventBus (250 ms progress coalescing, 8192 capacity)
├── clients: Arc<ClientFactory>                (updated on settings change with resolved global proxy URL)
├── engines: HashMap<TaskKind, Arc<dyn Transfer>>  (Http, Metalink → HttpEngine; Ftp → FtpEngine; Torrent, Magnet → TorrentEngine; Hls → HlsEngine)
├── tasks: TaskTable  (DashMap<TaskId, Arc<Mutex<Task>>> + per-task RunHandle {control, join, sink, limiter})
├── queues: QueueManager (slots per queue: max_concurrent, paused; global max_active; admission order)
├── bandwidth: BandwidthManager (global limiter → per-queue limiters → per-task; traffic mode; measured capacity)
├── scheduler: Scheduler (60 s tick + boundary timers; EnvironmentSnapshot with hysteresis; gate tasks/queues; run ScheduleActions; ReadyForSleep)
├── rules: RulesEngine (apply at add time + MoveAfterCompletion/FinderTags/RunAutomation at completion)
├── automation: AutomationRunner (swoop-automation crate executor + consent gate + platform actions as events)
├── history + duplicates (store lookups by path/name+size/checksum/url)
├── disk: DiskMonitor (30 s tick: free space per destination, low-space notifications, volume presence → block/unblock tasks with VolumeUnavailable)
├── diagnostics: per-task log ring (last 500 in memory, persisted via store), health inputs → HealthScore recompute every 5 s for active tasks
├── devices: pairing (code, TTL 120 s, single-use), tokens (32 random bytes → base64url; sha256 hex stored), lockout per IP
├── credentials: keyring (service "app.swoop.desktop", account = CredentialId) + store credential index
├── grabber: swoop_grabber::Crawler
├── archives: swoop-archive crate
├── updater: swoop-update crate
├── plugins: swoop-plugins crate
└── logs: tracing layer writing a 2,000-line ring buffer (redacted) + daily rolling file in log_dir
```

## Task lifecycle (TaskManager)

- `add_task`: build `Task` from `NewTaskRequest` (kind detection: magnet: → Magnet; torrent_base64 → parse via TorrentEngine::parse_torrent, store blob, Source::TorrentFile; metalink_url / .metalink → Metalink; .m3u8 / hls_playlist_url → Hls; ftp(s):// → Ftp; http(s):// → Http; else Validation error). Sanitise name (`safety::sanitize_filename`), resolve directory (request → recipe → category dir (if `organise_by_category`) → queue dir → settings.download_directory, expand `~`), `validate_destination_dir`. Apply rules (RulesEngine::apply → may change dir/name/queue/category/tags/priority/limits/connections; deterministic order by (priority, created_at)). Category auto-assign by extension/MIME when none. Duplicate detection when `settings.storage.duplicate_detection`: exact final path exists → `path`; active/queued task with same URL → `url_history` (existing_task_id); history entry with same URL → `url_history`; history name+size when size known → `size`; policy from `request.options.conflict_policy` or settings default: Ask → task stays Pending with `AddTaskResult.duplicate` + `Notification::DuplicateDetected`; Replace → proceed (file removed at completion rename time); Rename/KeepBoth → `unique_path` name; Skip → return the existing task without creating. Persist (`insert_task`), publish `TaskAdded`, then if `start` → `enqueue` (state Queued, or Scheduled if the queue/task has a schedule whose window is closed).
- Admission loop (`QueueManager::tick`, triggered on any state change and every 1 s): for each queue ordered by `priority DESC, position`, while `active < max_concurrent` and global active < `bandwidth.max_active_downloads` (torrents count against `max_active_torrents`), pick the next `Queued` task ordered by `priority DESC, position ASC` whose `blocked_by` is empty and whose `next_retry_at` is past → `start_run`.
- `start_run`: fresh `TransferControl` (limits from task options/queue/global), `TaskSink` (implements ProgressSink → bus.progress with rev, state changes → transition + store.update_task_state + `TaskStateChanged` + `TaskUpdated`; metadata → merge unless name/directory locked; checkpoint → store.upsert_checkpoint debounced 3 s + immediate on pause; log → ring + store batch; stat → health inputs), spawn `engine.run(ctx)`; on outcome:
  - Completed → `Verifying` (if `options.checksum` → `checksum::verify_file`; else if `verify_checksum_on_complete` compute default algorithm hash for history/dedup) → `Processing` (rules post actions: MoveAfterCompletion, DateFolder/DomainFolder already applied at add; FinderTags/RevealInFinder/RunAutomation → PlatformAction events) → `store.complete_task` (transaction) → `Completed` + `Notification::Completed` + automation events (DownloadCompleted, DownloadVerified, TorrentFinished) + queue completion action when the queue drains.
  - Seeding → state Seeding (task remains "active" but not counted against download slots).
  - Paused → `Paused` (blocked_by must already contain the reason set by the pauser).
  - Cancelled → `Cancelled`.
  - Failed(e) → classify: `WaitForCondition` → Paused with DiskSpace/VolumeUnavailable/NetworkUnavailable block (DiskMonitor/network watcher unblock later); `RestartFromScratch` → discard checkpoint + part file, attempt += 1 (max 2 restarts) → Queued; `Degrade` → set `options.max_connections = 1`, keep checkpoint → Queued; retryable classes → `Retrying` with `next_retry_at = now + BackoffPolicy(delay with retry_after hint)`; else `Failed` + `Notification::Failed` + automation `DownloadFailed`.
- Pause: add `PauseReason::User`, `control.pause.cancel()`; if not running just transition. Resume: remove User block; if `blocked_by` empty → Queued. Queue pause: block all its non-terminal tasks with `Queue(name)`; resume removes. Schedule gating: `Schedule(name)`; condition: `Condition(desc)`.
- Shutdown: cancel timers, pause every running task with `Shutdown` block (engines flush + checkpoint), wait ≤ 10 s, `store.flush`, torrent engine shutdown, store close.
- Recovery at start: `store.recover(temp_suffix)` then: `ready_to_complete` → run completion path (verify/complete); `torrents_to_readd` handled lazily by TorrentEngine on run; `paused_by_shutdown` → remove Shutdown block → Queued.

## Progress/stats ticker (250 ms)
Reads `control.counters` for every running task, feeds a `SpeedMeter` per task (kept in RunHandle), computes speed/eta, updates `task.progress`, `bus.progress(ProgressUpdate{rev})`, `store.progress_writer().update`. Every 1 s: `GlobalStats` (sum speeds, counts by state, completed/failed today from history, free space of default dir, traffic mode/limits) → event + speed sample every 10 s → store.

## Bandwidth
`RateLimiter` tree: global ("global") → per queue ("queue:<id>") → per task. `set_traffic_mode`/`set_global_limits` update settings + global limiter; queue.bandwidth updates the queue limiter; task limit updates its control + limiter. Torrent limits are applied through `TransferControl.download_limit/upload_limit` (engine polls). `optimize()`: measure capacity as the peak aggregate speed over the last 10 min of samples (fallback: current), set `measured_capacity`, choose `connections_per_task` = clamp(capacity / 2 MB/s, 4, 16) when servers support ranges (from recent tasks' `range_supported`), enable adaptive, persist, return settings.

## Remote auth (used by swoop-server)
`authenticate(token, ip)`: sha256(token) → store.find_device_by_token_hash → not revoked and not expired → touch → Device. Local token: file `local-api.token` (created 0600 on first start, 32 random bytes base64url) → `authenticate` also accepts it and returns a synthetic admin Device{id:"local", kind:"cli"}. Pairing: `start_pairing(scopes)` → code from alphabet `ABCDEFGHJKLMNPQRSTUVWXYZ23456789` (8 chars, shown as `XXXX-XXXX`), expires 120 s, `PairingStarted` event; `complete_pairing(code, name, kind, ip)`: constant-time compare, lockout per IP (`max_failed_attempts` → `lockout_minutes`), single-use → create Device (expires per `session_ttl_hours`), token returned once, `PairingCompleted` + `Notification::DevicePaired`, audit row.

## Import/export
`ExportBundle` from store lists (tasks optional, history optional); `import` upserts with id conflict handling (`overwrite` or skip), automations imported without consent, settings validated, report counts.
