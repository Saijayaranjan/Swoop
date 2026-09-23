// Auth state: the bearer token and paired-device info live in localStorage so a reload doesn't
// force re-pairing. All access is guarded so this module is safe to import in non-browser
// contexts (tests, SSR-less build tooling).

import { createStore } from "./store.ts";

const TOKEN_KEY = "osprey.token";
const DEVICE_KEY = "osprey.device";

export interface StoredDevice {
  id: string;
  name: string;
}

function storageAvailable(): boolean {
  try {
    return typeof localStorage !== "undefined";
  } catch {
    return false;
  }
}

function readToken(): string | null {
  if (!storageAvailable()) return null;
  try {
    return localStorage.getItem(TOKEN_KEY);
  } catch {
    return null;
  }
}

function readDevice(): StoredDevice | null {
  if (!storageAvailable()) return null;
  try {
    const raw = localStorage.getItem(DEVICE_KEY);
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    if (
      typeof parsed === "object" &&
      parsed !== null &&
      typeof (parsed as { id?: unknown }).id === "string" &&
      typeof (parsed as { name?: unknown }).name === "string"
    ) {
      return parsed as StoredDevice;
    }
    return null;
  } catch {
    return null;
  }
}

export interface AuthState {
  token: string | null;
  device: StoredDevice | null;
}

export const authStore = createStore<AuthState>({ token: readToken(), device: readDevice() });

export function getToken(): string | null {
  return authStore.getState().token;
}

export function setAuth(token: string, device: StoredDevice): void {
  if (storageAvailable()) {
    try {
      localStorage.setItem(TOKEN_KEY, token);
      localStorage.setItem(DEVICE_KEY, JSON.stringify(device));
    } catch {
      // Storage may be unavailable (private mode); the in-memory store still works for this session.
    }
  }
  authStore.setState({ token, device });
}

/** Store a bearer token with no known device record yet (the `?token=` QR handoff). */
export function setTokenOnly(token: string): void {
  if (storageAvailable()) {
    try {
      localStorage.setItem(TOKEN_KEY, token);
    } catch {
      // ignore
    }
  }
  authStore.setState((s) => ({ token, device: s.device }));
}

export function clearAuth(): void {
  if (storageAvailable()) {
    try {
      localStorage.removeItem(TOKEN_KEY);
      localStorage.removeItem(DEVICE_KEY);
    } catch {
      // ignore
    }
  }
  authStore.setState({ token: null, device: null });
}

/** Pull `?token=` from the `osprey://pair` QR handoff URL, if present, and strip it from the URL. */
export function consumeTokenFromUrl(): string | null {
  if (typeof window === "undefined") return null;
  const params = new URLSearchParams(window.location.search);
  const token = params.get("token");
  if (!token) return null;
  params.delete("token");
  const query = params.toString();
  const url = `${window.location.pathname}${query ? `?${query}` : ""}${window.location.hash}`;
  window.history.replaceState(null, "", url);
  return token;
}
