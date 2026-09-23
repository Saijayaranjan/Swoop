/**
 * Transport-agnostic client for the native-messaging protocol: request/response correlation by
 * `id`, per-request timeouts, `subscribe`/`ping`/`launch` helpers, and event fan-out.
 *
 * Deliberately knows nothing about `browser.runtime.connectNative` — it only needs an object with
 * a `postMessage`, which makes it trivial to unit test with a fake transport (see
 * test/api-client.test.ts) and lets `src/background/native-port.ts` layer reconnect/backoff on
 * top without duplicating the correlation logic.
 */

import type {
  HostErrorBody,
  HostOutgoingMessage,
  HttpMethod,
} from './native-protocol.ts';
import { isHostEvent, isHostPong, isHostResponse } from './native-protocol.ts';

export interface Transport {
  postMessage(message: HostOutgoingMessage): void;
}

export class ApiError extends Error {
  readonly kind: string;
  readonly status: number | undefined;

  constructor(kind: string, message: string, status?: number) {
    super(message);
    this.name = 'ApiError';
    this.kind = kind;
    this.status = status;
  }
}

export class TimeoutError extends Error {
  constructor(message = 'Request timed out') {
    super(message);
    this.name = 'TimeoutError';
  }
}

interface PendingEntry {
  resolve(value: unknown): void;
  reject(reason: unknown): void;
  timer: ReturnType<typeof setTimeout> | undefined;
}

export const DEFAULT_TIMEOUT_MS = 10_000;

export class NativeApiClient {
  private transport: Transport;
  private nextId = 1;
  private pending = new Map<number, PendingEntry>();
  private eventListeners = new Set<(event: { type: string; data: unknown }) => void>();
  private defaultTimeoutMs: number;

  constructor(transport: Transport, options: { defaultTimeoutMs?: number } = {}) {
    this.transport = transport;
    this.defaultTimeoutMs = options.defaultTimeoutMs ?? DEFAULT_TIMEOUT_MS;
  }

  /** Feed a message received from the real transport (a Port's `onMessage`, or a fake in tests). */
  handleMessage(raw: unknown): void {
    if (isHostEvent(raw)) {
      for (const listener of this.eventListeners) listener(raw.event);
      return;
    }
    if (isHostPong(raw)) {
      // Pongs are also correlated through `ping()`'s pending entry via a synthetic id of 0 is
      // not used; instead `ping()` resolves by listening once. See `ping()` below.
      for (const listener of this.pongListeners) listener(raw);
      return;
    }
    if (isHostResponse(raw)) {
      const entry = this.pending.get(raw.id);
      if (!entry) return; // stale / already timed out
      this.pending.delete(raw.id);
      if (entry.timer !== undefined) clearTimeout(entry.timer);
      if (raw.ok) {
        entry.resolve(raw.body);
      } else {
        const err: HostErrorBody = raw.error;
        entry.reject(new ApiError(err.type, err.message, raw.status));
      }
    }
  }

  /** Register a listener for pushed `{type:"event", event:{...}}` frames. Returns an unsubscribe fn. */
  onEvent(listener: (event: { type: string; data: unknown }) => void): () => void {
    this.eventListeners.add(listener);
    return () => this.eventListeners.delete(listener);
  }

  private pongListeners = new Set<(pong: { version: string; running: boolean }) => void>();

  /** Called by the owner (native-port.ts) when the underlying transport disconnects, so every
   * in-flight request fails fast instead of hanging until its timeout. */
  rejectAllPending(reason: unknown): void {
    for (const [id, entry] of this.pending) {
      if (entry.timer !== undefined) clearTimeout(entry.timer);
      entry.reject(reason);
      this.pending.delete(id);
    }
  }

  private send<T>(
    build: (id: number) => HostOutgoingMessage,
    timeoutMs: number,
  ): Promise<T> {
    const id = this.nextId++;
    return new Promise<T>((resolve, reject) => {
      const timer =
        timeoutMs > 0
          ? setTimeout(() => {
              this.pending.delete(id);
              reject(new TimeoutError());
            }, timeoutMs)
          : undefined;
      this.pending.set(id, {
        resolve: resolve as (value: unknown) => void,
        reject,
        timer,
      });
      try {
        this.transport.postMessage(build(id));
      } catch (err) {
        this.pending.delete(id);
        if (timer !== undefined) clearTimeout(timer);
        reject(err instanceof Error ? err : new Error(String(err)));
      }
    });
  }

  request<T = unknown>(
    method: HttpMethod,
    path: string,
    body?: unknown,
    timeoutMs = this.defaultTimeoutMs,
  ): Promise<T> {
    return this.send<T>((id) => ({ id, type: 'request', method, path, body }), timeoutMs);
  }

  get<T = unknown>(path: string, timeoutMs?: number): Promise<T> {
    return this.request<T>('GET', path, undefined, timeoutMs);
  }

  post<T = unknown>(path: string, body?: unknown, timeoutMs?: number): Promise<T> {
    return this.request<T>('POST', path, body, timeoutMs);
  }

  put<T = unknown>(path: string, body?: unknown, timeoutMs?: number): Promise<T> {
    return this.request<T>('PUT', path, body, timeoutMs);
  }

  patch<T = unknown>(path: string, body?: unknown, timeoutMs?: number): Promise<T> {
    return this.request<T>('PATCH', path, body, timeoutMs);
  }

  delete<T = unknown>(path: string, timeoutMs?: number): Promise<T> {
    return this.request<T>('DELETE', path, undefined, timeoutMs);
  }

  subscribe(events: string[], tasks?: string[], timeoutMs?: number): Promise<unknown> {
    return this.send(
      (id) => ({ id, type: 'subscribe', events, ...(tasks ? { tasks } : {}) }),
      timeoutMs ?? this.defaultTimeoutMs,
    );
  }

  unsubscribe(timeoutMs?: number): Promise<unknown> {
    return this.send((id) => ({ id, type: 'unsubscribe' }), timeoutMs ?? this.defaultTimeoutMs);
  }

  /** `ping` replies with a un-correlated `{type:"pong",...}` (no echoed id), so it is handled
   * out-of-band from `pending` via a one-shot pong listener race against a timeout. */
  ping(timeoutMs = 2_000): Promise<{ version: string; running: boolean }> {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      let settled = false;
      const timer = setTimeout(() => {
        if (settled) return;
        settled = true;
        this.pongListeners.delete(onPong);
        reject(new TimeoutError('ping timed out'));
      }, timeoutMs);
      const onPong = (pong: { version: string; running: boolean }): void => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        this.pongListeners.delete(onPong);
        resolve(pong);
      };
      this.pongListeners.add(onPong);
      try {
        this.transport.postMessage({ id, type: 'ping' });
      } catch (err) {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        this.pongListeners.delete(onPong);
        reject(err instanceof Error ? err : new Error(String(err)));
      }
    });
  }

  /** Ask the host to launch the desktop app (`open -b app.osprey.desktop`, macOS-only host side). */
  launch(): void {
    const id = this.nextId++;
    this.transport.postMessage({ id, type: 'launch' });
  }
}
