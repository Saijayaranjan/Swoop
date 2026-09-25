/**
 * Popup entry point. One glanceable screen: connection status, live speed, an "add link" field,
 * media found on the current page, and active/recent downloads — with a links picker as a sheet
 * and the "Capture downloads" switch, Open Swoop and Settings in the footer.
 *
 * Talks to the background through `runtime.sendMessage` (src/shared/background-client.ts) and
 * the long-lived popup port (live native events), and to the page's content script with
 * `tabs.sendMessage` for on-page scans.
 */

import browser from 'webextension-polyfill';
import { POPUP_PORT_NAME, type ConnectionStatus, type PopupPortMessage } from '../shared/messages.ts';
import { getActiveTabId, launchApp, quickDownload, takePendingBulkLinks } from '../shared/background-client.ts';
import { SettingsStore } from '../shared/settings.ts';
import type { AddTaskResult, NewTaskRequest, SwoopEvent } from '../shared/types.ts';
import { brandMark } from '../shared-ui/brand.ts';
import { applyI18n, tr } from '../shared-ui/dom.ts';
import { hydrateIcons } from '../shared-ui/icons.ts';
import { errorMessage, showToast, type PopupContext } from './context.ts';
import { initDownloads } from './downloads.ts';
import { initLinks } from './links.ts';
import { initMedia } from './media.ts';
import { initStatus, openOptions } from './status.ts';

function byId<T extends HTMLElement>(id: string): T {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el as T;
}

/** Accepts http(s)/ftp URLs and magnet links; returns the task request or null. */
function requestFor(raw: string): NewTaskRequest | null {
  const text = raw.trim();
  if (/^magnet:\?/i.test(text)) return { magnet: text, start: true, origin: 'browser' };
  try {
    const url = new URL(text);
    if (!['http:', 'https:', 'ftp:'].includes(url.protocol)) return null;
    return { url: url.href, start: true, origin: 'browser' };
  } catch {
    return null;
  }
}

function initAddLink(ctx: PopupContext): { setEnabled(enabled: boolean): void } {
  const form = byId<HTMLFormElement>('add-link');
  const input = byId<HTMLInputElement>('add-link-input');
  const paste = byId<HTMLButtonElement>('paste-btn');
  const add = byId<HTMLButtonElement>('add-btn');

  const sync = (): void => {
    const hasText = input.value.trim().length > 0;
    add.hidden = !hasText;
    paste.hidden = hasText;
  };
  input.addEventListener('input', sync);

  paste.addEventListener('click', async () => {
    try {
      const text = await navigator.clipboard.readText();
      if (text.trim()) {
        input.value = text.trim();
        sync();
      }
      input.focus();
    } catch {
      // Clipboard reads need a permission the extension doesn't ask for; fall back to the keyboard.
      input.focus();
      ctx.toast(tr('toastPasteHint', 'Press ⌘V / Ctrl+V to paste'));
    }
  });

  form.addEventListener('submit', async (event) => {
    event.preventDefault();
    const request = requestFor(input.value);
    if (!request) {
      ctx.toast(tr('toastInvalidLink', 'That doesn’t look like a download link.'), 'error');
      input.focus();
      return;
    }
    add.disabled = true;
    try {
      await quickDownload<AddTaskResult>(request);
      ctx.toast(tr('toastDownloadStarted', 'Download started'));
      input.value = '';
      sync();
    } catch (err) {
      ctx.toast(errorMessage(err), 'error');
    } finally {
      add.disabled = false;
    }
  });

  sync();
  return {
    setEnabled(enabled: boolean): void {
      input.disabled = !enabled;
      paste.disabled = !enabled;
      add.disabled = !enabled;
      form.classList.toggle('is-disabled', !enabled);
    },
  };
}

async function initCapture(): Promise<void> {
  const toggle = byId<HTMLInputElement>('capture-toggle');
  const sub = byId<HTMLElement>('capture-sub');
  const store = new SettingsStore(browser.storage.local);
  const paint = (on: boolean): void => {
    toggle.checked = on;
    sub.textContent = on
      ? tr('captureOnHint', 'Matching downloads go to Swoop')
      : tr('captureOffHint', 'Your browser keeps every download');
  };
  paint((await store.get()).intercept_downloads);
  toggle.addEventListener('change', () => {
    paint(toggle.checked);
    void store.update({ intercept_downloads: toggle.checked });
  });
  store.onChange((settings) => paint(settings.intercept_downloads));
}

async function main(): Promise<void> {
  applyI18n();
  hydrateIcons(document);
  byId('brand-mark').replaceWith(brandMark(26));
  document.title = tr('extensionName', 'Swoop');

  // Opened as a regular tab (context-menu fallback) rather than as the toolbar popup.
  if (window.innerWidth > 520) document.documentElement.classList.add('in-tab');

  const ctx: PopupContext = { tabId: null, connection: null, toast: showToast };

  const addLink = initAddLink(ctx);
  const downloads = initDownloads(ctx);
  const media = initMedia(ctx);
  const links = initLinks(ctx);
  void initCapture();

  let firstStatus = true;
  const status = initStatus(ctx, (state) => {
    const online = state === 'online';
    downloads.setUsable(online || state === 'checking');
    addLink.setEnabled(online || state === 'checking');
    if (online && firstStatus) void downloads.refresh();
    if (online) firstStatus = false;
  });

  byId('settings-btn').addEventListener('click', () => openOptions());
  byId('links-btn').addEventListener('click', () => void links.open());
  byId('open-app-btn').addEventListener('click', async () => {
    const connection: ConnectionStatus | null = ctx.connection;
    if (connection?.mode === 'remote') {
      const stored = await browser.storage.local.get('swoopSettings');
      const url = (stored['swoopSettings'] as { remote_url?: string } | undefined)?.remote_url;
      if (url) await browser.tabs.create({ url });
      return;
    }
    try {
      await launchApp();
      ctx.toast(tr('toastLaunching', 'Launching Swoop…'));
    } catch (err) {
      ctx.toast(errorMessage(err), 'error');
    }
  });

  // Live events. Status frames from the port are only trusted after our own first check, so a
  // cold background can't flash "Not running" before it has actually asked.
  let statusChecked = false;
  const port = browser.runtime.connect({ name: POPUP_PORT_NAME });
  port.onMessage.addListener((raw) => {
    const message = raw as PopupPortMessage;
    if (message.type === 'connection-status') {
      if (statusChecked) status.apply(message.status);
    } else if (message.type === 'event') {
      downloads.handleEvent(message.event as SwoopEvent);
    }
  });

  ctx.tabId = await getActiveTabId().catch(() => null);
  const [pending] = await Promise.all([
    takePendingBulkLinks().catch(() => []),
    status.refresh().then(() => {
      statusChecked = true;
    }),
    media.refresh(),
  ]);

  const wantsLinks = new URLSearchParams(location.search).get('tab') === 'links';
  if (pending.length > 0 || wantsLinks) void links.open(pending);
}

document.addEventListener('DOMContentLoaded', () => {
  void main();
});
