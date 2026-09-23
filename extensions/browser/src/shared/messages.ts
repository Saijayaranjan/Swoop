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

export type BackgroundRequestMessage =
  | ApiRequestMessage
  | GetDetectedMediaMessage
  | ClearDetectedMediaMessage
  | EnrichHlsMessage
  | GetConnectionStatusMessage
  | LaunchAppMessage
  | QuickDownloadMessage
  | GetActiveTabIdMessage
  | TakePendingBulkLinksMessage;

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
}

export interface DetectedMediaForTab {
  tabId: number;
  items: DetectedMedia[];
}

// ---------------------------------------------------------------------------------------------
// long-lived popup <-> background port ("popup")
// ---------------------------------------------------------------------------------------------

export const POPUP_PORT_NAME = 'osprey-popup';

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
