/**
 * Transient badge overlay used when we deliberately *didn't* intercept a matching download (host
 * unreachable, or the add-task call failed) so the user has a visible clue instead of silence.
 * Reverts to the normal active-count badge after a few seconds.
 */

import browser from 'webextension-polyfill';
import { loadSessionState } from './state.ts';

const HINT_DURATION_MS = 6_000;
let revertTimer: ReturnType<typeof setTimeout> | undefined;

export async function showBadgeHint(text: string, color: string): Promise<void> {
  if (revertTimer !== undefined) clearTimeout(revertTimer);
  await browser.action.setBadgeText({ text });
  await browser.action.setBadgeBackgroundColor({ color });
  revertTimer = setTimeout(() => {
    revertTimer = undefined;
    revertToCount().catch(() => {});
  }, HINT_DURATION_MS);
}

async function revertToCount(): Promise<void> {
  const state = await loadSessionState();
  const text = state.badgeCount > 0 ? String(Math.min(state.badgeCount, 999)) : '';
  await browser.action.setBadgeText({ text });
  await browser.action.setBadgeBackgroundColor({ color: '#2f6fed' });
}
