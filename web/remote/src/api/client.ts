// Thin REST client for the Osprey remote API. Every call targets a same-origin relative path
// (`/api/v1/...`) -- the embedded web UI is always served by the same daemon it controls, so
// there is never a reason to talk to any other origin.

import { mapNetworkError, mapResponseError } from "./errors.ts";
import type {
  AddTaskResult,
  Dashboard,
  Device,
  DiskInfo,
  EngineInfo,
  FileSelection,
  GlobalStats,
  NewTaskRequest,
  PeerInfo,
  ProbeResult,
  Queue,
  QueueSummary,
  SeedingLimits,
  Task,
  TaskDiagnostics,
  TaskFilter,
  TaskLogEntry,
  TaskPage,
  TaskPatch,
  TaskRow,
  TrafficMode,
} from "./types.ts";

export interface ApiClientOptions {
  getToken: () => string | null;
  /** Called whenever any request comes back 401; the caller is responsible for clearing auth state. */
  onUnauthorized: () => void;
}

function buildQuery(params: Record<string, string | number | boolean | readonly string[] | undefined>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined) {
      continue;
    }
    if (Array.isArray(value)) {
      for (const item of value) {
        search.append(key, item);
      }
    } else {
      search.append(key, String(value));
    }
  }
  const qs = search.toString();
  return qs.length > 0 ? `?${qs}` : "";
}

export function taskFilterToQuery(filter: TaskFilter): string {
  return buildQuery({
    text: filter.text,
    state: filter.state,
    kind: filter.kind,
    queue_id: filter.queue_id,
    category_id: filter.category_id,
    domain: filter.domain,
    tag: filter.tag,
    smart: filter.smart,
    sort: filter.sort,
    desc: filter.desc,
    limit: filter.limit,
    offset: filter.offset,
  });
}

export class ApiClient {
  private readonly options: ApiClientOptions;

  constructor(options: ApiClientOptions) {
    this.options = options;
  }

  private async request<T>(path: string, init: RequestInit = {}): Promise<T> {
    const headers = new Headers(init.headers);
    headers.set("Accept", "application/json");
    if (init.body !== undefined && init.body !== null && !headers.has("Content-Type")) {
      headers.set("Content-Type", "application/json");
    }
    const token = this.options.getToken();
    if (token) {
      headers.set("Authorization", `Bearer ${token}`);
    }

    let response: Response;
    try {
      response = await fetch(path, { ...init, headers });
    } catch (cause) {
      throw mapNetworkError(cause);
    }

    if (response.status === 401) {
      this.options.onUnauthorized();
    }

    if (response.status === 204) {
      return undefined as T;
    }

    const text = await response.text();
    let parsed: unknown;
    let parseFailed = false;
    if (text.length > 0) {
      try {
        parsed = JSON.parse(text);
      } catch {
        parseFailed = true;
      }
    }

    if (!response.ok || parseFailed) {
      throw mapResponseError(response.status, parsed, parseFailed);
    }
    return parsed as T;
  }

  private get<T>(path: string): Promise<T> {
    return this.request<T>(path, { method: "GET" });
  }
  private post<T>(path: string, body?: unknown): Promise<T> {
    return this.request<T>(path, { method: "POST", body: body === undefined ? undefined : JSON.stringify(body) });
  }
  private put<T>(path: string, body?: unknown): Promise<T> {
    return this.request<T>(path, { method: "PUT", body: body === undefined ? undefined : JSON.stringify(body) });
  }
  private patch<T>(path: string, body?: unknown): Promise<T> {
    return this.request<T>(path, { method: "PATCH", body: body === undefined ? undefined : JSON.stringify(body) });
  }
  private del<T>(path: string, body?: unknown): Promise<T> {
    return this.request<T>(path, { method: "DELETE", body: body === undefined ? undefined : JSON.stringify(body) });
  }

  // ----- pairing (no auth token yet) -----
  pair(code: string, deviceName: string, deviceKind: string): Promise<{ device: Device; token: string }> {
    return this.post("/api/v1/pair", { code, device_name: deviceName, device_kind: deviceKind });
  }

  // ----- engine -----
  info(): Promise<EngineInfo> {
    return this.get("/api/v1/info");
  }
  stats(): Promise<GlobalStats> {
    return this.get("/api/v1/stats");
  }
  dashboard(): Promise<Dashboard> {
    return this.get("/api/v1/dashboard");
  }
  disk(path?: string): Promise<DiskInfo> {
    return this.get(`/api/v1/disk${buildQuery({ path })}`);
  }
  setTrafficMode(mode: TrafficMode): Promise<void> {
    return this.post("/api/v1/traffic-mode", { mode });
  }
  setLimits(download: number, upload: number): Promise<void> {
    return this.post("/api/v1/limits", { download, upload });
  }

  // ----- tasks -----
  probeTask(request: NewTaskRequest): Promise<ProbeResult> {
    return this.post("/api/v1/tasks/probe", request);
  }
  addTask(request: NewTaskRequest): Promise<AddTaskResult> {
    return this.post("/api/v1/tasks", request);
  }
  addTasksBatch(requests: NewTaskRequest[]): Promise<AddTaskResult[]> {
    return this.post("/api/v1/tasks/batch", requests);
  }
  listTasks(filter: TaskFilter): Promise<TaskPage> {
    return this.get(`/api/v1/tasks${taskFilterToQuery(filter)}`);
  }
  listRows(filter: TaskFilter): Promise<TaskRow[]> {
    return this.get(`/api/v1/tasks/rows${taskFilterToQuery(filter)}`);
  }
  getTask(id: string): Promise<Task> {
    return this.get(`/api/v1/tasks/${encodeURIComponent(id)}`);
  }
  patchTask(id: string, patch: TaskPatch): Promise<Task> {
    return this.patch(`/api/v1/tasks/${encodeURIComponent(id)}`, patch);
  }
  deleteTask(id: string, deleteFile: boolean): Promise<void> {
    return this.del(`/api/v1/tasks/${encodeURIComponent(id)}${buildQuery({ delete_file: deleteFile })}`);
  }
  removeTasks(ids: string[], deleteFile: boolean): Promise<{ removed: number }> {
    return this.post("/api/v1/tasks/remove", { ids, delete_file: deleteFile });
  }
  taskAction(
    id: string,
    action:
      | "start"
      | "pause"
      | "resume"
      | "restart"
      | "retry"
      | "cancel"
      | "redownload"
      | "verify"
      | "retry-segments"
      | "duplicate"
      | "reannounce",
  ): Promise<Task> {
    return this.post(`/api/v1/tasks/${encodeURIComponent(id)}/${action}`);
  }
  retryFromSource(id: string, newUrl?: string): Promise<Task> {
    return this.post(`/api/v1/tasks/${encodeURIComponent(id)}/retry-from-source`, { url: newUrl });
  }
  resolveDuplicate(id: string, policy: string): Promise<Task> {
    return this.post(`/api/v1/tasks/${encodeURIComponent(id)}/resolve-duplicate`, { policy });
  }
  setTaskPriority(id: string, priority: string): Promise<Task> {
    return this.post(`/api/v1/tasks/${encodeURIComponent(id)}/priority`, { priority });
  }
  taskLog(id: string, limit = 200): Promise<TaskLogEntry[]> {
    return this.get(`/api/v1/tasks/${encodeURIComponent(id)}/log${buildQuery({ limit })}`);
  }
  diagnostics(id: string): Promise<TaskDiagnostics> {
    return this.get(`/api/v1/tasks/${encodeURIComponent(id)}/diagnostics`);
  }
  pauseAll(): Promise<{ count: number }> {
    return this.post("/api/v1/tasks/pause-all");
  }
  resumeAll(): Promise<{ count: number }> {
    return this.post("/api/v1/tasks/resume-all");
  }
  retryFailed(): Promise<{ count: number }> {
    return this.post("/api/v1/tasks/retry-failed");
  }
  clearCompleted(): Promise<{ count: number }> {
    return this.post("/api/v1/tasks/clear-completed");
  }

  // ----- torrents -----
  torrentPeers(id: string): Promise<PeerInfo[]> {
    return this.get(`/api/v1/tasks/${encodeURIComponent(id)}/peers`);
  }
  setTorrentFiles(id: string, selection: FileSelection[]): Promise<Task> {
    return this.put(`/api/v1/tasks/${encodeURIComponent(id)}/files`, selection);
  }
  setSequential(id: string, sequential: boolean): Promise<Task> {
    return this.post(`/api/v1/tasks/${encodeURIComponent(id)}/sequential`, { sequential });
  }
  setSeedingLimits(id: string, limits: SeedingLimits): Promise<Task> {
    return this.put(`/api/v1/tasks/${encodeURIComponent(id)}/seeding`, limits);
  }
  addTrackers(id: string, trackers: string[]): Promise<Task> {
    return this.post(`/api/v1/tasks/${encodeURIComponent(id)}/trackers`, { trackers });
  }
  removeTracker(id: string, tracker: string): Promise<Task> {
    return this.del(`/api/v1/tasks/${encodeURIComponent(id)}/trackers`, { tracker });
  }
  setTrackerEnabled(id: string, tracker: string, enabled: boolean): Promise<Task> {
    return this.post(`/api/v1/tasks/${encodeURIComponent(id)}/trackers/enable`, { tracker, enabled });
  }

  // ----- queues -----
  queueSummaries(): Promise<QueueSummary[]> {
    return this.get("/api/v1/queues/summaries");
  }
  listQueues(): Promise<Queue[]> {
    return this.get("/api/v1/queues");
  }
  pauseQueue(id: string): Promise<Queue> {
    return this.post(`/api/v1/queues/${encodeURIComponent(id)}/pause`);
  }
  resumeQueue(id: string): Promise<Queue> {
    return this.post(`/api/v1/queues/${encodeURIComponent(id)}/resume`);
  }

  // ----- devices -----
  forgetDevice(id: string): Promise<void> {
    return this.del(`/api/v1/devices/${encodeURIComponent(id)}`);
  }
  renameDevice(id: string, name: string): Promise<Device> {
    return this.patch(`/api/v1/devices/${encodeURIComponent(id)}`, { name });
  }
}
