/**
 * Pure URL / exclusion-matching helpers shared by the interception, media-detector and popup
 * code. No browser globals — safe to unit test directly (test/url-utils.test.ts).
 */

/** Schemes we must never intercept or otherwise treat as a "real" network download. */
const NEVER_INTERCEPT_SCHEMES = new Set(['blob:', 'data:', 'chrome:', 'moz-extension:', 'chrome-extension:', 'about:', 'file:']);

export function schemeOf(url: string): string | null {
  const m = /^([a-zA-Z][a-zA-Z0-9+.-]*:)/.exec(url);
  return m ? (m[1] as string).toLowerCase() : null;
}

/** True for `blob:`, `data:`, `chrome://`, `moz-extension://`, etc — never candidates for interception. */
export function isNeverInterceptUrl(url: string): boolean {
  const scheme = schemeOf(url);
  if (!scheme) return true; // can't parse -> treat as unsafe/ignorable
  return NEVER_INTERCEPT_SCHEMES.has(scheme);
}

export function hostnameOf(url: string): string | null {
  try {
    return new URL(url).hostname.toLowerCase();
  } catch {
    return null;
  }
}

/** `sub.example.com` matches an exclusion of `example.com`; exact match also counts. */
export function domainMatches(hostname: string, pattern: string): boolean {
  const h = hostname.toLowerCase();
  const p = pattern.toLowerCase().replace(/^\*\./, '');
  return h === p || h.endsWith(`.${p}`);
}

export function isExcludedDomain(url: string, excludedDomains: readonly string[]): boolean {
  const hostname = hostnameOf(url);
  if (!hostname) return false;
  return excludedDomains.some((pattern) => domainMatches(hostname, pattern));
}

/** `pattern` may contain `*` wildcards (glob-style), matched against the full URL. */
export function urlPatternMatches(url: string, pattern: string): boolean {
  if (pattern.length === 0) return false;
  const escaped = pattern.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*');
  try {
    return new RegExp(`^${escaped}$`, 'i').test(url);
  } catch {
    return false;
  }
}

export function isExcludedByPattern(url: string, patterns: readonly string[]): boolean {
  return patterns.some((p) => urlPatternMatches(url, p));
}

export function extensionOf(pathOrUrl: string): string | null {
  try {
    const pathname = pathOrUrl.includes('://') ? new URL(pathOrUrl).pathname : pathOrUrl;
    const base = pathname.split('/').pop() ?? '';
    const dot = base.lastIndexOf('.');
    if (dot <= 0 || dot === base.length - 1) return null;
    return base.slice(dot + 1).toLowerCase();
  } catch {
    return null;
  }
}

export function extensionMatches(url: string, extensions: readonly string[]): boolean {
  const ext = extensionOf(url);
  if (!ext) return false;
  return extensions.some((e) => e.toLowerCase() === ext);
}

export interface InterceptDecisionInput {
  url: string;
  bytesKnown: number | null;
  settings: {
    intercept_downloads: boolean;
    intercept_min_size: number;
    intercept_extensions: readonly string[];
    excluded_domains: readonly string[];
    excluded_url_patterns: readonly string[];
  };
  alwaysBrowserDomains?: readonly string[];
}

export interface InterceptDecision {
  intercept: boolean;
  reason: string;
}

/**
 * Decide whether a browser download should be cancelled and handed to Osprey instead. Mirrors
 * docs/api/extension.md: "when enabled and the item matches (extension list, min size when
 * known, not excluded domain / URL pattern)".
 */
export function decideInterception(input: InterceptDecisionInput): InterceptDecision {
  const { url, bytesKnown, settings } = input;

  if (isNeverInterceptUrl(url)) {
    return { intercept: false, reason: 'unsupported-scheme' };
  }
  if (!settings.intercept_downloads) {
    return { intercept: false, reason: 'disabled' };
  }
  if (input.alwaysBrowserDomains && isExcludedDomain(url, input.alwaysBrowserDomains)) {
    return { intercept: false, reason: 'always-browser-site' };
  }
  if (isExcludedDomain(url, settings.excluded_domains)) {
    return { intercept: false, reason: 'excluded-domain' };
  }
  if (isExcludedByPattern(url, settings.excluded_url_patterns)) {
    return { intercept: false, reason: 'excluded-pattern' };
  }
  const extMatch = extensionMatches(url, settings.intercept_extensions);
  if (!extMatch) {
    return { intercept: false, reason: 'extension-not-listed' };
  }
  if (bytesKnown !== null && bytesKnown < settings.intercept_min_size) {
    return { intercept: false, reason: 'below-min-size' };
  }
  return { intercept: true, reason: 'matched' };
}
