/**
 * Handles `runtime.sendMessage` traffic from the popup/options page (src/shared/messages.ts).
 * Every branch calls through to the real native port / media detector / settings store — there is
 * no message type that is accepted but silently ignored.
 */

import browser from 'webextension-polyfill';
import { ApiError } from '../shared/api-client.ts';
import type { NativePort } from './native-port.ts';
import type { MediaDetector } from '../media-detector.ts';
import type { SettingsStore } from '../shared/settings.ts';
import { takePendingBulkLinks } from './state-bulk.ts';
import type {
  BackgroundRequestMessage,
  BackgroundResponse,
  ConnectionStatus,
  DetectedMediaForTab,
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
]);

function isBackgroundRequest(message: unknown): message is BackgroundRequestMessage {
  if (typeof message !== 'object' || message === null || !('type' in message)) return false;
  const type = (message as { type: unknown }).type;
  return typeof type === 'string' && KNOWN_TYPES.has(type as BackgroundRequestMessage['type']);
}

export interface RouterDeps {
  nativePort: NativePort;
  mediaDetector: MediaDetector;
  settingsStore: SettingsStore;
}

export function registerMessageRouter(deps: RouterDeps): void {
  browser.runtime.onMessage.addListener((message: unknown) => {
    if (!isBackgroundRequest(message)) return undefined;
    return handle(message, deps);
  });
}

async function handle(
  message: BackgroundRequestMessage,
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
        const response: ConnectionStatus = {
          connected: status.connected,
          running: status.running === true,
          version: status.version,
        };
        return { ok: true, data: response };
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
