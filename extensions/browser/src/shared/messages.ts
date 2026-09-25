/**
 * Typed message contracts for `runtime.sendMessage` / `tabs.sendMessage` traffic between the
 * popup, options page, content scripts and the background service worker. Keeping these in one
 * place means every call site is a discriminated-union switch instead of stringly-typed guesses.
 */

import type { HttpMethod } from './native-protocol.ts';
import type { DetectedMedia, ExtensionSettings, NewTaskRequest } from './types.ts';

// ---------------------------------------------------------------------------------------------
// content script <-> background/popup
// ---------------------------------------------------------------------------------------------

export interface ScanRequestMessage {
  type: 'scan' | 'scan-selection';
}

export interface CandidateLink {
  url: string;
  text: string;
  looksDownloadable: boolean;
}

export interface CandidatePageMedia {
  url: string;
  kind: 'video' | 'audio' | 'image' | 'hls_playlist' | 'dash_manifest';
  title: string | null;
  notDownloadable: boolean;
  poster?: string | null;
  duration?: number | null;
  width?: number | null;
  height?: number | null;
}

export interface ScanResultMessage {
  type: 'scan-result';
  pageUrl: string;
  pageTitle: string;
  links: CandidateLink[];
  media: CandidatePageMedia[];
}

// ---------------------------------------------------------------------------------------------
// popup/options -> background
// ---------------------------------------------------------------------------------------------

export interface ApiRequestMessage {
  type: 'api-request';
  method: HttpMethod;
  path: string;
  body?: unknown;
}

export interface GetDetectedMediaMessage {
  type: 'get-detected-media';
  tabId: number;
}

export interface ClearDetectedMediaMessage {
  type: 'clear-detected-media';
  tabId: number;
}

export interface EnrichHlsMessage {
  type: 'enrich-hls';
  tabId: number;
  pageUrl: string | null;
}

export interface GetConnectionStatusMessage {
  type: 'get-connection-status';
}

export interface LaunchAppMessage {
  type: 'launch-app';
}

export interface QuickDownloadMessage {
  type: 'quick-download';
  request: NewTaskRequest;
}

export interface GetActiveTabIdMessage {
  type: 'get-active-tab-id';
}

export interface TakePendingBulkLinksMessage {
  type: 'take-pending-bulk-links';
}

/** Options page: complete pairing with a remote Swoop using a one-time code. */
export interface PairRemoteMessage {
  type: 'pair-remote';
  url: string;
  code: string;
}

/** Options page: store a device token pasted by hand (instead of pairing with a code). */
export interface SetRemoteTokenMessage {
  type: 'set-remote-token';
  url: string;
  token: string;
}

/** Options page: forget the remote Swoop and return to the local native host. */
export interface ForgetRemoteMessage {
  type: 'forget-remote';
}

/** Content script: which in-page UI it may show. */
export interface GetPageUiConfigMessage {
  type: 'get-page-ui-config';
}

/** Content script: network-detected media for the sender's own tab. */
export interface GetPageMediaMessage {
  type: 'get-page-media';
}

/** Content script: a button pressed on an in-page download prompt. */
export interface PromptActionMessage {
  type: 'prompt-action';
  promptId: string;
  action: 'use-browser' | 'always-browser';
}

export type BackgroundRequestMessage =
  | ApiRequestMessage
  | GetDetectedMediaMessage
  | ClearDetectedMediaMessage
  | EnrichHlsMessage
  | GetConnectionStatusMessage
  | LaunchAppMessage
  | QuickDownloadMessage
  | GetActiveTabIdMessage
  | TakePendingBulkLinksMessage
  | PairRemoteMessage
  | SetRemoteTokenMessage
  | ForgetRemoteMessage
  | GetPageUiConfigMessage
  | GetPageMediaMessage
  | PromptActionMessage;

/** The only request types a content script (an untrusted page context) may send. Everything
 *  else is accepted from the extension's own pages only (src/background/message-router.ts). */
export const CONTENT_SCRIPT_MESSAGE_TYPES: ReadonlySet<BackgroundRequestMessage['type']> = new Set([
  'get-page-ui-config',
  'get-page-media',
  'quick-download',
  'launch-app',
  'prompt-action',
]);

export interface BackgroundOkResponse<T = unknown> {
  ok: true;
  data: T;
}

export interface BackgroundErrResponse {
  ok: false;
  error: string;
  errorKind?: string;
}

export type BackgroundResponse<T = unknown> = BackgroundOkResponse<T> | BackgroundErrResponse;

export interface ConnectionStatus {
  connected: boolean;
  running: boolean;
  version: string | null;
  /** Which transport answered; absent from older backgrounds, treat as `native`. */
  mode?: 'native' | 'remote';
}

export interface PageUiConfig {
  detectMedia: boolean;
  mediaButton: boolean;
}

export interface PairRemoteResult {
  deviceName: string | null;
}

// ---------------------------------------------------------------------------------------------
// background -> content script (in-page prompt after an intercepted download)
// ---------------------------------------------------------------------------------------------

export type PagePromptVariant = 'sent' | 'not-running' | 'failed';

export interface PagePrompt {
  promptId: string;
  variant: PagePromptVariant;
  fileName: string;
  host: string | null;
  size: number | null;
  /** The local app can be launched from the prompt (native connection only). */
  canLaunch: boolean;
}

export interface ShowPagePromptMessage {
  type: 'swoop-page-prompt';
  prompt: PagePrompt;
}

export interface DetectedMediaForTab {
  tabId: number;
  items: DetectedMedia[];
}

// ---------------------------------------------------------------------------------------------
// long-lived popup <-> background port ("popup")
// ---------------------------------------------------------------------------------------------

export const POPUP_PORT_NAME = 'swoop-popup';

export interface PortEventFrame {
  type: 'event';
  event: { type: string; data: unknown };
}

export interface PortStatusFrame {
  type: 'connection-status';
  status: ConnectionStatus;
}

export type PopupPortMessage = PortEventFrame | PortStatusFrame;

export function isBackgroundResponseOk<T>(
  response: BackgroundResponse<T>,
): response is BackgroundOkResponse<T> {
  return response.ok;
}

export type { ExtensionSettings };
