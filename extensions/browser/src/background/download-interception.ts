/**
 * Download interception (docs/api/extension.md "Interception"):
 *   - Chrome/Edge: `downloads.onDeterminingFilename` (Firefox does not implement this event).
 *   - Firefox: `downloads.onCreated` + `downloads.cancel` + `downloads.erase`.
 * Both paths funnel into `maybeIntercept`, which applies the shared exclusion/extension/size
 * rules (src/shared/url-utils.ts), dedupes by download id (never intercept the same item twice,
 * survives service-worker restarts via `state.ts`), forwards cookies, and — if the native host is
 * unreachable — lets the browser's own download continue and raises a badge hint instead of
 * silently losing the file.
 *
 * After each decision the active tab gets a small in-page prompt (src/content/page-ui.ts):
 * "Sent to Osprey" with a one-click "Use browser instead" (when `show_confirmation` is on), "Kept
 * in your browser" when Osprey was unreachable, and — always — a warning if the hand-off failed
 * after the browser's copy was already cancelled, with a way to download it in the browser.
 */

import browser from 'webextension-polyfill';
import type { NativePort } from './native-port.ts';
import type { SettingsStore } from '../shared/settings.ts';
import { decideInterception } from '../shared/url-utils.ts';
import { buildCookieHeader } from '../shared/cookie-utils.ts';
import { isDownloadHandled, markDownloadHandled } from './state.ts';
import type { AddTaskResult, NewTaskRequest } from '../shared/types.ts';
import type { PagePrompt, PagePromptVariant, ShowPagePromptMessage } from '../shared/messages.ts';
import { hostnameOf } from '../shared/url-utils.ts';
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

// --- In-page prompts -------------------------------------------------------------------------

interface PromptEntry {
  url: string;
  host: string | null;
  taskId: string | null;
}

const PROMPT_TTL_MS = 2 * 60_000;
const prompts = new Map<string, PromptEntry>();
/** URLs the user just asked to download in the browser instead; the next matching download is
 *  left alone once (single use, short-lived). */
const bypassUrls = new Map<string, number>();

function consumeBypass(url: string): boolean {
  const until = bypassUrls.get(url);
  if (until === undefined) return false;
  bypassUrls.delete(url);
  return until > Date.now();
}

function fileNameFor(item: DownloadItem): string {
  const fromItem = item.filename?.split(/[\\/]/).pop();
  if (fromItem) return fromItem;
  try {
    const last = new URL(item.url).pathname.split('/').filter(Boolean).pop();
    if (last) return decodeURIComponent(last);
  } catch {
    // fall through
  }
  return hostnameOf(item.url) ?? item.url;
}

async function showPagePrompt(
  item: DownloadItem,
  variant: PagePromptVariant,
  taskId: string | null,
  canLaunch: boolean,
): Promise<void> {
  const promptId = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
  const host = hostnameOf(item.url);
  prompts.set(promptId, { url: item.url, host, taskId });
  setTimeout(() => prompts.delete(promptId), PROMPT_TTL_MS);

  const size = item.totalBytes && item.totalBytes > 0 ? item.totalBytes : item.fileSize && item.fileSize > 0 ? item.fileSize : null;
  const prompt: PagePrompt = { promptId, variant, fileName: fileNameFor(item), host, size, canLaunch };
  try {
    const [tab] = await browser.tabs.query({ active: true, lastFocusedWindow: true });
    if (tab?.id === undefined || !/^https?:/i.test(tab.url ?? '')) return;
    const message: ShowPagePromptMessage = { type: 'osprey-page-prompt', prompt };
    await browser.tabs.sendMessage(tab.id, message);
  } catch {
    // No content script on this tab (store pages, PDFs, …): the badge hint still shows.
  }
}

/** Handles a button pressed on an in-page prompt (via message-router.ts). */
export async function handlePromptAction(
  promptId: string,
  action: 'use-browser' | 'always-browser',
  deps: InterceptionDeps,
): Promise<void> {
  const entry = prompts.get(promptId);
  if (!entry) throw new Error('This prompt has expired.');
  if (action === 'always-browser') {
    if (entry.host) await deps.settingsStore.addAlwaysBrowserDomain(entry.host);
    return;
  }
  if (entry.taskId) {
    await deps.nativePort
      .request('DELETE', `/api/v1/tasks/${encodeURIComponent(entry.taskId)}?delete_file=true`)
      .catch(() => {});
    entry.taskId = null;
  }
  bypassUrls.set(entry.url, Date.now() + 30_000);
  await browser.downloads.download({ url: entry.url });
}

/** Returns `true` if the item was (or is being) cancelled and forwarded to Osprey. */
async function maybeIntercept(
  item: DownloadItem,
  deps: InterceptionDeps,
  cancel: () => Promise<void>,
): Promise<boolean> {
  if (await isDownloadHandled(item.id)) return false;
  if (consumeBypass(item.url)) {
    await markDownloadHandled(item.id);
    return false;
  }

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
    if (settings.show_confirmation) {
      void showPagePrompt(item, 'not-running', null, status.mode === 'native');
    }
    return false;
  }

  await markDownloadHandled(item.id);
  await cancel();

  const request = await buildTaskRequest(item);
  try {
    const result = await deps.nativePort.post<AddTaskResult>('/api/v1/tasks', request);
    if (settings.show_confirmation) {
      void showPagePrompt(item, 'sent', result?.task?.id ?? null, status.mode === 'native');
    }
  } catch {
    await showBadgeHint('!', '#dc2626');
    // The browser's copy is already gone — always tell the user and offer it back.
    void showPagePrompt(item, 'failed', null, false);
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
