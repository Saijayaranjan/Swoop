// WebSocket client for `/api/v1/events`: connects, reconnects with backoff, applies task/stat
// events into the stores, and resyncs the full task table on `hello`, `lagged`, a detected
// sequence gap, or reconnect (docs/api/websocket.md).

import { ApiClient } from "../api/client.ts";
import { createStore } from "./store.ts";
import { pushToast } from "./toast.ts";
import {
  applyProgress,
  pushSpeedSample,
  removeTaskRow,
  setGlobalStats,
  setQueueSummaries,
  setTaskSnapshot,
  upsertTaskRow,
} from "./tasks.ts";
import type { GlobalStats, Notification, ProgressUpdate, QueueSummary, TaskRow, WsFrame } from "../api/types.ts";

export type ConnectionStatus = "connecting" | "open" | "reconnecting" | "closed";

export const connectionStatusStore = createStore<ConnectionStatus>("connecting");

const MIN_BACKOFF_MS = 1000;
const MAX_BACKOFF_MS = 30000;
const STATS_POLL_MS = 4000;

function notificationMessage(n: Notification): { message: string; kind: "info" | "success" | "error" } {
  switch (n.kind) {
    case "completed":
      return { message: `${n.name ?? "Download"} finished downloading`, kind: "success" };
    case "failed":
      return { message: `${n.name ?? "Download"} failed${n.reason ? `: ${n.reason}` : ""}`, kind: "error" };
    case "torrent_finished":
      return { message: `${n.name ?? "Torrent"} finished`, kind: "success" };
    case "low_disk_space":
      return { message: `Low disk space on ${n.path ?? "destination"}`, kind: "error" };
    case "duplicate_detected":
      return { message: `${n.name ?? "Download"} looks like a duplicate`, kind: "info" };
    case "update_available":
      return { message: `Osprey ${n.version ?? ""} is available`, kind: "info" };
    case "queue_finished":
      return { message: `Queue ${n.name ?? ""} finished`, kind: "info" };
    case "device_paired":
      return { message: `New device paired: ${n.name ?? ""}`, kind: "info" };
    case "automation_failed":
      return { message: `Automation ${n.name ?? ""} failed`, kind: "error" };
    case "checksum_mismatch":
      return { message: `${n.name ?? "Download"} failed checksum verification`, kind: "error" };
    default:
      return { message: n.name ?? "Notification", kind: "info" };
  }
}

export class EventsClient {
  private socket: WebSocket | null = null;
  private lastSeq: number | null = null;
  private backoffMs = MIN_BACKOFF_MS;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private statsPollTimer: ReturnType<typeof setInterval> | null = null;
  private closedByUser = false;
  private readonly api: ApiClient;
  private readonly getToken: () => string | null;
  private readonly getHighVolume: () => boolean;

  constructor(api: ApiClient, getToken: () => string | null, getHighVolume: () => boolean) {
    this.api = api;
    this.getToken = getToken;
    this.getHighVolume = getHighVolume;
  }

  start(): void {
    this.closedByUser = false;
    this.connect();
    this.ensureStatsPolling();
  }

  stop(): void {
    this.closedByUser = true;
    if (this.reconnectTimer !== null) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.statsPollTimer !== null) {
      clearInterval(this.statsPollTimer);
      this.statsPollTimer = null;
    }
    this.socket?.close();
    this.socket = null;
    connectionStatusStore.setState("closed");
  }

  private ensureStatsPolling(): void {
    if (this.statsPollTimer !== null) return;
    this.statsPollTimer = setInterval(() => {
      this.api.stats().then(setGlobalStats).catch(() => {});
    }, STATS_POLL_MS);
  }

  private wsUrl(): string {
    const token = this.getToken() ?? "";
    const highVolume = this.getHighVolume();
    const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
    const params = new URLSearchParams({ token, high_volume: String(highVolume) });
    return `${proto}//${window.location.host}/api/v1/events?${params.toString()}`;
  }

  private connect(): void {
    if (typeof window === "undefined" || !this.getToken()) {
      return;
    }
    connectionStatusStore.setState((prev) => (prev === "open" ? prev : "connecting"));
    let socket: WebSocket;
    try {
      socket = new WebSocket(this.wsUrl());
    } catch {
      this.scheduleReconnect();
      return;
    }
    this.socket = socket;

    socket.addEventListener("open", () => {
      this.backoffMs = MIN_BACKOFF_MS;
      connectionStatusStore.setState("open");
    });

    socket.addEventListener("message", (event: MessageEvent<string>) => {
      this.handleMessage(event.data);
    });

    socket.addEventListener("close", () => {
      this.socket = null;
      if (!this.closedByUser) {
        connectionStatusStore.setState("reconnecting");
        this.scheduleReconnect();
      }
    });

    socket.addEventListener("error", () => {
      socket.close();
    });
  }

  private scheduleReconnect(): void {
    if (this.closedByUser || this.reconnectTimer !== null) return;
    const delay = this.backoffMs + Math.floor(Math.random() * 250);
    this.backoffMs = Math.min(this.backoffMs * 2, MAX_BACKOFF_MS);
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.connect();
    }, delay);
  }

  private resync(): void {
    this.api
      .listRows({})
      .then(setTaskSnapshot)
      .catch(() => {});
    this.api
      .queueSummaries()
      .then(setQueueSummaries)
      .catch(() => {});
  }

  private handleMessage(raw: string): void {
    let frame: WsFrame;
    try {
      frame = JSON.parse(raw) as WsFrame;
    } catch {
      return;
    }

    if (typeof frame.seq === "number") {
      if (this.lastSeq !== null && frame.seq > this.lastSeq + 1) {
        this.resync();
      }
      this.lastSeq = frame.seq;
    }

    switch (frame.type) {
      case "hello":
        this.resync();
        break;
      case "lagged":
        this.resync();
        break;
      case "ping":
        this.socket?.send(JSON.stringify({ type: "pong" }));
        break;
      case "task_added":
      case "task_updated":
        upsertTaskRow(frame.data as TaskRow);
        break;
      case "task_removed": {
        const data = frame.data as { task_id: string };
        removeTaskRow(data.task_id);
        break;
      }
      case "progress":
        applyProgress(frame.data as ProgressUpdate[]);
        break;
      case "global_stats": {
        const stats = frame.data as GlobalStats;
        setGlobalStats(stats);
        pushSpeedSample({ at: stats.at, download: stats.download_speed, upload: stats.upload_speed });
        break;
      }
      case "queue_summaries":
        setQueueSummaries(frame.data as QueueSummary[]);
        break;
      case "notification": {
        const { message, kind } = notificationMessage(frame.data as Notification);
        pushToast(message, kind);
        break;
      }
      default:
        break;
    }
  }
}
