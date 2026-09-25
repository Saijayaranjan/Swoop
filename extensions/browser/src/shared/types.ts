/**
 * TypeScript mirrors of the Rust wire types the extension talks to, taken from:
 *   - crates/swoop-domain/src/task.rs      (NewTaskRequest, TaskOptions, Progress, Checksum)
 *   - crates/swoop-domain/src/media.rs     (DetectedMedia, MediaVariant, MediaInfo)
 *   - crates/swoop-domain/src/settings.rs  (BrowserSettings)
 *   - crates/swoop-domain/src/events.rs    (Event, GlobalStats, Notification, TaskLogEntry)
 *   - crates/swoop-domain/src/error.rs     (ErrorKind)
 *   - crates/swoop-services/src/api.rs     (TaskRow, AddTaskResult, ProbeResult, EngineInfo)
 *
 * All Swoop ids (`TaskId`, `QueueId`, `CategoryId`, ...) are `#[serde(transparent)]` newtypes
 * over `String`, so they are plain strings on the wire. `Millis` is `#[serde(transparent)]` over
 * `i64`, so it is a plain number (Unix epoch milliseconds) on the wire.
 *
 * Everything here is a pure type/const module: no browser globals, safe to unit test directly.
 */

// A string-literal union that still accepts unknown values, so the extension does not break when
// the desktop app adds a new variant of an open-ended enum (error kinds, notification kinds it
// does not yet render, etc). Known members keep autocomplete/exhaustiveness where it matters.
type Extensible<T extends string> = T | (string & Record<never, never>);

export type Millis = number;

export type TaskKind = 'http' | 'ftp' | 'torrent' | 'magnet' | 'metalink' | 'hls';

export type TaskState =
  | 'pending'
  | 'queued'
  | 'scheduled'
  | 'resolving'
  | 'connecting'
  | 'downloading'
  | 'paused'
  | 'retrying'
  | 'verifying'
  | 'processing'
  | 'completed'
  | 'failed'
  | 'cancelled'
  | 'seeding';

export const ACTIVE_TASK_STATES: ReadonlySet<TaskState> = new Set([
  'queued',
  'resolving',
  'connecting',
  'downloading',
  'retrying',
  'verifying',
  'processing',
]);

export type Priority = 'low' | 'normal' | 'high' | 'urgent';

export type ConflictPolicy = 'ask' | 'replace' | 'rename' | 'skip' | 'keep_both';

export type ChecksumAlgorithm = 'md5' | 'sha1' | 'sha256' | 'sha512' | 'blake3';

export interface Checksum {
  algorithm: ChecksumAlgorithm;
  value: string;
}

export type ErrorKind = Extensible<
  | 'invalid_url'
  | 'unsupported_scheme'
  | 'dns_failure'
  | 'tls_failure'
  | 'certificate_invalid'
  | 'connection_refused'
  | 'connection_timeout'
  | 'connection_reset'
  | 'read_timeout'
  | 'proxy_error'
  | 'authentication_required'
  | 'forbidden'
  | 'not_found'
  | 'throttled'
  | 'server_error'
  | 'range_not_supported'
  | 'source_changed'
  | 'expired_url'
  | 'redirect_loop'
  | 'truncated'
  | 'checksum_mismatch'
  | 'disk_full'
  | 'disk_write_error'
  | 'disk_read_error'
  | 'permission_denied'
  | 'volume_unavailable'
  | 'invalid_filename'
  | 'path_traversal'
  | 'file_exists'
  | 'invalid_torrent'
  | 'tracker_failure'
  | 'no_peers'
  | 'dht_unavailable'
  | 'parse_error'
  | 'protected_content'
  | 'mirror_exhausted'
  | 'unknown'
>;

/** Per-task transfer options. `undefined`/absent means "inherit from queue/global settings". */
export interface TaskOptions {
  max_connections?: number | null;
  download_limit?: number | null;
  upload_limit?: number | null;
  headers?: Record<string, string>;
  user_agent?: string | null;
  referer?: string | null;
  cookies?: string | null;
  credential?: string | null;
  proxy?: string | null;
  direct_connection?: boolean;
  checksum?: Checksum | null;
  conflict_policy?: ConflictPolicy;
  preallocate?: boolean | null;
  sparse?: boolean | null;
  adaptive?: boolean | null;
  max_retries?: number | null;
  sequential?: boolean | null;
  seed_ratio_limit?: number | null;
  seed_time_limit_minutes?: number | null;
  max_peers?: number | null;
  media_variant?: string | null;
  allow_http2?: boolean | null;
  open_when_done?: boolean;
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

/** Input for creating a task through any front door, incl. the extension. */
export interface NewTaskRequest {
  url?: string | null;
  mirrors?: string[];
  magnet?: string | null;
  torrent_base64?: string | null;
  metalink_url?: string | null;
  hls_playlist_url?: string | null;
  name?: string | null;
  directory?: string | null;
  queue_id?: string | null;
  category_id?: string | null;
  schedule_id?: string | null;
  priority?: Priority | null;
  tags?: string[];
  options?: TaskOptions;
  /** Start immediately (`true`) or leave `Pending` for the user to confirm ("Queue"). */
  start: boolean;
  origin: string;
  selected_files?: number[] | null;
  /** Page the link was found on; used by rules and history. */
  referer_page?: string | null;
}

export interface TaskRow {
  id: string;
  name: string;
  kind: TaskKind;
  state: TaskState;
  domain: string | null;
  progress: Progress;
  queue_id: string;
  category_id: string | null;
  priority: Priority;
  position: number;
  created_at: Millis;
  error_kind: ErrorKind | null;
  health: number;
  file_path: string | null;
}

export interface DuplicateInfo {
  matched_by: string;
  existing_path: string | null;
  existing_task_id: string | null;
  existing_size: number | null;
  existing_checksum: Checksum | null;
  existing_completed_at: Millis | null;
}

/** Minimal shape of `Task` the extension actually reads off `AddTaskResult`. */
export interface TaskLike {
  id: string;
  name: string;
  kind: TaskKind;
  state: TaskState;
  progress: Progress;
  error?: { kind: ErrorKind; message?: string } | null;
  [key: string]: unknown;
}

export interface AddTaskResult {
  task: TaskLike;
  duplicate: DuplicateInfo | null;
}

export interface ResolvedMetadata {
  name?: string | null;
  total?: number | null;
  mime?: string | null;
  resumable?: boolean | null;
  final_url?: string | null;
  etag?: string | null;
  last_modified?: string | null;
  server?: string | null;
  content_disposition?: string | null;
  http_version?: string | null;
  remote_addr?: string | null;
  file_path?: string | null;
}

export interface ProbeResult {
  kind: TaskKind;
  metadata: ResolvedMetadata;
  suggested_name: string;
  suggested_directory: string;
  suggested_queue: string;
  suggested_category: string | null;
  free_space: number | null;
  duplicate: DuplicateInfo | null;
  applicable_rules: string[];
  warnings: string[];
}

export type MediaKind =
  | 'video'
  | 'audio'
  | 'image'
  | 'hls_playlist'
  | 'dash_manifest'
  | 'unknown';

export interface MediaVariant {
  id: string;
  label: string;
  url: string;
  width?: number | null;
  height?: number | null;
  bandwidth?: number | null;
  codecs?: string | null;
  frame_rate?: number | null;
  estimated_size?: number | null;
  audio_only?: boolean;
  container?: string | null;
}

export interface DetectedMedia {
  url: string;
  kind: MediaKind;
  title?: string | null;
  mime?: string | null;
  size?: number | null;
  page_url?: string | null;
  variants?: MediaVariant[];
  protected?: boolean;
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

export type TrafficMode = Extensible<'normal' | 'browsing' | 'gaming' | 'night' | 'custom'>;

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

export type NotificationPayload =
  | { kind: 'completed'; task_id: string; name: string; path: string }
  | { kind: 'failed'; task_id: string; name: string; reason: string }
  | { kind: 'queued'; task_id: string; name: string }
  | { kind: 'scheduled'; task_id: string; name: string; at: Millis }
  | { kind: 'checksum_mismatch'; task_id: string; name: string }
  | { kind: 'low_disk_space'; path: string; free: number; required: number }
  | { kind: 'torrent_finished'; task_id: string; name: string }
  | { kind: 'device_paired'; device_id: string; name: string }
  | { kind: 'automation_failed'; automation_id: string; name: string; error: string }
  | { kind: 'duplicate_detected'; task_id: string; name: string; existing_path: string }
  | { kind: 'update_available'; version: string; notes_url: string | null }
  | { kind: 'queue_finished'; queue_id: string; name: string };

export interface TaskStateChangedData {
  task_id: string;
  from: TaskState;
  to: TaskState;
  at: Millis;
}

export interface ProgressUpdate {
  task_id: string;
  progress: Progress;
  rev: number;
}

export interface TaskRemovedData {
  task_id: string;
  deleted_file: boolean;
}

/**
 * The subset of `Event::type_name()` variants (docs/api/websocket.md, events.rs) the extension
 * subscribes to and acts on. `data` is typed per `type` below; unrecognised events still arrive
 * (see `SwoopEvent` catch-all) so a forward-compatible switch never silently drops one.
 */
export type SwoopEventType =
  | 'task_added'
  | 'task_updated'
  | 'task_removed'
  | 'task_state_changed'
  | 'progress'
  | 'global_stats'
  | 'notification'
  | 'settings_changed';

export type SwoopEvent =
  | { type: 'task_added'; data: TaskLike }
  | { type: 'task_updated'; data: TaskLike }
  | { type: 'task_removed'; data: TaskRemovedData }
  | { type: 'task_state_changed'; data: TaskStateChangedData }
  | { type: 'progress'; data: ProgressUpdate[] }
  | { type: 'global_stats'; data: GlobalStats }
  | { type: 'notification'; data: NotificationPayload }
  | { type: 'settings_changed'; data: unknown }
  | { type: Extensible<SwoopEventType>; data: unknown };

/** `BrowserSettings` (crates/swoop-domain/src/settings.rs) — mirrored 1:1 in the options UI. */
export interface BrowserSettings {
  intercept_downloads: boolean;
  /** Minimum size in bytes for automatic interception; smaller files stay in the browser. */
  intercept_min_size: number;
  intercept_extensions: string[];
  excluded_domains: string[];
  excluded_url_patterns: string[];
  detect_media: boolean;
  show_confirmation: boolean;
}

export function defaultBrowserSettings(): BrowserSettings {
  return {
    intercept_downloads: true,
    intercept_min_size: 1024 * 1024,
    intercept_extensions: [
      'zip', 'rar', '7z', 'tar', 'gz', 'iso', 'dmg', 'pkg', 'exe', 'msi', 'mp4', 'mkv',
      'mp3', 'flac', 'pdf', 'epub', 'apk', 'deb', 'rpm', 'torrent',
    ],
    excluded_domains: [],
    excluded_url_patterns: [],
    detect_media: true,
    show_confirmation: true,
  };
}

/** How the extension reaches Swoop: the local native-messaging host (default) or a paired
 *  remote Swoop over HTTPS with a device token. */
export type ConnectionMode = 'native' | 'remote';

/** Extension-only settings, layered on top of the mirrored `BrowserSettings`. */
export interface ExtensionSettings extends BrowserSettings {
  /** Per-site "always use browser" (never intercept) — a superset of `excluded_domains`
   *  the user can toggle from the popup/toast, kept separate so it round-trips independently. */
  always_browser_domains: string[];
  notify_on_complete: boolean;
  notify_on_failure: boolean;
  /** Show the small in-page download button over playing video/audio (needs `detect_media`). */
  media_button: boolean;
  connection_mode: ConnectionMode;
  /** Base URL of a paired remote Swoop (`https://host:41780`). The device token is stored
   *  separately (src/shared/remote.ts) so it never travels with the settings object. */
  remote_url: string;
}

export function defaultExtensionSettings(): ExtensionSettings {
  return {
    ...defaultBrowserSettings(),
    always_browser_domains: [],
    notify_on_complete: true,
    notify_on_failure: true,
    media_button: true,
    connection_mode: 'native',
    remote_url: '',
  };
}
