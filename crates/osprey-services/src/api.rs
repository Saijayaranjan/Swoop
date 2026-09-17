//! The engine command surface. Every front door (Swift app via FFI, REST/WS server, CLI,
//! browser native host) calls exactly these methods, so behaviour is identical everywhere.
//!
//! Conventions:
//! * Methods are `async`; none blocks the caller for I/O beyond a SQLite write.
//! * All mutations emit [`Event`]s on the bus returned by [`EngineApi::subscribe`]; callers
//!   should render from events, not from the return values.
//! * Ids are opaque strings (UUIDs). Missing ids → `DomainError::NotFound`.

use async_trait::async_trait;
use osprey_domain::automation::{AutomationRule, AutomationRun};
use osprey_domain::category::Category;
use osprey_domain::device::{AuditEntry, Device, Scope};
use osprey_runtime::engine::ResolvedMetadata;
use osprey_domain::events::{GlobalStats, TaskLogEntry};
use osprey_domain::history::{HistoryEntry, HistoryQuery};
use osprey_domain::media::DetectedMedia;
use osprey_domain::queue::{Queue, QueueSummary, TrafficMode};
use osprey_domain::rules::{Rule, RuleSubject};
use osprey_domain::schedule::{EnvironmentSnapshot, Schedule};
use osprey_domain::settings::Settings;
use osprey_domain::torrent::PeerInfo;
use osprey_domain::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

pub use osprey_runtime::bus::EventSubscription;

// ---------------------------------------------------------------------------------------------
// Query / filter types
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskFilter {
    /// Free text over name, URL, domain, tags.
    pub text: Option<String>,
    pub states: Vec<TaskState>,
    pub kinds: Vec<TaskKind>,
    pub queue_id: Option<QueueId>,
    pub category_id: Option<CategoryId>,
    pub domain: Option<String>,
    pub tag: Option<String>,
    /// `active`, `queued`, `scheduled`, `complete`, `failed`, `torrent`, `media`, `paused` — UI smart filters.
    pub smart: Option<String>,
    pub created_since: Option<Millis>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub sort: TaskSort,
    pub descending: bool,
    /// Paging: `limit == 0` means no limit.
    pub limit: u32,
    pub offset: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskSort {
    /// Manual queue order (position) — the default for the Downloads view.
    #[default]
    Position,
    CreatedAt,
    Name,
    Size,
    Progress,
    Speed,
    Eta,
    State,
    Domain,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskPage {
    pub tasks: Vec<Task>,
    pub total: u32,
}

/// Lightweight row for large lists (the Swift table binds to these; full `Task` on selection).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskRow {
    pub id: TaskId,
    pub name: String,
    pub kind: TaskKind,
    pub state: TaskState,
    pub domain: Option<String>,
    pub progress: Progress,
    pub queue_id: QueueId,
    pub category_id: Option<CategoryId>,
    pub priority: Priority,
    pub position: i64,
    pub created_at: Millis,
    pub error_kind: Option<ErrorKind>,
    pub health: u8,
    pub file_path: Option<PathBuf>,
}

impl From<&Task> for TaskRow {
    fn from(t: &Task) -> Self {
        Self {
            id: t.id.clone(),
            name: t.name.clone(),
            kind: t.kind,
            state: t.state,
            domain: t.domain(),
            progress: t.progress.clone(),
            queue_id: t.queue_id.clone(),
            category_id: t.category_id.clone(),
            priority: t.priority,
            position: t.position,
            created_at: t.created_at,
            error_kind: t.error.as_ref().map(|e| e.kind),
            health: t.health.score,
            file_path: t.file_path.clone(),
        }
    }
}

/// Partial update of a task's editable fields. `None` = leave unchanged.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskPatch {
    pub name: Option<String>,
    pub directory: Option<PathBuf>,
    pub queue_id: Option<QueueId>,
    pub category_id: Option<Option<CategoryId>>,
    pub schedule_id: Option<Option<ScheduleId>>,
    pub priority: Option<Priority>,
    pub tags: Option<Vec<String>>,
    pub options: Option<TaskOptions>,
    /// Replace the mirror list.
    pub mirrors: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AddTaskResult {
    pub task: Task,
    /// A duplicate was found; the task was created `Pending` waiting for [`EngineApi::resolve_duplicate`].
    pub duplicate: Option<DuplicateInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DuplicateInfo {
    /// `path`, `filename`, `size`, `checksum`, `url_history`.
    pub matched_by: String,
    pub existing_path: Option<PathBuf>,
    pub existing_task_id: Option<TaskId>,
    pub existing_size: Option<u64>,
    pub existing_checksum: Option<Checksum>,
    pub existing_completed_at: Option<Millis>,
}

/// Probe result shown in the Add dialog before the task is created.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProbeResult {
    pub kind: TaskKind,
    pub metadata: ResolvedMetadata,
    pub suggested_name: String,
    pub suggested_directory: PathBuf,
    pub suggested_queue: QueueId,
    pub suggested_category: Option<CategoryId>,
    pub free_space: Option<u64>,
    pub duplicate: Option<DuplicateInfo>,
    /// Rules that would apply (names), in order.
    pub applicable_rules: Vec<String>,
    pub warnings: Vec<String>,
}

/// Full diagnostics for a task, also rendered as text by [`EngineApi::diagnostics_text`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskDiagnostics {
    pub task: Option<Task>,
    pub log: Vec<TaskLogEntry>,
    pub connections: Vec<ConnectionInfo>,
    pub environment: EnvironmentSnapshot,
    pub app_version: String,
    pub os: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub segment_index: u32,
    pub source_index: u32,
    pub range_start: u64,
    pub range_end: u64,
    pub committed: u64,
    pub speed: u64,
    pub state: String,
    pub remote_addr: Option<String>,
    pub http_version: Option<String>,
    pub retries: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DiskInfo {
    pub path: PathBuf,
    pub free: Option<u64>,
    pub total: Option<u64>,
    pub reserved: u64,
    pub required_by_active: u64,
    pub volume_available: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SpeedSample {
    pub at: Millis,
    pub download: u64,
    pub upload: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Dashboard {
    pub stats: GlobalStats,
    pub queues: Vec<QueueSummary>,
    pub recent: Vec<TaskRow>,
    pub speed_history: Vec<SpeedSample>,
    pub disks: Vec<DiskInfo>,
    pub scheduled_next: Vec<(ScheduleId, Millis)>,
}

// ---------------------------------------------------------------------------------------------
// Torrent / media / grabber / archive types
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileSelection {
    pub index: u32,
    pub selected: bool,
    /// 0 low, 1 normal, 2 high.
    pub priority: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GrabberOptions {
    pub url: String,
    pub max_depth: u8,
    /// `same_domain`, `subdomains`, `external`.
    pub scope: String,
    pub respect_robots: bool,
    pub concurrency: u8,
    pub max_pages: u32,
    pub include_extensions: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub include_regex: Option<String>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    /// Probe HEAD for sizes/MIME of discovered files.
    pub probe_files: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GrabberFile {
    pub url: String,
    pub name: String,
    pub extension: String,
    pub domain: String,
    pub found_on: String,
    pub size: Option<u64>,
    pub mime: Option<String>,
    pub kind: String,
    pub depth: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GrabberSession {
    pub id: String,
    pub options: GrabberOptions,
    pub pages_crawled: u32,
    pub pages_queued: u32,
    pub files: Vec<GrabberFile>,
    pub done: bool,
    pub error: Option<String>,
    pub started_at: Millis,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArchiveEntry {
    pub path: String,
    pub size: u64,
    pub compressed_size: Option<u64>,
    pub is_dir: bool,
    pub modified: Option<Millis>,
    pub crc32: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ArchiveListing {
    pub format: String,
    pub entries: Vec<ArchiveEntry>,
    pub truncated: bool,
    /// Whether the archive appears intact (central directory found, CRCs plausible).
    pub intact: Option<bool>,
    pub supports_selective_extraction: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PairingInfo {
    pub code: String,
    pub expires_at: Millis,
    /// URL the phone opens, embedding host:port and the TLS fingerprint (not the code).
    pub url: String,
    pub tls_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub available: bool,
    pub latest_version: Option<String>,
    pub notes: Option<String>,
    pub download_url: Option<String>,
    pub signature_valid: Option<bool>,
    pub checked_at: Millis,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportBundle {
    pub schema_version: u32,
    pub exported_at: Millis,
    pub settings: Option<Settings>,
    pub queues: Vec<Queue>,
    pub categories: Vec<Category>,
    pub rules: Vec<Rule>,
    pub schedules: Vec<Schedule>,
    pub automations: Vec<AutomationRule>,
    pub tasks: Vec<Task>,
    pub history: Vec<HistoryEntry>,
    pub recipes: Vec<Recipe>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportOptions {
    pub settings: bool,
    pub queues: bool,
    pub categories: bool,
    pub rules: bool,
    pub schedules: bool,
    pub automations: bool,
    pub tasks: bool,
    pub history: bool,
    pub recipes: bool,
    /// Replace existing entries with the same id.
    pub overwrite: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportReport {
    pub imported: std::collections::BTreeMap<String, u32>,
    pub skipped: std::collections::BTreeMap<String, u32>,
    pub errors: Vec<String>,
}

/// A reusable download workflow ("PDF → Documents → date folder → Finder tag").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    pub id: RecipeId,
    pub name: String,
    pub icon: String,
    pub queue_id: Option<QueueId>,
    pub category_id: Option<CategoryId>,
    pub directory: Option<PathBuf>,
    pub options: TaskOptions,
    pub tags: Vec<String>,
    pub rule_actions: Vec<osprey_domain::rules::RuleAction>,
    pub automation_id: Option<AutomationId>,
    pub created_at: Millis,
    pub updated_at: Millis,
}

/// A plugin as seen by the UI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginInfo {
    pub id: PluginId,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub kind: String,
    pub permissions: Vec<String>,
    pub enabled: bool,
    pub path: PathBuf,
    pub granted_permissions: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EngineInfo {
    pub version: String,
    pub build: String,
    pub os: String,
    pub arch: String,
    pub data_dir: PathBuf,
    pub local_api_port: u16,
    pub remote_enabled: bool,
    pub remote_port: Option<u16>,
    pub uptime_seconds: u64,
    pub headless: bool,
    pub ffmpeg_available: bool,
}

// ---------------------------------------------------------------------------------------------
// The API
// ---------------------------------------------------------------------------------------------

#[async_trait]
pub trait EngineApi: Send + Sync + 'static {
    // ----- lifecycle & events -----
    fn info(&self) -> EngineInfo;
    fn subscribe(&self) -> EventSubscription;
    /// Platform layer pushes environment measurements (battery, network, VPN…).
    async fn update_environment(&self, env: EnvironmentSnapshot);
    fn environment(&self) -> EnvironmentSnapshot;
    /// Graceful shutdown: pause transfers, flush checkpoints, close the store.
    async fn shutdown(&self);

    // ----- tasks -----
    async fn probe(&self, request: NewTaskRequest) -> DomainResult<ProbeResult>;
    async fn add_task(&self, request: NewTaskRequest) -> DomainResult<AddTaskResult>;
    async fn add_tasks(&self, requests: Vec<NewTaskRequest>) -> DomainResult<Vec<AddTaskResult>>;
    async fn resolve_duplicate(&self, id: TaskId, policy: ConflictPolicy) -> DomainResult<Task>;
    async fn get_task(&self, id: TaskId) -> DomainResult<Task>;
    async fn list_tasks(&self, filter: TaskFilter) -> DomainResult<TaskPage>;
    async fn list_rows(&self, filter: TaskFilter) -> DomainResult<Vec<TaskRow>>;
    async fn count_tasks(&self, filter: TaskFilter) -> DomainResult<u32>;
    async fn start_task(&self, id: TaskId) -> DomainResult<Task>;
    async fn pause_task(&self, id: TaskId) -> DomainResult<Task>;
    async fn resume_task(&self, id: TaskId) -> DomainResult<Task>;
    /// Restart from scratch, discarding partial data.
    async fn restart_task(&self, id: TaskId) -> DomainResult<Task>;
    /// Retry a failed task keeping partial data.
    async fn retry_task(&self, id: TaskId) -> DomainResult<Task>;
    /// Re-resolve the source (new HEAD/metadata) then continue; useful for expired URLs.
    async fn retry_from_source(&self, id: TaskId, new_url: Option<String>) -> DomainResult<Task>;
    /// Download again into a new task, keeping the completed one.
    async fn redownload(&self, id: TaskId) -> DomainResult<Task>;
    async fn cancel_task(&self, id: TaskId) -> DomainResult<Task>;
    async fn remove_task(&self, id: TaskId, delete_file: bool) -> DomainResult<()>;
    async fn remove_tasks(&self, ids: Vec<TaskId>, delete_file: bool) -> DomainResult<u32>;
    async fn update_task(&self, id: TaskId, patch: TaskPatch) -> DomainResult<Task>;
    async fn duplicate_task(&self, id: TaskId) -> DomainResult<Task>;
    async fn set_task_limit(
        &self,
        id: TaskId,
        download: Option<u64>,
        upload: Option<u64>,
    ) -> DomainResult<Task>;
    async fn set_task_connections(&self, id: TaskId, connections: u8) -> DomainResult<Task>;
    async fn set_task_priority(&self, id: TaskId, priority: Priority) -> DomainResult<Task>;
    /// Move tasks to `after` (None = top) within their queue; persists manual order.
    async fn reorder_tasks(&self, ids: Vec<TaskId>, after: Option<TaskId>) -> DomainResult<()>;
    async fn retry_failed_segments(&self, id: TaskId) -> DomainResult<Task>;
    async fn verify_task(&self, id: TaskId, checksum: Option<Checksum>) -> DomainResult<Task>;
    async fn task_log(&self, id: TaskId, limit: u32) -> DomainResult<Vec<TaskLogEntry>>;
    async fn diagnostics(&self, id: TaskId) -> DomainResult<TaskDiagnostics>;
    async fn diagnostics_text(&self, id: TaskId) -> DomainResult<String>;
    async fn pause_all(&self) -> DomainResult<u32>;
    async fn resume_all(&self) -> DomainResult<u32>;
    async fn retry_all_failed(&self) -> DomainResult<u32>;
    async fn clear_completed(&self) -> DomainResult<u32>;

    // ----- torrents -----
    async fn set_torrent_files(
        &self,
        id: TaskId,
        selection: Vec<FileSelection>,
    ) -> DomainResult<Task>;
    async fn set_torrent_sequential(&self, id: TaskId, sequential: bool) -> DomainResult<Task>;
    async fn set_seeding_limits(
        &self,
        id: TaskId,
        limits: osprey_domain::torrent::SeedingLimits,
    ) -> DomainResult<Task>;
    async fn torrent_peers(&self, id: TaskId) -> DomainResult<Vec<PeerInfo>>;
    async fn add_trackers(&self, id: TaskId, trackers: Vec<String>) -> DomainResult<Task>;
    async fn remove_tracker(&self, id: TaskId, tracker: String) -> DomainResult<Task>;
    async fn set_tracker_enabled(
        &self,
        id: TaskId,
        tracker: String,
        enabled: bool,
    ) -> DomainResult<Task>;
    async fn reannounce(&self, id: TaskId) -> DomainResult<()>;
    /// Fetch the curated tracker list from settings and apply it to public torrents.
    async fn refresh_tracker_list(&self) -> DomainResult<u32>;

    // ----- media -----
    async fn detect_media(
        &self,
        url: String,
        page_url: Option<String>,
    ) -> DomainResult<DetectedMedia>;

    // ----- queues -----
    async fn list_queues(&self) -> DomainResult<Vec<Queue>>;
    async fn queue_summaries(&self) -> DomainResult<Vec<QueueSummary>>;
    async fn create_queue(&self, queue: Queue) -> DomainResult<Queue>;
    async fn update_queue(&self, queue: Queue) -> DomainResult<Queue>;
    async fn delete_queue(&self, id: QueueId, move_tasks_to: Option<QueueId>) -> DomainResult<()>;
    async fn pause_queue(&self, id: QueueId) -> DomainResult<Queue>;
    async fn resume_queue(&self, id: QueueId) -> DomainResult<Queue>;
    async fn reorder_queues(&self, ids: Vec<QueueId>) -> DomainResult<()>;

    // ----- categories -----
    async fn list_categories(&self) -> DomainResult<Vec<Category>>;
    async fn create_category(&self, c: Category) -> DomainResult<Category>;
    async fn update_category(&self, c: Category) -> DomainResult<Category>;
    async fn delete_category(&self, id: CategoryId) -> DomainResult<()>;

    // ----- rules -----
    async fn list_rules(&self) -> DomainResult<Vec<Rule>>;
    async fn create_rule(&self, r: Rule) -> DomainResult<Rule>;
    async fn update_rule(&self, r: Rule) -> DomainResult<Rule>;
    async fn delete_rule(&self, id: RuleId) -> DomainResult<()>;
    /// Dry-run: which rules match and what they would do, in order.
    async fn test_rules(
        &self,
        subject: RuleSubject,
    ) -> DomainResult<Vec<(Rule, Vec<osprey_domain::rules::RuleAction>)>>;

    // ----- schedules -----
    async fn list_schedules(&self) -> DomainResult<Vec<Schedule>>;
    async fn create_schedule(&self, s: Schedule) -> DomainResult<Schedule>;
    async fn update_schedule(&self, s: Schedule) -> DomainResult<Schedule>;
    async fn delete_schedule(&self, id: ScheduleId) -> DomainResult<()>;

    // ----- automation -----
    async fn list_automations(&self) -> DomainResult<Vec<AutomationRule>>;
    async fn create_automation(&self, a: AutomationRule) -> DomainResult<AutomationRule>;
    async fn update_automation(&self, a: AutomationRule) -> DomainResult<AutomationRule>;
    async fn delete_automation(&self, id: AutomationId) -> DomainResult<()>;
    /// Record consent for every code-executing action in the rule (computes consent hashes).
    async fn grant_automation_consent(&self, id: AutomationId) -> DomainResult<AutomationRule>;
    async fn automation_runs(
        &self,
        id: Option<AutomationId>,
        limit: u32,
    ) -> DomainResult<Vec<AutomationRun>>;
    /// Run an automation now against a task (for testing rules).
    async fn run_automation(
        &self,
        id: AutomationId,
        task_id: TaskId,
    ) -> DomainResult<AutomationRun>;

    // ----- recipes -----
    async fn list_recipes(&self) -> DomainResult<Vec<Recipe>>;
    async fn save_recipe(&self, r: Recipe) -> DomainResult<Recipe>;
    async fn delete_recipe(&self, id: RecipeId) -> DomainResult<()>;
    async fn apply_recipe(
        &self,
        id: RecipeId,
        request: NewTaskRequest,
    ) -> DomainResult<AddTaskResult>;

    // ----- history -----
    async fn history(&self, query: HistoryQuery) -> DomainResult<Vec<HistoryEntry>>;
    async fn history_count(&self, query: HistoryQuery) -> DomainResult<u32>;
    async fn delete_history(&self, ids: Vec<TaskId>) -> DomainResult<u32>;
    async fn clear_history(&self) -> DomainResult<u32>;

    // ----- bandwidth -----
    async fn set_traffic_mode(&self, mode: TrafficMode) -> DomainResult<()>;
    async fn set_global_limits(&self, download: u64, upload: u64) -> DomainResult<()>;
    /// "Optimize": measure and apply connection/segment settings from the observed network.
    async fn optimize(&self) -> DomainResult<Settings>;

    // ----- settings -----
    fn settings(&self) -> Arc<Settings>;
    async fn update_settings(&self, settings: Settings) -> DomainResult<Settings>;
    /// Store a secret in the OS keychain and return its reference.
    async fn store_credential(
        &self,
        name: String,
        username: Option<String>,
        secret: String,
    ) -> DomainResult<CredentialId>;
    async fn list_credentials(&self) -> DomainResult<Vec<(CredentialId, String, Option<String>)>>;
    async fn delete_credential(&self, id: CredentialId) -> DomainResult<()>;

    // ----- stats / dashboard / disk -----
    fn global_stats(&self) -> GlobalStats;
    async fn dashboard(&self) -> DomainResult<Dashboard>;
    async fn disk_info(&self, path: Option<PathBuf>) -> DomainResult<DiskInfo>;

    // ----- remote devices -----
    async fn list_devices(&self) -> DomainResult<Vec<Device>>;
    async fn start_pairing(&self, scopes: Vec<Scope>) -> DomainResult<PairingInfo>;
    async fn cancel_pairing(&self) -> DomainResult<()>;
    /// Called by the server when a client presents a code; returns the device + bearer token.
    async fn complete_pairing(
        &self,
        code: String,
        device_name: String,
        device_kind: String,
        ip: String,
    ) -> DomainResult<(Device, String)>;
    async fn revoke_device(&self, id: DeviceId) -> DomainResult<()>;
    async fn rename_device(&self, id: DeviceId, name: String) -> DomainResult<Device>;
    /// Validate a bearer token; returns the device (updates last_seen) or NotFound.
    async fn authenticate(&self, token: &str, ip: &str) -> DomainResult<Device>;
    async fn audit_log(&self, limit: u32) -> DomainResult<Vec<AuditEntry>>;
    async fn record_audit(&self, entry: AuditEntry) -> DomainResult<()>;
    /// The local-API token used by the CLI and the native messaging host.
    fn local_token(&self) -> String;

    // ----- site grabber -----
    async fn grabber_start(&self, options: GrabberOptions) -> DomainResult<GrabberSession>;
    async fn grabber_status(&self, id: String) -> DomainResult<GrabberSession>;
    async fn grabber_cancel(&self, id: String) -> DomainResult<()>;
    async fn grabber_add(
        &self,
        id: String,
        urls: Vec<String>,
        request: NewTaskRequest,
    ) -> DomainResult<Vec<AddTaskResult>>;
    async fn grabber_list(&self) -> DomainResult<Vec<GrabberSession>>;

    // ----- archives -----
    async fn archive_list(&self, path: PathBuf) -> DomainResult<ArchiveListing>;
    async fn archive_extract(
        &self,
        path: PathBuf,
        entries: Option<Vec<String>>,
        destination: PathBuf,
    ) -> DomainResult<u32>;

    // ----- import / export -----
    async fn export(
        &self,
        include_tasks: bool,
        include_history: bool,
    ) -> DomainResult<ExportBundle>;
    async fn import(
        &self,
        bundle: ExportBundle,
        options: ImportOptions,
    ) -> DomainResult<ImportReport>;

    // ----- updates -----
    async fn check_for_updates(&self) -> DomainResult<UpdateInfo>;
    /// Download the update with the engine and verify its Ed25519 signature; returns the path.
    async fn download_update(&self) -> DomainResult<PathBuf>;

    // ----- plugins -----
    async fn list_plugins(&self) -> DomainResult<Vec<PluginInfo>>;
    async fn set_plugin_enabled(
        &self,
        id: PluginId,
        enabled: bool,
        granted_permissions: Vec<String>,
    ) -> DomainResult<PluginInfo>;
    async fn uninstall_plugin(&self, id: PluginId) -> DomainResult<()>;

    // ----- logs -----
    async fn recent_logs(&self, limit: u32, level: Option<String>) -> DomainResult<Vec<String>>;
}

pub type SharedEngine = Arc<dyn EngineApi>;
