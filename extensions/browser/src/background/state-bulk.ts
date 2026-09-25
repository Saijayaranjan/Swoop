/**
 * One-shot handoff of candidate links from a "Download all links on page…" / "…in selection"
 * context-menu action to the popup's links picker, via `storage.session` (survives the popup opening
 * in a fresh script context, cleared once the popup consumes it).
 */

import browser from 'webextension-polyfill';
import type { CandidateLink } from '../shared/messages.ts';

const KEY = 'swoopPendingBulkLinks';

function sessionArea(): browser.Storage.StorageArea | undefined {
  return (browser.storage as unknown as { session?: browser.Storage.StorageArea }).session;
}

export async function setPendingBulkLinks(links: CandidateLink[]): Promise<void> {
  const area = sessionArea();
  if (!area) return;
  await area.set({ [KEY]: links });
}

export async function takePendingBulkLinks(): Promise<CandidateLink[]> {
  const area = sessionArea();
  if (!area) return [];
  const stored = await area.get(KEY);
  const value = stored[KEY];
  await area.remove(KEY);
  return Array.isArray(value) ? (value as CandidateLink[]) : [];
}
