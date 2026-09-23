/**
 * Download interception (docs/api/extension.md "Interception"):
 *   - Chrome/Edge: `downloads.onDeterminingFilename` (Firefox does not implement this event).
 *   - Firefox: `downloads.onCreated` + `downloads.cancel` + `downloads.erase`.
 * Both paths funnel into `maybeIntercept`, which applies the shared exclusion/extension/size
 * rules (src/shared/url-utils.ts), dedupes by download id (never intercept the same item twice,
 * survives service-worker restarts via `state.ts`), forwards cookies, and — if the native host is
 * unreachable — lets the browser's own download continue and raises a badge hint instead of
 * silently losing the file.
 */

import browser from 'webextension-polyfill';
import type { NativePort } from './native-port.ts';
import type { SettingsStore } from '../shared/settings.ts';
import { decideInterception } from '../shared/url-utils.ts';
import { buildCookieHeader } from '../shared/cookie-utils.ts';
import { isDownloadHandled, markDownloadHandled } from './state.ts';
import type { AddTaskResult, NewTaskRequest } from '../shared/types.ts';
import { showBadgeHint } from './badge-hint.ts';

type DownloadItem = browser.Downloads.DownloadItem;

async function guessTabTitle(referrer: string | undefined): Promise<string | null> {
  if (!referrer) return null;
  try {
    const tabs = await browser.tabs.query({ url: referrer });
    return tabs[0]?.title ?? null;
  } catch {
    return null;
  }
}

async function buildCookieString(url: string): Promise<string | undefined> {
  try {
    const cookies = await browser.cookies.getAll({ url });
    const header = buildCookieHeader(cookies);
    return header.length > 0 ? header : undefined;
  } catch {
    return undefined;
  }
}

async function buildTaskRequest(item: DownloadItem): Promise<NewTaskRequest> {
  const cookies = await buildCookieString(item.url);
  const title = await guessTabTitle(item.referrer);
  const suggestedName = item.filename?.split(/[\\/]/).pop();
  return {
    url: item.url,
    name: title && !suggestedName ? undefined : suggestedName || undefined,
    start: true,
    origin: 'browser',
    referer_page: item.referrer || undefined,
    options: {
      referer: item.referrer || undefined,
      cookies,
    },
  };
}

export interface InterceptionDeps {
  nativePort: NativePort;
  settingsStore: SettingsStore;
}

/** Returns `true` if the item was (or is being) cancelled and forwarded to Osprey. */
async function maybeIntercept(
  item: DownloadItem,
  deps: InterceptionDeps,
  cancel: () => Promise<void>,
): Promise<boolean> {
  if (await isDownloadHandled(item.id)) return false;

  const settings = await deps.settingsStore.get();
  const decision = decideInterception({
    url: item.url,
    bytesKnown: item.fileSize && item.fileSize > 0 ? item.fileSize : item.totalBytes ?? null,
    settings,
    alwaysBrowserDomains: settings.always_browser_domains,
  });
  if (!decision.intercept) return false;

  // Check the app is actually reachable before we cancel the browser's own copy — otherwise the
  // user loses the file entirely. "let the browser continue and show a badge hint" (spec).
  const status = await deps.nativePort.ping();
  if (!status.running) {
    await showBadgeHint('!', '#d97706');
    return false;
  }

  await markDownloadHandled(item.id);
  await cancel();

  const request = await buildTaskRequest(item);
  try {
    await deps.nativePort.post<AddTaskResult>('/api/v1/tasks', request);
  } catch {
    await showBadgeHint('!', '#dc2626');
  }
  return true;
}

export function registerDownloadInterception(deps: InterceptionDeps): void {
  const downloads = browser.downloads as browser.Downloads.Static & {
    onDeterminingFilename?: {
      addListener(
        cb: (item: DownloadItem, suggest: (suggestion?: { filename: string }) => void) => void,
      ): void;
    };
  };

  if (downloads.onDeterminingFilename) {
    // Chrome / Edge: fires before the file is written. We let the browser proceed with its own
    // suggestion immediately (so the UI never stalls waiting on us), then cancel+erase right
    // after if interception applies — the standard pattern for this event, since it offers no
    // synchronous "refuse this download" hook.
    downloads.onDeterminingFilename.addListener((item, suggest) => {
      suggest();
      maybeIntercept(item, deps, async () => {
        await browser.downloads.cancel(item.id).catch(() => {});
        await browser.downloads.erase({ id: item.id }).catch(() => {});
      }).catch(() => {});
    });
  } else {
    // Firefox: the item already exists (and may have started writing) by the time onCreated
    // fires; cancel + erase it from history once we decide to take it over.
    browser.downloads.onCreated.addListener((item) => {
      maybeIntercept(item, deps, async () => {
        await browser.downloads.cancel(item.id).catch(() => {});
        await browser.downloads.erase({ id: item.id }).catch(() => {});
      }).catch(() => {});
    });
  }
}
