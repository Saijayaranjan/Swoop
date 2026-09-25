// TypeScript mirrors of the Rust DTOs in crates/swoop-domain and crates/swoop-services.
// Ids and Millis are transparent newtypes serialized as string / number respectively.

export type TaskId = string;
export type QueueId = string;
export type CategoryId = string;
export type ScheduleId = string;
export type DeviceId = string;
export type CredentialId = string;
export type AutomationId = string;
export type Millis = number;

export type TaskKind = "http" | "ftp" | "torrent" | "magnet" | "metalink" | "hls";

export type TaskState =
  | "pending"
  | "queued"
  | "scheduled"
  | "resolving"
  | "connecting"
  | "downloading"
  | "paused"
  | "retrying"
  | "verifying"
  | "processing"
  | "completed"
  | "failed"
  | "cancelled"
  | "seeding";

export type Priority = "low" | "normal" | "high" | "urgent";

export type ConflictPolicy = "ask" | "replace" | "rename" | "skip" | "keep_both";

export type ChecksumAlgorithm = "md5" | "sha1" | "sha256" | "sha512" | "blake3";

export interface Checksum {
  algorithm: ChecksumAlgorithm;
  value: string;
}

export type ErrorKind =
  | "invalid_url"
  | "unsupported_scheme"
  | "dns_failure"
  | "tls_failure"
  | "certificate_invalid"
  | "connection_refused"
  | "connection_timeout"
  | "connection_reset"
  | "read_timeout"
  | "proxy_error"
  | "authentication_required"
  | "forbidden"
  | "not_found"
  | "throttled"
  | "server_error"
  | "range_not_supported"
  | "source_changed"
  | "expired_url"
  | "redirect_loop"
  | "truncated"
  | "checksum_mismatch"
  | "disk_full"
  | "disk_write_error"
  | "disk_read_error"
  | "permission_denied"
  | "volume_unavailable"
  | "invalid_filename"
  | "path_traversal"
  | "file_exists"
  | "invalid_torrent"
  | "tracker_failure"
  | "no_peers"
  | "dht_unavailable"
  | "parse_error"
  | "protected_content"
  | "mirror_exhausted"
  | "network_unavailable"
  | "unexpected_content"
  | "live_stream_unsupported"
  | "quota_exceeded"
  | "cancelled"
  | "internal"
  | "unknown";

/** `ErrorKind::explanation_key` — i18n key for the human explanation of an error kind. */
export function errorExplanationKey(kind: ErrorKind): string {
  return `error.${kind}`;
}

export interface TaskError {
  kind: ErrorKind;
  message: string;
  status_code?: number;
  detail?: string;
  source_url?: string;
  retry_after_ms?: number;
  at: Millis;
}

export interface Progress {
  downloaded: number;
  uploaded: number;
  total: number | null;
  speed: number;
  instant_speed: number;
  upload_speed: number;
  eta_seconds: number | null;
  active_connections: number;
  peers: number;
  seeds: number;
  ratio: number;
  fraction: number;
}

export interface TaskRow {
  id: TaskId;
  name: string;
  kind: TaskKind;
  state: TaskState;
  domain: string | null;
  progress: Progress;
  queue_id: QueueId;
  category_id: CategoryId | null;
  priority: Priority;
  position: number;
  created_at: Millis;
  error_kind: ErrorKind | null;
  health: number;
  file_path: string | null;
}

export type PauseReason =
  | "user"
  | "queue"
  | "schedule"
  | "disk_space"
  | "battery"
  | "network"
  | "condition"
  | string;

export interface TaskStats {
  average_speed: number;
  peak_speed: number;
  retries: number;
  failed_connections: number;
  segments_reassigned: number;
  mirrors_switched: number;
  active_seconds: number;
  bytes_discarded: number;
  throughput_drops: number;
  range_supported: boolean | null;
  http_version: string | null;
  final_url: string | null;
  etag: string | null;
  last_modified: string | null;
  server: string | null;
  content_type: string | null;
  content_disposition: string | null;
  remote_addr: string | null;
}

export interface TorrentFile {
  index: number;
  path: string;
  size: number;
  downloaded: number;
  selected: boolean;
  priority: number;
}

export interface TrackerStatus {
  url: string;
  tier: number;
  enabled: boolean;
  last_announce_at?: Millis;
  next_announce_at?: Millis;
  seeders?: number;
  leechers?: number;
  latency_ms?: number;
  last_error?: string;
  health: string;
  consecutive_failures: number;
}

export interface PeerInfo {
  address: string;
  client: string | null;
  download_speed: number;
  upload_speed: number;
  progress: number;
  flags: string;
  downloaded: number;
  uploaded: number;
}

export interface TorrentInfo {
  info_hash: string;
  name: string;
  total_size: number;
  piece_length: number;
  piece_count: number;
  files: TorrentFile[];
  trackers: TrackerStatus[];
  private: boolean;
  comment: string | null;
  created_by: string | null;
  magnet: string | null;
  have_metadata: boolean;
  seeders_total: number;
  leechers_total: number;
  connected_peers: number;
  connected_seeds: number;
  dht_nodes: number;
  uploaded: number;
  ratio: number;
  availability: number;
  piece_map_rle: number[];
  seeding_since: Millis | null;
}

export interface SeedingLimits {
  ratio_limit?: number | null;
  time_limit_minutes?: number | null;
  upload_limit?: number | null;
  seed_when_complete?: boolean | null;
}

export interface TaskOptions {
  max_connections: number | null;
  download_limit: number | null;
  upload_limit: number | null;
  headers: Record<string, string>;
  user_agent: string | null;
  referer: string | null;
  cookies: string | null;
  credential: CredentialId | null;
  proxy: string | null;
  direct_connection: boolean;
  checksum: Checksum | null;
  conflict_policy: ConflictPolicy;
  preallocate: boolean | null;
  sparse: boolean | null;
  adaptive: boolean | null;
  max_retries: number | null;
  sequential: boolean | null;
  seed_ratio_limit: number | null;
  seed_time_limit_minutes: number | null;
  max_peers: number | null;
  media_variant: string | null;
  allow_http2: boolean | null;
  open_when_done: boolean;
}

export type SourceType = "urls" | "torrent_file" | "magnet" | "metalink" | "hls";

export interface Source {
  type: SourceType;
  urls?: string[];
  info_hash?: string;
  name?: string;
  uri?: string;
  url?: string | null;
  document?: string | null;
  playlist_url?: string;
  variant?: string | null;
}

export interface Task {
  id: TaskId;
  kind: TaskKind;
  source: Source;
  name: string;
  directory: string;
  file_path: string | null;
  state: TaskState;
  blocked_by: PauseReason[];
  rev: number;
  name_locked: boolean;
  directory_locked: boolean;
  status_detail: string | null;
  error: TaskError | null;
  progress: Progress;
  stats: TaskStats;
  queue_id: QueueId;
  category_id: CategoryId | null;
  schedule_id: ScheduleId | null;
  priority: Priority;
  position: number;
  tags: string[];
  options: TaskOptions;
  torrent: TorrentInfo | null;
  origin: string;
  mime: string | null;
  created_at: Millis;
  updated_at: Millis;
  started_at: Millis | null;
  completed_at: Millis | null;
  verified_checksum: Checksum | null;
  attempt: number;
  next_retry_at: Millis | null;
}

export interface TaskPage {
  tasks: Task[];
  total: number;
}

export interface TaskPatch {
  name?: string;
  directory?: string;
  queue_id?: QueueId;
  category_id?: CategoryId | null;
  schedule_id?: ScheduleId | null;
  priority?: Priority;
  tags?: string[];
  options?: TaskOptions;
  mirrors?: string[];
}

export interface DuplicateInfo {
  matched_by: string;
  existing_path: string | null;
  existing_task_id: TaskId | null;
  existing_size: number | null;
  existing_checksum: Checksum | null;
  existing_completed_at: Millis | null;
}

export interface AddTaskResult {
  task: Task;
  duplicate: DuplicateInfo | null;
}

export interface ResolvedMetadata {
  name?: string;
  size?: number | null;
  mime?: string | null;
  [key: string]: unknown;
}

export interface ProbeResult {
  kind: TaskKind;
  metadata: ResolvedMetadata;
  suggested_name: string;
  suggested_directory: string;
  suggested_queue: QueueId;
  suggested_category: CategoryId | null;
  free_space: number | null;
  duplicate: DuplicateInfo | null;
  applicable_rules: string[];
  warnings: string[];
}

export interface TaskLogEntry {
  task_id: TaskId;
  at: Millis;
  level: "trace" | "debug" | "info" | "warn" | "error";
  code: string;
  message: string;
}

export interface ConnectionInfo {
  segment_index: number;
  source_index: number;
  range_start: number;
  range_end: number;
  committed: number;
  speed: number;
  state: string;
  remote_addr: string | null;
  http_version: string | null;
  retries: number;
}

export interface TaskDiagnostics {
  task: Task | null;
  log: TaskLogEntry[];
  connections: ConnectionInfo[];
  app_version: string;
  os: string;
}

export interface DiskInfo {
  path: string;
  free: number | null;
  total: number | null;
  reserved: number;
  required_by_active: number;
  volume_available: boolean;
}

export interface SpeedSample {
  at: Millis;
  download: number;
  upload: number;
}

export type TrafficMode = "unlimited" | "full_speed" | "balanced" | "browsing" | "custom";

export interface BandwidthProfile {
  download_limit: number;
  upload_limit: number;
  connections_per_task: number;
}

export interface Queue {
  id: QueueId;
  name: string;
  icon: string;
  color: string | null;
  max_concurrent: number;
  bandwidth: BandwidthProfile;
  schedule_id: ScheduleId | null;
  directory: string | null;
  priority: number;
  paused: boolean;
  builtin: boolean;
  position: number;
  created_at: Millis;
  updated_at: Millis;
}

export interface QueueSummary {
  queue_id: QueueId;
  active: number;
  waiting: number;
  completed: number;
  failed: number;
  download_speed: number;
  upload_speed: number;
}

export interface GlobalStats {
  download_speed: number;
  upload_speed: number;
  active: number;
  downloading: number;
  seeding: number;
  queued: number;
  scheduled: number;
  paused: number;
  completed_today: number;
  failed_today: number;
  total_tasks: number;
  bytes_today: number;
  free_space: number | null;
  network_available: boolean;
  traffic_mode: TrafficMode;
  download_limit: number;
  upload_limit: number;
  at: Millis;
}

export interface Dashboard {
  stats: GlobalStats;
  queues: QueueSummary[];
  recent: TaskRow[];
  speed_history: SpeedSample[];
  disks: DiskInfo[];
  scheduled_next: [ScheduleId, Millis][];
}

export interface FileSelection {
  index: number;
  selected: boolean;
  priority: number;
}

export type NotificationKind =
  | "completed"
  | "failed"
  | "queued"
  | "scheduled"
  | "checksum_mismatch"
  | "low_disk_space"
  | "torrent_finished"
  | "device_paired"
  | "automation_failed"
  | "duplicate_detected"
  | "update_available"
  | "queue_finished";

export interface Notification {
  kind: NotificationKind;
  task_id?: TaskId;
  name?: string;
  path?: string;
  reason?: string;
  at?: Millis;
  device_id?: DeviceId;
  automation_id?: AutomationId;
  error?: string;
  existing_path?: string;
  version?: string;
  notes_url?: string | null;
  queue_id?: QueueId;
  free?: number;
  required?: number;
}

export type Scope = "read" | "add" | "control" | "admin";

export interface Device {
  id: DeviceId;
  name: string;
  kind: string;
  scopes: Scope[];
  created_at: Millis;
  last_seen_at: Millis | null;
  last_ip: string | null;
  expires_at: Millis | null;
  revoked: boolean;
}

export interface PairingInfo {
  code: string;
  expires_at: Millis;
  url: string;
  tls_fingerprint: string | null;
}

export interface EngineInfo {
  version: string;
  build: string;
  os: string;
  arch: string;
  data_dir: string;
  local_api_port: number;
  remote_enabled: boolean;
  remote_port: number | null;
  uptime_seconds: number;
  headless: boolean;
  ffmpeg_available: boolean;
}

export interface NewTaskRequest {
  url?: string;
  mirrors: string[];
  magnet?: string;
  torrent_base64?: string;
  metalink_url?: string;
  hls_playlist_url?: string;
  name?: string;
  directory?: string;
  queue_id?: QueueId;
  category_id?: CategoryId;
  schedule_id?: ScheduleId;
  priority?: Priority;
  tags: string[];
  options: Partial<TaskOptions>;
  start: boolean;
  origin: string;
  selected_files?: number[];
  referer_page?: string;
}

/** `TaskFilter` query params for GET /tasks and /tasks/rows. */
export interface TaskFilter {
  text?: string;
  state?: TaskState[];
  kind?: TaskKind;
  queue_id?: QueueId;
  category_id?: CategoryId;
  domain?: string;
  tag?: string;
  smart?: "active" | "queued" | "scheduled" | "complete" | "failed" | "torrent" | "media" | "paused";
  sort?: "position" | "created_at" | "name" | "size" | "progress" | "speed" | "eta" | "state" | "domain";
  desc?: boolean;
  limit?: number;
  offset?: number;
}

/** The API error envelope: `{"error": {"type": ..., "message": ...}}`. */
export interface ApiErrorBody {
  error: {
    type:
      | "not_found"
      | "validation"
      | "conflict"
      | "permission_denied"
      | "storage"
      | "engine"
      | "unavailable"
      | "internal"
      | "unauthorized"
      | "rate_limited";
    message: string;
  };
}

export interface ProgressUpdate {
  task_id: TaskId;
  progress: Progress;
  rev: number;
}

export interface WsFrame {
  type: string;
  data?: unknown;
  seq?: number;
}
