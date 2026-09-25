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
  PageUiConfig,
  PairRemoteResult,
} from './messages.ts';
import type { NewTaskRequest } from './types.ts';

/** An error returned by the background, keeping the API/host error kind when there is one. */
export class BackgroundError extends Error {
  readonly kind: string | undefined;
  constructor(message: string, kind?: string) {
    super(message);
    this.name = 'BackgroundError';
    this.kind = kind;
  }
}

async function send<T>(message: BackgroundRequestMessage): Promise<T> {
  const response = (await browser.runtime.sendMessage(message)) as BackgroundResponse<T> | undefined;
  if (!response) {
    throw new Error('No response from background — try reopening the popup.');
  }
  if (!response.ok) {
    throw new BackgroundError(response.error, response.errorKind);
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

export function pairRemote(url: string, code: string): Promise<PairRemoteResult> {
  return send<PairRemoteResult>({ type: 'pair-remote', url, code });
}

export function setRemoteToken(url: string, token: string): Promise<null> {
  return send<null>({ type: 'set-remote-token', url, token });
}

export function forgetRemote(): Promise<null> {
  return send<null>({ type: 'forget-remote' });
}

export function getPageUiConfig(): Promise<PageUiConfig> {
  return send<PageUiConfig>({ type: 'get-page-ui-config' });
}

export function getPageMedia(): Promise<DetectedMediaForTab> {
  return send<DetectedMediaForTab>({ type: 'get-page-media' });
}

export function promptAction(promptId: string, action: 'use-browser' | 'always-browser'): Promise<null> {
  return send<null>({ type: 'prompt-action', promptId, action });
}
