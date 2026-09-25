/**
 * Handles `runtime.sendMessage` traffic from the popup/options page and the content script
 * (src/shared/messages.ts). Every branch calls through to the real native port / media detector /
 * settings store — there is no message type that is accepted but silently ignored.
 *
 * Content scripts run inside arbitrary web pages, so they may only send the small set of request
 * types in `CONTENT_SCRIPT_MESSAGE_TYPES`; everything else (raw API requests, pairing, …) is
 * accepted from the extension's own pages only.
 */

import browser from 'webextension-polyfill';
import { ApiError } from '../shared/api-client.ts';
import type { NativePort } from './native-port.ts';
import type { MediaDetector } from '../media-detector.ts';
import type { SettingsStore } from '../shared/settings.ts';
import { takePendingBulkLinks } from './state-bulk.ts';
import { handlePromptAction } from './download-interception.ts';
import { pairWithRemote } from './remote-client.ts';
import { REMOTE_TOKEN_KEY, normalizePairingCode, normalizeRemoteUrl } from '../shared/remote.ts';
import {
  CONTENT_SCRIPT_MESSAGE_TYPES,
  type BackgroundRequestMessage,
  type BackgroundResponse,
  type ConnectionStatus,
  type DetectedMediaForTab,
  type PageUiConfig,
  type PairRemoteResult,
} from '../shared/messages.ts';

const KNOWN_TYPES = new Set<BackgroundRequestMessage['type']>([
  'api-request',
  'get-detected-media',
  'clear-detected-media',
  'enrich-hls',
  'get-connection-status',
  'launch-app',
  'quick-download',
  'get-active-tab-id',
  'take-pending-bulk-links',
  'pair-remote',
  'set-remote-token',
  'forget-remote',
  'get-page-ui-config',
  'get-page-media',
  'prompt-action',
]);

function isBackgroundRequest(message: unknown): message is BackgroundRequestMessage {
  if (typeof message !== 'object' || message === null || !('type' in message)) return false;
  const type = (message as { type: unknown }).type;
  return typeof type === 'string' && KNOWN_TYPES.has(type as BackgroundRequestMessage['type']);
}

/** True for the popup/options page (and the popup opened as a tab); false for content scripts. */
function isExtensionPage(sender: browser.Runtime.MessageSender): boolean {
  const origin = browser.runtime.getURL('');
  return typeof sender.url === 'string' && sender.url.startsWith(origin);
}

export interface RouterDeps {
  nativePort: NativePort;
  mediaDetector: MediaDetector;
  settingsStore: SettingsStore;
}

export function registerMessageRouter(deps: RouterDeps): void {
  browser.runtime.onMessage.addListener((message: unknown, sender: browser.Runtime.MessageSender) => {
    if (!isBackgroundRequest(message)) return undefined;
    if (sender.id !== undefined && sender.id !== browser.runtime.id) return undefined;
    if (!isExtensionPage(sender) && !CONTENT_SCRIPT_MESSAGE_TYPES.has(message.type)) {
      return Promise.resolve<BackgroundResponse>({ ok: false, error: 'not allowed from a web page' });
    }
    return handle(message, sender, deps);
  });
}

function toConnectionStatus(status: Awaited<ReturnType<NativePort['ping']>>): ConnectionStatus {
  return {
    connected: status.connected,
    running: status.running === true,
    version: status.version,
    mode: status.mode,
  };
}

function deviceName(): string {
  const target = typeof __SWOOP_TARGET__ === 'string' ? __SWOOP_TARGET__ : 'browser';
  const label = target.charAt(0).toUpperCase() + target.slice(1);
  return `${label} extension`;
}

async function handle(
  message: BackgroundRequestMessage,
  sender: browser.Runtime.MessageSender,
  deps: RouterDeps,
): Promise<BackgroundResponse> {
  try {
    switch (message.type) {
      case 'api-request': {
        const data = await deps.nativePort.request(message.method, message.path, message.body);
        return { ok: true, data };
      }
      case 'get-detected-media': {
        const result: DetectedMediaForTab = {
          tabId: message.tabId,
          items: deps.mediaDetector.getForTab(message.tabId),
        };
        return { ok: true, data: result };
      }
      case 'clear-detected-media':
        deps.mediaDetector.clearTab(message.tabId);
        return { ok: true, data: null };
      case 'enrich-hls':
        await deps.mediaDetector.enrichHlsForTab(message.tabId, message.pageUrl, deps.nativePort);
        return { ok: true, data: null };
      case 'get-connection-status': {
        const status = await deps.nativePort.ping();
        return { ok: true, data: toConnectionStatus(status) };
      }
      case 'launch-app':
        await deps.nativePort.launch();
        return { ok: true, data: null };
      case 'quick-download': {
        const data = await deps.nativePort.post('/api/v1/tasks', message.request);
        return { ok: true, data };
      }
      case 'get-active-tab-id': {
        const [tab] = await browser.tabs.query({ active: true, currentWindow: true });
        return { ok: true, data: tab?.id ?? null };
      }
      case 'take-pending-bulk-links': {
        const links = await takePendingBulkLinks();
        return { ok: true, data: links };
      }
      case 'pair-remote': {
        const baseUrl = normalizeRemoteUrl(message.url);
        if (!baseUrl) return { ok: false, error: 'invalid_url', errorKind: 'invalid_url' };
        const code = normalizePairingCode(message.code);
        if (!code) return { ok: false, error: 'invalid_code', errorKind: 'invalid_code' };
        const paired = await pairWithRemote(baseUrl, code, deviceName());
        await browser.storage.local.set({ [REMOTE_TOKEN_KEY]: paired.token });
        await deps.settingsStore.update({ connection_mode: 'remote', remote_url: baseUrl });
        const result: PairRemoteResult = { deviceName: paired.device?.name ?? null };
        return { ok: true, data: result };
      }
      case 'set-remote-token': {
        const baseUrl = normalizeRemoteUrl(message.url);
        if (!baseUrl) return { ok: false, error: 'invalid_url', errorKind: 'invalid_url' };
        const token = message.token.trim();
        if (token.length < 16 || /\s/.test(token)) {
          return { ok: false, error: 'invalid_token', errorKind: 'invalid_token' };
        }
        await browser.storage.local.set({ [REMOTE_TOKEN_KEY]: token });
        await deps.settingsStore.update({ connection_mode: 'remote', remote_url: baseUrl });
        return { ok: true, data: null };
      }
      case 'forget-remote':
        await browser.storage.local.remove(REMOTE_TOKEN_KEY);
        await deps.settingsStore.update({ connection_mode: 'native' });
        return { ok: true, data: null };
      case 'get-page-ui-config': {
        const settings = await deps.settingsStore.get();
        const config: PageUiConfig = {
          detectMedia: settings.detect_media,
          mediaButton: settings.detect_media && settings.media_button,
        };
        return { ok: true, data: config };
      }
      case 'get-page-media': {
        const tabId = sender.tab?.id;
        const result: DetectedMediaForTab = {
          tabId: tabId ?? -1,
          items: tabId === undefined ? [] : deps.mediaDetector.getForTab(tabId),
        };
        return { ok: true, data: result };
      }
      case 'prompt-action':
        await handlePromptAction(message.promptId, message.action, deps);
        return { ok: true, data: null };
      default: {
        const exhaustive: never = message;
        return { ok: false, error: `unknown message type: ${JSON.stringify(exhaustive)}` };
      }
    }
  } catch (err) {
    const errorKind = err instanceof ApiError ? err.kind : undefined;
    const errorMessage = err instanceof Error ? err.message : String(err);
    return errorKind !== undefined
      ? { ok: false, error: errorMessage, errorKind }
      : { ok: false, error: errorMessage };
  }
}
