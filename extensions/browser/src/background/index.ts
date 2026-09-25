/**
 * Background entry point (MV3 service worker on Chrome/Edge; MV3 event page on Firefox — see
 * manifests/firefox.json). Every `browser.*.on*.addListener` call below runs synchronously during
 * the initial script evaluation, which is required for Chrome to correctly re-wake a suspended
 * service worker when one of those events fires; only the native-port connection/subscription
 * (genuinely async, and lazy by design — see native-port.ts) happens inside the IIFE at the
 * bottom.
 */

import browser from 'webextension-polyfill';
import { NativePort } from './native-port.ts';
import { SettingsStore } from '../shared/settings.ts';
import { MediaDetector } from '../media-detector.ts';
import { registerDownloadInterception } from './download-interception.ts';
import { createContextMenus, registerContextMenuHandlers } from './context-menus.ts';
import { registerCommands } from './commands.ts';
import { registerMessageRouter } from './message-router.ts';
import { registerPopupPort } from './popup-port.ts';
import { handleEventForBadge, updateBadge } from './badge.ts';
import { handleEventForNotifications } from './notifications.ts';
import { loadSessionState } from './state.ts';
import { REMOTE_TOKEN_KEY, normalizeRemoteUrl } from '../shared/remote.ts';
import type { OspreyEvent } from '../shared/types.ts';

const NATIVE_EVENTS = [
  'task_added',
  'task_updated',
  'task_removed',
  'task_state_changed',
  'global_stats',
  'notification',
  'settings_changed',
];

export const nativePort = new NativePort();
export const settingsStore = new SettingsStore(browser.storage.local);
export const mediaDetector = new MediaDetector(settingsStore);

// --- Synchronous listener registration (must happen at top level; see header comment) ---------

mediaDetector.register();
registerDownloadInterception({ nativePort, settingsStore });
registerContextMenuHandlers(nativePort);
registerCommands(nativePort);
registerMessageRouter({ nativePort, mediaDetector, settingsStore });
// High-volume `progress` events are only worth their cost while a popup is watching.
registerPopupPort(nativePort, {
  onOpenCountChange(count) {
    const events = count > 0 ? [...NATIVE_EVENTS, 'progress'] : NATIVE_EVENTS;
    nativePort.subscribe(events).catch(() => {});
  },
});

browser.runtime.onInstalled.addListener(() => {
  createContextMenus();
});

nativePort.onEvent((rawEvent) => {
  const event = rawEvent as OspreyEvent;
  void handleEventForBadge(event);
  if (event.type === 'notification') {
    settingsStore
      .get()
      .then((settings) => handleEventForNotifications(event, settings))
      .catch(() => {});
  }
});

// Keep extension storage (settings and the remote device token) out of reach of content scripts,
// which run inside web pages. They get the little they need via messages instead. Chrome/Edge
// only; Firefox has no per-area access levels.
try {
  const localArea = (globalThis as { chrome?: { storage?: { local?: {
    setAccessLevel?: (options: { accessLevel: string }) => Promise<void> | void;
  } } } }).chrome?.storage?.local;
  void Promise.resolve(localArea?.setAccessLevel?.({ accessLevel: 'TRUSTED_CONTEXTS' })).catch(() => {});
} catch {
  // Not supported here: content scripts still never touch storage themselves.
}

/** Point the connection at the local native host or the paired remote Osprey, per settings. */
async function configureConnection(): Promise<void> {
  const settings = await settingsStore.get();
  if (settings.connection_mode !== 'remote') {
    nativePort.useRemote(null);
    return;
  }
  const stored = await browser.storage.local.get(REMOTE_TOKEN_KEY);
  const token = stored[REMOTE_TOKEN_KEY];
  const baseUrl = normalizeRemoteUrl(settings.remote_url);
  if (typeof token === 'string' && token.length > 0 && baseUrl) {
    nativePort.useRemote({ baseUrl, token });
  } else {
    nativePort.useRemote(null);
  }
}

settingsStore.onChange(() => {
  void configureConnection();
});
browser.storage.onChanged.addListener((changes, area) => {
  if (area === 'local' && REMOTE_TOKEN_KEY in changes) void configureConnection();
});

// --- Async startup work --------------------------------------------------------------------

void (async () => {
  // Repaint the badge from whatever we last knew, immediately — don't wait for a fresh
  // `global_stats` tick after a service-worker restart.
  const session = await loadSessionState();
  await updateBadge(session.badgeCount);

  await configureConnection().catch(() => {});

  // Keep retrying (with backoff) as long as the extension is alive: this is also what keeps an
  // MV3 service worker from being suspended while Osprey is actively reporting progress.
  await nativePort.subscribe(NATIVE_EVENTS).catch(() => {
    // Native host not installed/reachable yet; native-port.ts already scheduled a retry.
  });
})();
