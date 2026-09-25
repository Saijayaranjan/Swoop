/**
 * Wire types for the extension <-> `swoop native-host` protocol
 * (docs/api/native-messaging.md). The host is a stateless relay onto the local REST API
 * (docs/api/rest.md) plus a push channel for WebSocket-shaped events (docs/api/websocket.md).
 */

export type HttpMethod = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';

export interface HostRequestMessage {
  id: number;
  type: 'request';
  method: HttpMethod;
  path: string;
  body?: unknown;
}

export interface HostSubscribeMessage {
  id: number;
  type: 'subscribe';
  events: string[];
  tasks?: string[];
}

export interface HostUnsubscribeMessage {
  id: number;
  type: 'unsubscribe';
}

export interface HostPingMessage {
  id: number;
  type: 'ping';
}

export interface HostLaunchMessage {
  id: number;
  type: 'launch';
}

export type HostOutgoingMessage =
  | HostRequestMessage
  | HostSubscribeMessage
  | HostUnsubscribeMessage
  | HostPingMessage
  | HostLaunchMessage;

export interface HostErrorBody {
  type: string;
  message: string;
}

export interface HostResponseOk {
  id: number;
  ok: true;
  status: number;
  body: unknown;
}

export interface HostResponseErr {
  id: number;
  ok: false;
  error: HostErrorBody;
  /** Some hosts also echo `status` on errors; treat as optional. */
  status?: number;
}

export type HostResponseMessage = HostResponseOk | HostResponseErr;

export interface HostPongMessage {
  type: 'pong';
  version: string;
  running: boolean;
}

export interface HostEventMessage {
  type: 'event';
  event: { type: string; data: unknown };
}

export type HostIncomingMessage = HostResponseMessage | HostPongMessage | HostEventMessage;

export function isHostResponse(msg: unknown): msg is HostResponseMessage {
  return (
    typeof msg === 'object' &&
    msg !== null &&
    'id' in msg &&
    'ok' in msg &&
    typeof (msg as { id: unknown }).id === 'number'
  );
}

export function isHostPong(msg: unknown): msg is HostPongMessage {
  return (
    typeof msg === 'object' &&
    msg !== null &&
    (msg as { type?: unknown }).type === 'pong'
  );
}

export function isHostEvent(msg: unknown): msg is HostEventMessage {
  return (
    typeof msg === 'object' &&
    msg !== null &&
    (msg as { type?: unknown }).type === 'event'
  );
}

/** The native host's own "unavailable" error type (app not running). */
export const HOST_UNAVAILABLE = 'unavailable';
