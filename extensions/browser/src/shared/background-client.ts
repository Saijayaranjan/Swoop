/**
 * Typed wrapper over `runtime.sendMessage` for the popup/options page to reach the background
 * (src/background/message-router.ts). Every exported function corresponds 1:1 to a
 * `BackgroundRequestMessage` variant — there is no ad hoc message shape elsewhere in the UI code.
 */

import browser from 'webextension-polyfill';
import type { HttpMethod } from './native-protocol.ts';
import type {
  BackgroundRequestMessage,
  BackgroundResponse,
  CandidateLink,
  ConnectionStatus,
  DetectedMediaForTab,
} from './messages.ts';
import type { NewTaskRequest } from './types.ts';

async function send<T>(message: BackgroundRequestMessage): Promise<T> {
  const response = (await browser.runtime.sendMessage(message)) as BackgroundResponse<T> | undefined;
  if (!response) {
    throw new Error('No response from background — try reopening the popup.');
  }
  if (!response.ok) {
    throw new Error(response.error);
  }
  return response.data;
}

export function apiRequest<T>(method: HttpMethod, path: string, body?: unknown): Promise<T> {
  return send<T>({ type: 'api-request', method, path, body });
}

export function getDetectedMedia(tabId: number): Promise<DetectedMediaForTab> {
  return send<DetectedMediaForTab>({ type: 'get-detected-media', tabId });
}

export function clearDetectedMedia(tabId: number): Promise<null> {
  return send<null>({ type: 'clear-detected-media', tabId });
}

export function enrichHls(tabId: number, pageUrl: string | null): Promise<null> {
  return send<null>({ type: 'enrich-hls', tabId, pageUrl });
}

export function getConnectionStatus(): Promise<ConnectionStatus> {
  return send<ConnectionStatus>({ type: 'get-connection-status' });
}

export function launchApp(): Promise<null> {
  return send<null>({ type: 'launch-app' });
}

export function quickDownload<T>(request: NewTaskRequest): Promise<T> {
  return send<T>({ type: 'quick-download', request });
}

export function getActiveTabId(): Promise<number | null> {
  return send<number | null>({ type: 'get-active-tab-id' });
}

export function takePendingBulkLinks(): Promise<CandidateLink[]> {
  return send<CandidateLink[]>({ type: 'take-pending-bulk-links' });
}
