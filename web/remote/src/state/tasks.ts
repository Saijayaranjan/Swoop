// Task table + global stats + queue summaries store, updated by REST snapshots and WS events.

import { createStore } from "./store.ts";
import {
  applyProgressBatch,
  applySnapshot,
  applyTaskRemoved,
  applyTaskRow,
  createTaskTable,
  type TaskTableState,
} from "./taskMerge.ts";
import type { GlobalStats, ProgressUpdate, QueueSummary, TaskRow } from "../api/types.ts";

export const taskTableStore = createStore<TaskTableState>(createTaskTable());

export function setTaskSnapshot(rows: readonly TaskRow[]): void {
  taskTableStore.setState(applySnapshot(rows));
}

export function upsertTaskRow(row: TaskRow): void {
  taskTableStore.setState((s) => applyTaskRow(s, row));
}

export function removeTaskRow(taskId: string): void {
  taskTableStore.setState((s) => applyTaskRemoved(s, taskId));
}

export function applyProgress(updates: readonly ProgressUpdate[]): void {
  taskTableStore.setState((s) => applyProgressBatch(s, updates));
}

const DEFAULT_STATS: GlobalStats = {
  download_speed: 0,
  upload_speed: 0,
  active: 0,
  downloading: 0,
  seeding: 0,
  queued: 0,
  scheduled: 0,
  paused: 0,
  completed_today: 0,
  failed_today: 0,
  total_tasks: 0,
  bytes_today: 0,
  free_space: null,
  network_available: true,
  traffic_mode: "unlimited",
  download_limit: 0,
  upload_limit: 0,
  at: 0,
};

export const globalStatsStore = createStore<GlobalStats>(DEFAULT_STATS);
export const queueSummariesStore = createStore<QueueSummary[]>([]);

/** Header stats are fed by both WS `global_stats` events and a polling fallback; keep the newer one. */
export function setGlobalStats(stats: GlobalStats): void {
  globalStatsStore.setState((prev) => (stats.at >= prev.at ? stats : prev));
}

export function setQueueSummaries(summaries: QueueSummary[]): void {
  queueSummariesStore.setState(summaries);
}

/** Rolling speed history for the dashboard sparkline (kept client-side; capped in length). */
const SPEED_HISTORY_LIMIT = 120;
export interface SpeedPoint {
  at: number;
  download: number;
  upload: number;
}
export const speedHistoryStore = createStore<SpeedPoint[]>([]);

export function pushSpeedSample(point: SpeedPoint): void {
  speedHistoryStore.setState((history) => {
    const next = [...history, point];
    return next.length > SPEED_HISTORY_LIMIT ? next.slice(next.length - SPEED_HISTORY_LIMIT) : next;
  });
}

export function seedSpeedHistory(points: readonly SpeedPoint[]): void {
  speedHistoryStore.setState(points.slice(-SPEED_HISTORY_LIMIT));
}
