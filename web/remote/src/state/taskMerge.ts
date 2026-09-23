// Rev-based merge logic for applying WebSocket events onto the client's task table.
// Mirrors the server's contract (docs/api/websocket.md): `task_added`/`task_updated` carry a
// full TaskRow (authoritative snapshot, no `rev` field of its own); `progress` batches carry
// `{task_id, progress, rev}` and clients must ignore any progress frame whose `rev` is not
// strictly newer than the one already applied for that task. We track that "last applied
// progress rev" separately from the row itself, since `TaskRow` doesn't carry it.

import type { ProgressUpdate, TaskRow } from "../api/types.ts";

export interface TaskTableState {
  readonly rows: ReadonlyMap<string, TaskRow>;
  readonly revs: ReadonlyMap<string, number>;
}

const NO_REV = -1;

export function createTaskTable(rows: readonly TaskRow[] = []): TaskTableState {
  const rowMap = new Map<string, TaskRow>();
  for (const row of rows) {
    rowMap.set(row.id, row);
  }
  return { rows: rowMap, revs: new Map() };
}

/** Replace the table wholesale (used on `hello`/`lagged`/reconnect via `GET /tasks/rows`). */
export function applySnapshot(rows: readonly TaskRow[]): TaskTableState {
  return createTaskTable(rows);
}

/** Apply a `task_added` or `task_updated` event: an authoritative full row. */
export function applyTaskRow(state: TaskTableState, row: TaskRow): TaskTableState {
  const rows = new Map(state.rows);
  rows.set(row.id, row);
  const revs = new Map(state.revs);
  revs.delete(row.id); // the fresh row already reflects current progress; reset dedup tracking
  return { rows, revs };
}

/** Apply a `task_removed` event. */
export function applyTaskRemoved(state: TaskTableState, taskId: string): TaskTableState {
  if (!state.rows.has(taskId)) {
    return state;
  }
  const rows = new Map(state.rows);
  rows.delete(taskId);
  const revs = new Map(state.revs);
  revs.delete(taskId);
  return { rows, revs };
}

/**
 * Apply a coalesced `progress` batch. Each entry is applied only if its `rev` is strictly
 * greater than the last-applied rev for that task (out-of-order or duplicate frames are
 * dropped). Updates for unknown task ids are ignored (a `task_added` should arrive separately,
 * or a resync will pick it up).
 */
export function applyProgressBatch(state: TaskTableState, updates: readonly ProgressUpdate[]): TaskTableState {
  let rows: Map<string, TaskRow> | null = null;
  let revs: Map<string, number> | null = null;
  for (const update of updates) {
    const existingRow = state.rows.get(update.task_id);
    if (!existingRow) {
      continue;
    }
    const lastRev = revs?.get(update.task_id) ?? state.revs.get(update.task_id) ?? NO_REV;
    if (update.rev <= lastRev) {
      continue;
    }
    if (rows === null) {
      rows = new Map(state.rows);
      revs = new Map(state.revs);
    }
    rows.set(update.task_id, { ...existingRow, progress: update.progress });
    (revs as Map<string, number>).set(update.task_id, update.rev);
  }
  if (rows === null) {
    return state;
  }
  return { rows, revs: revs as Map<string, number> };
}

export function tableToSortedArray(
  state: TaskTableState,
  compare: (a: TaskRow, b: TaskRow) => number,
): TaskRow[] {
  return Array.from(state.rows.values()).sort(compare);
}

/** Default sort: manual queue position, ascending (matches `TaskSort::Position`, the server default). */
export function byPosition(a: TaskRow, b: TaskRow): number {
  return a.position - b.position;
}
