// i18n scaffold: merges each locale's partial dictionary over the English source of truth so
// every key always resolves to *something*, and exposes a small `t(key, vars?)` helper with
// `{{var}}` interpolation.

import en, { type TranslationKey } from "./en.ts";
import hi from "./hi.ts";
import ta from "./ta.ts";

export type { TranslationKey };
export type Locale = "en" | "hi" | "ta";

export const SUPPORTED_LOCALES: readonly Locale[] = ["en", "hi", "ta"];

const partials: Record<Locale, Partial<Record<TranslationKey, string>>> = {
  en,
  hi,
  ta,
};

const mergedCache = new Map<Locale, Record<TranslationKey, string>>();

function mergedDictionary(locale: Locale): Record<TranslationKey, string> {
  const cached = mergedCache.get(locale);
  if (cached) {
    return cached;
  }
  const merged = { ...en, ...partials[locale] } as Record<TranslationKey, string>;
  mergedCache.set(locale, merged);
  return merged;
}

export function isSupportedLocale(value: string): value is Locale {
  return (SUPPORTED_LOCALES as readonly string[]).includes(value);
}

/** Best-effort locale detection from the browser, falling back to English. */
export function detectLocale(): Locale {
  if (typeof navigator === "undefined") {
    return "en";
  }
  const candidates = navigator.languages && navigator.languages.length > 0 ? navigator.languages : [navigator.language];
  for (const tag of candidates) {
    const base = tag.split("-")[0]?.toLowerCase();
    if (base && isSupportedLocale(base)) {
      return base;
    }
  }
  return "en";
}

/** Replace `{{name}}` placeholders in `template` with values from `vars`. */
export function interpolate(template: string, vars?: Record<string, string | number>): string {
  if (!vars) {
    return template;
  }
  return template.replace(/\{\{(\w+)\}\}/g, (match, name: string) => {
    const value = vars[name];
    return value === undefined ? match : String(value);
  });
}

/** Create a `t(key, vars?)` translator bound to a locale. Unknown keys return the key itself. */
export function createTranslator(locale: Locale): (key: TranslationKey, vars?: Record<string, string | number>) => string {
  const dict = mergedDictionary(locale);
  return (key, vars) => interpolate(dict[key] ?? key, vars);
}
