/**
 * Client for a paired remote Osprey (docs/api/rest.md, docs/api/websocket.md): REST over HTTPS
 * with a device bearer token, and the WebSocket event stream for live progress. Used by
 * `native-port.ts` in place of the native-messaging relay when the user picks "Remote Osprey" in
 * the options page. The server accepts the extension's own origin (`chrome-extension://…`,
 * `moz-extension://…`), so no CORS configuration is needed on the Osprey side.
 */

import { ApiError } from '../shared/api-client.ts';
import type { HttpMethod } from '../shared/native-protocol.ts';
import { remoteEventsUrl } from '../shared/remote.ts';
import type { EngineInfo } from '../shared/types.ts';

const REQUEST_TIMEOUT_MS = 10_000;
const MAX_BACKOFF_MS = 30_000;

type EventListener = (event: { type: string; data: unknown }) => void;

async function readJson(response: Response): Promise<unknown> {
  const text = await response.text();
  if (text.length === 0) return null;
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return text;
  }
}

function errorFrom(status: number, body: unknown): ApiError {
  const err = (body as { error?: { type?: unknown; message?: unknown } } | null)?.error;
  const kind = typeof err?.type === 'string' ? err.type : 'http';
  const message = typeof err?.message === 'string' ? err.message : `HTTP ${status}`;
  return new ApiError(kind, message, status);
}

export async function remoteFetch<T>(
  baseUrl: string,
  token: string | null,
  method: HttpMethod,
  path: string,
  body?: unknown,
): Promise<T> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
  const headers: Record<string, string> = { Accept: 'application/json' };
  if (token) headers['Authorization'] = `Bearer ${token}`;
  if (body !== undefined) headers['Content-Type'] = 'application/json';
  let response: Response;
  try {
    response = await fetch(`${baseUrl}${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: controller.signal,
      credentials: 'omit',
      cache: 'no-store',
      redirect: 'error',
    });
  } catch {
    throw new ApiError('unreachable', `Could not reach Osprey at ${new URL(baseUrl).host}`);
  } finally {
    clearTimeout(timer);
  }
  const data = await readJson(response);
  if (!response.ok) throw errorFrom(response.status, data);
  return data as T;
}

export class RemoteApiClient {
  readonly baseUrl: string;
  private token: string;
  private socket: WebSocket | null = null;
  private events: string[] = [];
  private listeners = new Set<EventListener>();
  private backoffMs = 1_000;
  private reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  private closed = false;

  constructor(baseUrl: string, token: string) {
    this.baseUrl = baseUrl;
    this.token = token;
  }

  request<T = unknown>(method: HttpMethod, path: string, body?: unknown): Promise<T> {
    return remoteFetch<T>(this.baseUrl, this.token, method, path, body);
  }

  info(): Promise<EngineInfo> {
    return this.request<EngineInfo>('GET', '/api/v1/info');
  }

  onEvent(listener: EventListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Opens (or keeps) the event stream and forwards only the requested event types. */
  subscribe(events: string[]): void {
    this.events = events;
    if (this.socket || this.closed) return;
    this.open();
  }

  private open(): void {
    let socket: WebSocket;
    try {
      socket = new WebSocket(remoteEventsUrl(this.baseUrl, this.token));
    } catch {
      this.scheduleReconnect();
      return;
    }
    this.socket = socket;
    socket.addEventListener('open', () => {
      this.backoffMs = 1_000;
    });
    socket.addEventListener('message', (message) => {
      if (typeof message.data !== 'string') return;
      let frame: { type?: unknown; data?: unknown };
      try {
        frame = JSON.parse(message.data) as { type?: unknown; data?: unknown };
      } catch {
        return;
      }
      if (typeof frame.type !== 'string' || !this.events.includes(frame.type)) return;
      const event = { type: frame.type, data: frame.data };
      for (const listener of this.listeners) listener(event);
    });
    socket.addEventListener('close', () => {
      if (this.socket === socket) this.socket = null;
      this.scheduleReconnect();
    });
  }

  private scheduleReconnect(): void {
    if (this.closed || this.events.length === 0 || this.reconnectTimer !== undefined) return;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = undefined;
      if (!this.closed && !this.socket) this.open();
    }, this.backoffMs);
    this.backoffMs = Math.min(this.backoffMs * 2, MAX_BACKOFF_MS);
  }

  close(): void {
    this.closed = true;
    if (this.reconnectTimer !== undefined) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = undefined;
    this.socket?.close();
    this.socket = null;
    this.listeners.clear();
  }
}

export interface PairResponse {
  device?: { name?: string } | null;
  token: string;
}

/** `POST /api/v1/pair` on the remote listener; returns the device token (shown only once). */
export function pairWithRemote(baseUrl: string, code: string, deviceName: string): Promise<PairResponse> {
  return remoteFetch<PairResponse>(baseUrl, null, 'POST', '/api/v1/pair', {
    code,
    device_name: deviceName,
    device_kind: 'browser',
  });
}
