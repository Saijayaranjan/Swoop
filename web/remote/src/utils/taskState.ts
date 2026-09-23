// Small state-machine helpers mirroring crates/osprey-domain/src/state.rs so the UI only shows
// actions that the server will actually accept.

import type { TaskState } from "../api/types.ts";

const PAUSABLE: ReadonlySet<TaskState> = new Set([
  "queued",
  "scheduled",
  "resolving",
  "connecting",
  "downloading",
  "retrying",
  "seeding",
]);

const RESUMABLE: ReadonlySet<TaskState> = new Set(["paused", "failed", "cancelled", "pending"]);

const TERMINAL: ReadonlySet<TaskState> = new Set(["completed", "failed", "cancelled"]);

export function canPause(state: TaskState): boolean {
  return PAUSABLE.has(state);
}

export function canResume(state: TaskState): boolean {
  return RESUMABLE.has(state);
}

export function canRetry(state: TaskState): boolean {
  return state === "failed";
}

export function canCancel(state: TaskState): boolean {
  return !TERMINAL.has(state);
}

export function isActive(state: TaskState): boolean {
  return (
    state === "resolving" ||
    state === "connecting" ||
    state === "downloading" ||
    state === "retrying" ||
    state === "verifying" ||
    state === "processing" ||
    state === "seeding"
  );
}
