/**
 * Ephemeral background state that must survive an MV3 service-worker suspension/restart, kept in
 * `browser.storage.session` (in-memory, cleared on browser restart, never written to disk) rather
 * than module-level variables which are lost the moment the worker is killed.
 */

import browser from 'webextension-polyfill';

const KEY = 'swoopSessionState';
const MAX_HANDLED_DOWNLOAD_IDS = 200;

export interface SessionState {
  badgeCount: number;
  popupOpen: boolean;
  /** Browser `downloads.DownloadItem.id`s already handled (cancelled+forwarded, or deliberately
   *  left to the browser), so a service-worker restart mid-download never double-handles one. */
  handledDownloadIds: number[];
}

function defaultState(): SessionState {
  return { badgeCount: 0, popupOpen: false, handledDownloadIds: [] };
}

// `storage.session` landed after `storage.local`/`storage.sync`; guard for older engines/tests.
function sessionArea(): browser.Storage.StorageArea | undefined {
  return (browser.storage as unknown as { session?: browser.Storage.StorageArea }).session;
}

export async function loadSessionState(): Promise<SessionState> {
  const area = sessionArea();
  if (!area) return defaultState();
  const stored = await area.get(KEY);
  const value = stored[KEY];
  if (typeof value !== 'object' || value === null) return defaultState();
  return { ...defaultState(), ...(value as Partial<SessionState>) };
}

export async function saveSessionState(state: SessionState): Promise<void> {
  const area = sessionArea();
  if (!area) return;
  await area.set({ [KEY]: state });
}

export async function markDownloadHandled(downloadId: number): Promise<void> {
  const state = await loadSessionState();
  if (state.handledDownloadIds.includes(downloadId)) return;
  const next = [...state.handledDownloadIds, downloadId].slice(-MAX_HANDLED_DOWNLOAD_IDS);
  await saveSessionState({ ...state, handledDownloadIds: next });
}

export async function isDownloadHandled(downloadId: number): Promise<boolean> {
  const state = await loadSessionState();
  return state.handledDownloadIds.includes(downloadId);
}

export async function setBadgeCount(count: number): Promise<void> {
  const state = await loadSessionState();
  await saveSessionState({ ...state, badgeCount: count });
}

export async function setPopupOpen(open: boolean): Promise<void> {
  const state = await loadSessionState();
  await saveSessionState({ ...state, popupOpen: open });
}
