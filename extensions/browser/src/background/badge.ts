/**
 * Badge text = active download count, refreshed from `global_stats` (authoritative counts) and
 * nudged instantly on `task_state_changed` so the badge does not lag a full stats tick behind a
 * pause/resume/completion the user just triggered (docs/api/extension.md, events.rs).
 */

import browser from 'webextension-polyfill';
import type { GlobalStats, SwoopEvent, TaskStateChangedData } from '../shared/types.ts';
import { ACTIVE_TASK_STATES } from '../shared/types.ts';
import { setBadgeCount } from './state.ts';

const BADGE_COLOR = '#2f6fed';

export async function updateBadge(count: number): Promise<void> {
  await setBadgeCount(count);
  const text = count > 0 ? String(Math.min(count, 999)) : '';
  await browser.action.setBadgeText({ text });
  if (count > 0) {
    await browser.action.setBadgeBackgroundColor({ color: BADGE_COLOR });
  }
}

export async function clearBadge(): Promise<void> {
  await updateBadge(0);
}

/** Tracks state transitions locally so a single `task_state_changed` can adjust the badge by ±1
 *  without waiting for the next `global_stats` tick. */
const locallyActiveTasks = new Set<string>();

function applyTaskStateChange(data: TaskStateChangedData): number | null {
  const wasActive = locallyActiveTasks.has(data.task_id);
  const isActive = ACTIVE_TASK_STATES.has(data.to);
  if (isActive) {
    locallyActiveTasks.add(data.task_id);
  } else {
    locallyActiveTasks.delete(data.task_id);
  }
  if (wasActive === isActive) return null;
  return locallyActiveTasks.size;
}

export async function handleEventForBadge(event: SwoopEvent): Promise<void> {
  if (event.type === 'global_stats') {
    const stats = event.data as GlobalStats;
    await updateBadge(stats.active);
    return;
  }
  if (event.type === 'task_state_changed') {
    const next = applyTaskStateChange(event.data as TaskStateChangedData);
    if (next !== null) await updateBadge(next);
  }
}
