// Client-side filtering/sorting for the Downloads list. The full task table is kept in sync via
// WebSocket (see state/ws.ts), so filtering/sorting/search all run locally for instant feedback
// instead of round-tripping to the server on every keystroke or chip tap.

import type { TaskRow, TaskState } from "../api/types.ts";

export type FilterChip = "all" | "active" | "queued" | "completed" | "failed" | "torrents" | "paused";

const ACTIVE_STATES: ReadonlySet<TaskState> = new Set([
  "resolving",
  "connecting",
  "downloading",
  "retrying",
  "verifying",
  "processing",
  "seeding",
]);

export function matchesChip(row: TaskRow, chip: FilterChip): boolean {
  switch (chip) {
    case "all":
      return true;
    case "active":
      return ACTIVE_STATES.has(row.state);
    case "queued":
      return row.state === "queued" || row.state === "pending" || row.state === "scheduled";
    case "completed":
      return row.state === "completed";
    case "failed":
      return row.state === "failed";
    case "torrents":
      return row.kind === "torrent" || row.kind === "magnet";
    case "paused":
      return row.state === "paused";
  }
}

export function matchesSearch(row: TaskRow, query: string): boolean {
  if (query.trim().length === 0) return true;
  const q = query.trim().toLowerCase();
  return row.name.toLowerCase().includes(q) || (row.domain ?? "").toLowerCase().includes(q);
}

export type SortKey = "position" | "name" | "size" | "progress" | "speed" | "eta" | "state" | "created_at";

function progressFraction(row: TaskRow): number {
  if (row.progress.total && row.progress.total > 0) {
    return row.progress.downloaded / row.progress.total;
  }
  return row.progress.fraction;
}

export function compareRows(a: TaskRow, b: TaskRow, key: SortKey, descending: boolean): number {
  let result: number;
  switch (key) {
    case "name":
      result = a.name.localeCompare(b.name);
      break;
    case "size":
      result = (a.progress.total ?? 0) - (b.progress.total ?? 0);
      break;
    case "progress":
      result = progressFraction(a) - progressFraction(b);
      break;
    case "speed":
      result = a.progress.speed - b.progress.speed;
      break;
    case "eta":
      result = (a.progress.eta_seconds ?? Infinity) - (b.progress.eta_seconds ?? Infinity);
      break;
    case "state":
      result = a.state.localeCompare(b.state);
      break;
    case "created_at":
      result = a.created_at - b.created_at;
      break;
    case "position":
    default:
      result = a.position - b.position;
      break;
  }
  return descending ? -result : result;
}

export function filterAndSortRows(
  rows: readonly TaskRow[],
  chip: FilterChip,
  search: string,
  sortKey: SortKey,
  descending: boolean,
): TaskRow[] {
  return rows
    .filter((r) => matchesChip(r, chip) && matchesSearch(r, search))
    .sort((a, b) => compareRows(a, b, sortKey, descending));
}
