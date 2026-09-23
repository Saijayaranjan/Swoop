/**
 * Owns the single `chrome.runtime.connectNative("app.osprey.bridge")` port
 * (docs/api/native-messaging.md), reconnecting it lazily with exponential backoff and replaying
 * the last `subscribe` request whenever a new connection is established. A live port is also what
 * keeps an MV3 service worker alive between browser-initiated wakeups, so `index.ts` opens one
 * eagerly at startup — but every other module reaches the API only through `ensureConnected()`,
 * so a suspended worker that has lost its port re-establishes it lazily on the next call rather
 * than assuming one already exists.
 */

import browser from 'webextension-polyfill';
import { NativeApiClient } from '../shared/api-client.ts';
import type { HttpMethod } from '../shared/native-protocol.ts';

export const NATIVE_HOST_ID = 'app.osprey.bridge';

const INITIAL_BACKOFF_MS = 1_000;
const MAX_BACKOFF_MS = 30_000;

export interface NativeStatus {
  /** The native-messaging port itself is open (host binary reachable). */
  connected: boolean;
  /** The Osprey desktop app / daemon answered the last ping as running. `null` = unknown yet. */
  running: boolean | null;
  version: string | null;
}

// webextension-polyfill types Port/connectNative loosely; narrow just what we use.
interface NativePortLike {
  postMessage(message: unknown): void;
  onMessage: { addListener(cb: (msg: unknown) => void): void };
  onDisconnect: { addListener(cb: () => void): void };
  disconnect(): void;
  error?: { message?: string } | null;
}

type ConnectNativeFn = (application: string) => NativePortLike;

export class NativePort {
  private connectNative: ConnectNativeFn;
  private port: NativePortLike | null = null;
  private client: NativeApiClient | null = null;
  private connecting: Promise<NativeApiClient> | null = null;
  private backoffMs = INITIAL_BACKOFF_MS;
  private reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  private subscribedEvents: string[] = [];
  private subscribedTasks: string[] | undefined;
  private eventListeners = new Set<(event: { type: string; data: unknown }) => void>();
  private statusListeners = new Set<(status: NativeStatus) => void>();
  private lastVersion: string | null = null;
  private lastRunning: boolean | null = null;

  constructor(connectNative: ConnectNativeFn = defaultConnectNative) {
    this.connectNative = connectNative;
  }

  isConnected(): boolean {
    return this.client !== null;
  }

  getStatus(): NativeStatus {
    return { connected: this.client !== null, running: this.lastRunning, version: this.lastVersion };
  }

  onEvent(listener: (event: { type: string; data: unknown }) => void): () => void {
    this.eventListeners.add(listener);
    return () => this.eventListeners.delete(listener);
  }

  onStatusChange(listener: (status: NativeStatus) => void): () => void {
    this.statusListeners.add(listener);
    return () => this.statusListeners.delete(listener);
  }

  private emitStatus(): void {
    const status = this.getStatus();
    for (const l of this.statusListeners) l(status);
  }

  async ensureConnected(): Promise<NativeApiClient> {
    if (this.client) return this.client;
    if (this.connecting) return this.connecting;
    this.connecting = this.doConnect();
    try {
      return await this.connecting;
    } finally {
      this.connecting = null;
    }
  }

  private async doConnect(): Promise<NativeApiClient> {
    let port: NativePortLike;
    try {
      port = this.connectNative(NATIVE_HOST_ID);
    } catch (err) {
      this.scheduleReconnect();
      throw err instanceof Error ? err : new Error(String(err));
    }

    const client = new NativeApiClient({ postMessage: (msg) => port.postMessage(msg) });
    client.onEvent((event) => {
      for (const l of this.eventListeners) l(event);
    });

    port.onMessage.addListener((msg) => client.handleMessage(msg));
    port.onDisconnect.addListener(() => this.handleDisconnect(port));

    this.port = port;
    this.client = client;
    this.backoffMs = INITIAL_BACKOFF_MS;

    if (this.subscribedEvents.length > 0) {
      client.subscribe(this.subscribedEvents, this.subscribedTasks).catch(() => {
        // Best-effort; a fresh subscribe is retried by whoever calls subscribe() again.
      });
    }

    this.emitStatus();
    return client;
  }

  private handleDisconnect(disconnectedPort: NativePortLike): void {
    if (this.port !== disconnectedPort) return; // stale listener from a superseded port
    const message = disconnectedPort.error?.message ?? 'native host disconnected';
    this.client?.rejectAllPending(new Error(message));
    this.client = null;
    this.port = null;
    this.lastRunning = null;
    this.emitStatus();
    this.scheduleReconnect();
  }

  private scheduleReconnect(): void {
    // Lazy by design: only keep retrying while something actually wants live events
    // (badge/notifications subscription). Plain requests reconnect on demand instead.
    if (this.subscribedEvents.length === 0) return;
    if (this.reconnectTimer !== undefined) return;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = undefined;
      this.ensureConnected().catch(() => {
        // scheduleReconnect() was already called by doConnect()'s catch path.
      });
    }, this.backoffMs);
    this.backoffMs = Math.min(this.backoffMs * 2, MAX_BACKOFF_MS);
  }

  async subscribe(events: string[], tasks?: string[]): Promise<void> {
    this.subscribedEvents = events;
    this.subscribedTasks = tasks;
    const client = await this.ensureConnected();
    await client.subscribe(events, tasks);
  }

  async request<T = unknown>(method: HttpMethod, path: string, body?: unknown): Promise<T> {
    const client = await this.ensureConnected();
    return client.request<T>(method, path, body);
  }

  get<T = unknown>(path: string): Promise<T> {
    return this.request<T>('GET', path);
  }

  post<T = unknown>(path: string, body?: unknown): Promise<T> {
    return this.request<T>('POST', path, body);
  }

  /** Never throws: returns `null` status fields when the host or app is unreachable. */
  async ping(): Promise<NativeStatus> {
    try {
      const client = await this.ensureConnected();
      const pong = await client.ping();
      this.lastRunning = pong.running;
      this.lastVersion = pong.version;
      this.emitStatus();
      return this.getStatus();
    } catch {
      this.lastRunning = null;
      this.emitStatus();
      return this.getStatus();
    }
  }

  async launch(): Promise<void> {
    const client = await this.ensureConnected();
    client.launch();
  }
}

function defaultConnectNative(application: string): NativePortLike {
  return browser.runtime.connectNative(application) as unknown as NativePortLike;
}
