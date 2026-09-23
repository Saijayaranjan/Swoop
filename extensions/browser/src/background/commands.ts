/**
 * `Alt+Shift+D` ("send-page-url" in manifests/base.json `commands`) — sends the current tab's URL
 * to Osprey as a new task, the keyboard equivalent of the "Download with Osprey" page context
 * menu entry (docs/api/extension.md "keyboard shortcut `Alt+Shift+D` sends the current page URL").
 */

import browser from 'webextension-polyfill';
import type { NativePort } from './native-port.ts';
import { showBadgeHint } from './badge-hint.ts';

const COMMAND_ID = 'send-page-url';

export function registerCommands(nativePort: NativePort): void {
  browser.commands.onCommand.addListener((command) => {
    if (command !== COMMAND_ID) return;
    void sendActiveTabUrl(nativePort);
  });
}

async function sendActiveTabUrl(nativePort: NativePort): Promise<void> {
  const [tab] = await browser.tabs.query({ active: true, currentWindow: true });
  if (!tab?.url) return;
  try {
    await nativePort.post('/api/v1/tasks', {
      url: tab.url,
      name: tab.title || undefined,
      start: true,
      origin: 'browser',
      referer_page: tab.url,
    });
  } catch {
    await showBadgeHint('!', '#dc2626');
  }
}
