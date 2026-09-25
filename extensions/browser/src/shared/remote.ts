/**
 * Remote-connection helpers (docs/api/rest.md "Pairing", "Authentication"). Pure — no browser
 * globals — so the URL rules are unit tested (test/remote.test.ts).
 *
 * A remote Swoop is reached over its TLS listener with a paired-device bearer token. Plain
 * `http:` is only accepted for loopback addresses, so a token is never sent in clear text over a
 * network.
 */

/** `browser.storage.local` key holding the device token, kept apart from the settings object. */
export const REMOTE_TOKEN_KEY = 'swoopRemoteToken';

const LOOPBACK_HOSTS = new Set(['localhost', '127.0.0.1', '[::1]', '::1']);

/**
 * Normalises what a user typed into a base URL (`https://host:port`), or returns `null` when it
 * is not acceptable. Adds `https://` when no scheme is given, drops any path such as `/api/v1`,
 * and rejects `http:` for anything but loopback.
 */
export function normalizeRemoteUrl(input: string): string | null {
  const trimmed = input.trim();
  if (trimmed.length === 0) return null;
  const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed) ? trimmed : `https://${trimmed}`;
  let url: URL;
  try {
    url = new URL(withScheme);
  } catch {
    return null;
  }
  if (url.username || url.password) return null;
  const host = url.hostname.toLowerCase();
  if (host.length === 0) return null;
  if (url.protocol === 'http:') {
    if (!LOOPBACK_HOSTS.has(host)) return null;
  } else if (url.protocol !== 'https:') {
    return null;
  }
  return `${url.protocol}//${url.host}`;
}

/** The WebSocket event-stream URL for a normalised base URL (docs/api/websocket.md). */
export function remoteEventsUrl(baseUrl: string, token: string): string {
  const ws = baseUrl.replace(/^http/i, 'ws');
  return `${ws}/api/v1/events?token=${encodeURIComponent(token)}`;
}

/** Pairing codes are shown as `ABCD-EFGH`; accept any case and missing/extra separators. */
export function normalizePairingCode(input: string): string | null {
  const compact = input.toUpperCase().replace(/[^A-Z0-9]/g, '');
  if (compact.length !== 8) return null;
  return `${compact.slice(0, 4)}-${compact.slice(4)}`;
}
