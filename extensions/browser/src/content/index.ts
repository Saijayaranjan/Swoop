/**
 * Content script (docs/api/extension.md "Media detection"): answers on-demand page scans from the
 * popup and context menus (src/content/scan.ts), and hosts Swoop's small in-page UI — the
 * prompt shown after a download is captured, and the media button over video/audio players
 * (src/content/page-ui.ts). The UI lives in a closed shadow root so page CSS can't reach it.
 * Nothing is sent to the background until a scan is requested or a player is interacted with.
 */

import browser from 'webextension-polyfill';
import type { ShowPagePromptMessage } from '../shared/messages.ts';
import { buildScanResult } from './scan.ts';
import { initMediaButton, showPrompt } from './page-ui.ts';

browser.runtime.onMessage.addListener((message: unknown) => {
  if (typeof message !== 'object' || message === null || !('type' in message)) return undefined;
  const type = (message as { type: unknown }).type;
  if (type === 'scan' || type === 'scan-selection') {
    return Promise.resolve(buildScanResult(type));
  }
  if (type === 'swoop-page-prompt' && window === window.top) {
    showPrompt((message as ShowPagePromptMessage).prompt);
    return Promise.resolve(true);
  }
  return undefined;
});

if (window === window.top) {
  initMediaButton();
}
