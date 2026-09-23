/**
 * Thin wrapper over `browser.i18n.getMessage`. All user-visible strings live in
 * `_locales/<lang>/messages.json`; this module is the only place that reads them so a missing key
 * is easy to spot (falls back to the key itself rather than throwing or rendering blank).
 */

import browser from 'webextension-polyfill';

export function t(key: string, substitutions?: string | string[]): string {
  const message = browser.i18n.getMessage(key, substitutions);
  return message.length > 0 ? message : key;
}
