# FFI surface (UniFFI → Swift)

Crate `swoop-ffi` exposes the engine to the macOS app in-process. Design rules from the
architecture review:

1. **Never lower a full `Task` unasked.** The table binds to `FfiTaskRow` (~200 B); details are
   fetched on selection with `task_detail(id) -> FfiTaskDetail` (includes segments, torrent files,
   trackers, media variants, stats, health breakdown, log tail).
2. **Snapshot + revision + resync.** `snapshot() -> FfiSnapshot { rev, rows, queues, categories,
   stats }` on launch; afterwards the app applies `FfiEvent`s. Each row/update carries `rev`; the
   app ignores anything older than what it holds. When the Rust forwarder's broadcast receiver lags
   it emits `FfiEvent::Resync` and the app reloads the snapshot.
3. **Events are delivered in batches from a dedicated forwarder task**, never from the publisher's
   thread: `add_listener(listener: EventListener)` where `EventListener::on_events(events:
   Vec<FfiEvent>)` is a UniFFI callback interface (`#[uniffi::export(with_foreign)]`). Batches are
   flushed at most every 100 ms. Swift must hop to the main actor asynchronously inside the callback
   (`Task { @MainActor in … }`) — never block.
4. **All async methods run on the engine's Tokio runtime**: the FFI wraps every call in
   `rt.spawn(async move { … }).await` (a helper macro `ffi_async!`) because UniFFI polls futures on
   the foreign executor.
5. **Errors** cross as `FfiError` (`#[derive(uniffi::Error)]`, `#[uniffi(flat_error)]` not used —
   keep the typed variants: `NotFound`, `Validation`, `Conflict`, `PermissionDenied`, `Storage`,
   `Engine`, `Unavailable`, `Internal`, each with `message`). Panics are caught at the boundary
   (`catch_unwind`) and reported as `Internal`.
6. **Records mirror the domain but are flat and FFI-friendly**: `String` ids, `i64` millis,
   `Option<u64>` sizes, enums as UniFFI enums (`FfiTaskState`, `FfiTaskKind`, `FfiPriority`,
   `FfiErrorKind`, `FfiTrafficMode`, `FfiScope`, …). JSON is used only for opaque payloads
   (rule conditions/actions, automation actions, schedule recurrence, settings) — those cross as
   JSON strings with typed helpers on the Swift side; this keeps the surface stable while the
   domain evolves. (`settings_json()/update_settings_json(json)`, `rules_json()`, …)
7. **Platform actions come back as events**: `FfiEvent::PlatformAction { task_id, action_json,
   context_json }` — Finder tag, reveal, open, AppleScript. `FfiEvent::Notification { … }` for the
   notification centre. `FfiEvent::ReadyForSleep`.
8. **Platform inputs go in through methods**: `update_environment(FfiEnvironment)`,
   `set_active_window(bool)` (quiet notifications), `store_credential(name, username, secret)`
   (the FFI writes to the keychain through the Rust `keyring` crate so headless behaves the same).

## Object: `SwoopEngine`

```
constructor open(config: FfiEngineConfig) throws FfiError      // data_dir, headless=false, log level, app version, bundle id
fn info() -> FfiEngineInfo
fn add_listener(listener: EventListener) -> u64 ; fn remove_listener(id: u64)
async fn snapshot() -> FfiSnapshot
async fn task_rows(filter: FfiTaskFilter) -> Vec<FfiTaskRow>
async fn task_detail(id: String) throws -> FfiTaskDetail
async fn probe(request: FfiNewTaskRequest) throws -> FfiProbeResult
async fn add_task(request: FfiNewTaskRequest) throws -> FfiAddTaskResult
async fn add_tasks(requests: Vec<FfiNewTaskRequest>) throws -> Vec<FfiAddTaskResult>
async fn task_action(id: String, action: FfiTaskAction) throws -> FfiTaskRow   // Start, Pause, Resume, Restart, Retry, Cancel, Redownload, Verify, RetrySegments, Duplicate
async fn tasks_action(ids: Vec<String>, action: FfiTaskAction) throws -> u32
async fn remove_tasks(ids: Vec<String>, delete_file: bool) throws -> u32
async fn update_task(id: String, patch: FfiTaskPatch) throws -> FfiTaskRow
async fn resolve_duplicate(id: String, policy: FfiConflictPolicy) throws -> FfiTaskRow
async fn reorder_tasks(ids: Vec<String>, after: Option<String>) throws
async fn set_task_limit / set_task_connections / set_task_priority
async fn task_log(id, limit) -> Vec<FfiLogEntry> ; async fn diagnostics_text(id) throws -> String
async fn pause_all / resume_all / retry_all_failed / clear_completed -> u32
// torrents
async fn set_torrent_files(id, Vec<FfiFileSelection>) ; set_torrent_sequential ; set_seeding_limits(id, FfiSeedingLimits)
async fn torrent_peers(id) -> Vec<FfiPeer> ; add_trackers ; remove_tracker ; set_tracker_enabled ; reannounce ; refresh_tracker_list
// media
async fn detect_media(url, page_url) throws -> FfiDetectedMedia
// queues / categories / rules / schedules / automations / recipes (JSON in, JSON out where the shape is complex)
async fn queues() -> Vec<FfiQueue> ; save_queue(FfiQueue) ; delete_queue(id, move_to) ; pause_queue ; resume_queue ; reorder_queues
async fn categories() -> Vec<FfiCategory> ; save_category ; delete_category
async fn rules_json() -> String ; save_rule_json(json) -> String ; delete_rule(id) ; test_rules_json(subject_json) -> String
async fn schedules_json / save_schedule_json / delete_schedule
async fn automations_json / save_automation_json / delete_automation / grant_automation_consent(id) / automation_runs_json(id?, limit) / run_automation(id, task_id)
async fn recipes_json / save_recipe_json / delete_recipe / apply_recipe(id, request)
// history
async fn history(query: FfiHistoryQuery) -> Vec<FfiHistoryEntry> ; history_count ; delete_history(ids) ; clear_history
// bandwidth & settings
async fn set_traffic_mode(FfiTrafficMode) ; set_global_limits(down, up) ; optimize() -> String(settings json)
fn settings_json() -> String ; async fn update_settings_json(json) throws -> String
async fn store_credential(name, username?, secret) -> String ; credentials() -> Vec<FfiCredential> ; delete_credential(id)
// stats
fn global_stats() -> FfiGlobalStats ; async fn dashboard() -> FfiDashboard ; async fn disk_info(path?) -> FfiDiskInfo
// devices
async fn devices() ; start_pairing(scopes) -> FfiPairingInfo ; cancel_pairing ; revoke_device ; rename_device ; audit_log(limit)
// grabber
async fn grabber_start(FfiGrabberOptions) -> FfiGrabberSession ; grabber_status(id) ; grabber_cancel(id) ; grabber_add(id, urls, request) ; grabber_list()
// archives
async fn archive_list(path) -> FfiArchiveListing ; archive_extract(path, entries?, destination) -> u32
// import/export
async fn export_json(tasks, history) -> String ; import_json(json, FfiImportOptions) -> FfiImportReport
// updates / plugins / logs
async fn check_for_updates() -> FfiUpdateInfo ; download_update() -> String(path)
async fn check_app_update(manual) -> FfiUpdateInfo ; fn update_progress() -> FfiUpdateProgress
async fn stage_update(bundle_path) -> String(staged bundle) ; fn releases_page_url() -> String
async fn plugins() ; set_plugin_enabled ; uninstall_plugin
async fn recent_logs(limit, level?) -> Vec<String>
// platform inputs
fn update_environment(env: FfiEnvironment) ; fn set_active_window(active: bool)
async fn shutdown()
```

`FfiEvent` variants: `TaskAdded(FfiTaskRow)`, `TaskUpdated(FfiTaskRow)`, `TaskRemoved{id}`,
`TaskStateChanged{id, from, to}`, `Progress(Vec<FfiProgress>)`, `TaskLog(FfiLogEntry)`,
`QueueUpdated(FfiQueue)`, `QueueRemoved{id}`, `QueueSummaries(Vec<FfiQueueSummary>)`,
`CategoriesChanged`, `RulesChanged`, `SchedulesChanged`, `AutomationsChanged`, `RecipesChanged`,
`SettingsChanged`, `GlobalStats(FfiGlobalStats)`, `Notification(FfiNotification)`,
`DevicesChanged`, `PairingStarted{code, expires_at, url}`, `PairingCompleted{device_name}`,
`DiskSpace{path, free}`, `NetworkChanged{available, metered}`, `PlatformAction{task_id, action_json, context_json}`,
`GrabberProgress{session_id, pages, files, done}`, `UpdateCheck{available, version, notes}`,
`ReadyForSleep{reason}`, `Resync`, `EngineStopping`.
