import { test } from "node:test";
import assert from "node:assert/strict";
import {
  createTaskTable,
  applySnapshot,
  applyTaskRow,
  applyTaskRemoved,
  applyProgressBatch,
  tableToSortedArray,
  byPosition,
} from "./taskMerge.ts";
import type { Progress, TaskRow } from "../api/types.ts";

function progress(overrides: Partial<Progress> = {}): Progress {
  return {
    downloaded: 0,
    uploaded: 0,
    total: 1000,
    speed: 0,
    instant_speed: 0,
    upload_speed: 0,
    eta_seconds: null,
    active_connections: 0,
    peers: 0,
    seeds: 0,
    ratio: 0,
    fraction: 0,
    ...overrides,
  };
}

function row(overrides: Partial<TaskRow> = {}): TaskRow {
  return {
    id: "t1",
    name: "file.zip",
    kind: "http",
    state: "downloading",
    domain: "example.com",
    progress: progress(),
    queue_id: "queue-default",
    category_id: null,
    priority: "normal",
    position: 0,
    created_at: 0,
    error_kind: null,
    health: 100,
    file_path: null,
    ...overrides,
  };
}

test("createTaskTable indexes rows by id", () => {
  const t = createTaskTable([row({ id: "a" }), row({ id: "b" })]);
  assert.equal(t.rows.size, 2);
  assert.ok(t.rows.has("a") && t.rows.has("b"));
});

test("applyTaskRow upserts and resets progress dedup tracking", () => {
  let t = createTaskTable([row({ id: "t1" })]);
  t = applyProgressBatch(t, [{ task_id: "t1", progress: progress({ downloaded: 500 }), rev: 5 }]);
  assert.equal(t.rows.get("t1")?.progress.downloaded, 500);

  // A fresh full row (e.g. task_updated) should apply even though later progress carries a
  // lower rev than 5 -- the dedup tracking is reset by the authoritative row.
  t = applyTaskRow(t, row({ id: "t1", name: "renamed.zip" }));
  assert.equal(t.rows.get("t1")?.name, "renamed.zip");

  t = applyProgressBatch(t, [{ task_id: "t1", progress: progress({ downloaded: 10 }), rev: 1 }]);
  assert.equal(t.rows.get("t1")?.progress.downloaded, 10);
});

test("applyProgressBatch drops stale and duplicate revs", () => {
  let t = createTaskTable([row({ id: "t1" })]);
  t = applyProgressBatch(t, [{ task_id: "t1", progress: progress({ downloaded: 100 }), rev: 10 }]);
  assert.equal(t.rows.get("t1")?.progress.downloaded, 100);

  // Lower rev: dropped.
  t = applyProgressBatch(t, [{ task_id: "t1", progress: progress({ downloaded: 50 }), rev: 9 }]);
  assert.equal(t.rows.get("t1")?.progress.downloaded, 100);

  // Equal rev: dropped (must be strictly greater).
  t = applyProgressBatch(t, [{ task_id: "t1", progress: progress({ downloaded: 999 }), rev: 10 }]);
  assert.equal(t.rows.get("t1")?.progress.downloaded, 100);

  // Higher rev: applied.
  t = applyProgressBatch(t, [{ task_id: "t1", progress: progress({ downloaded: 200 }), rev: 11 }]);
  assert.equal(t.rows.get("t1")?.progress.downloaded, 200);
});

test("applyProgressBatch ignores unknown task ids", () => {
  const t0 = createTaskTable([row({ id: "t1" })]);
  const t1 = applyProgressBatch(t0, [{ task_id: "unknown", progress: progress(), rev: 1 }]);
  assert.equal(t1, t0); // unchanged reference: no-op short circuit
  assert.equal(t1.rows.size, 1);
});

test("applyTaskRemoved removes the row and its rev tracking", () => {
  let t = createTaskTable([row({ id: "t1" }), row({ id: "t2" })]);
  t = applyProgressBatch(t, [{ task_id: "t1", progress: progress(), rev: 3 }]);
  t = applyTaskRemoved(t, "t1");
  assert.equal(t.rows.has("t1"), false);
  assert.equal(t.revs.has("t1"), false);
  assert.equal(t.rows.size, 1);
});

test("applyTaskRemoved is a no-op for unknown ids", () => {
  const t0 = createTaskTable([row({ id: "t1" })]);
  const t1 = applyTaskRemoved(t0, "nope");
  assert.equal(t1, t0);
});

test("applySnapshot replaces the table wholesale and clears rev tracking", () => {
  let t = createTaskTable([row({ id: "t1" })]);
  t = applyProgressBatch(t, [{ task_id: "t1", progress: progress(), rev: 5 }]);
  t = applySnapshot([row({ id: "t2" })]);
  assert.equal(t.rows.has("t1"), false);
  assert.equal(t.rows.has("t2"), true);
  assert.equal(t.revs.size, 0);
});

test("tableToSortedArray + byPosition sorts by manual queue order", () => {
  const t = createTaskTable([row({ id: "c", position: 3 }), row({ id: "a", position: 1 }), row({ id: "b", position: 2 })]);
  const sorted = tableToSortedArray(t, byPosition);
  assert.deepEqual(sorted.map((r) => r.id), ["a", "b", "c"]);
});
