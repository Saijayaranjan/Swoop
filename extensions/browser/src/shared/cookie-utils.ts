/**
 * Builds the `options.cookies` header string forwarded on intercepted downloads
 * (docs/api/extension.md: "cookies for the URL (`chrome.cookies.getAll` -> `options.cookies`)").
 * Pure module — takes plain cookie records, not the `browser.cookies` API, so it is trivially
 * testable.
 */

export interface CookieLike {
  name: string;
  value: string;
}

/** RFC 6265 `Cookie` header: `name=value; name2=value2`, in the given order. */
export function buildCookieHeader(cookies: readonly CookieLike[]): string {
  return cookies
    .filter((c) => c.name.length > 0)
    .map((c) => `${c.name}=${c.value}`)
    .join('; ');
}
