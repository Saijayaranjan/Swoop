// Local UI preferences, persisted to localStorage: theme, locale, high-volume WS events, and
// the device name shown to the server (editable after pairing too).

import { createStore } from "./store.ts";
import { detectLocale, type Locale } from "../i18n/index.ts";

const KEY = "osprey.settings";

export type Theme = "system" | "light" | "dark";

export interface UiSettings {
  theme: Theme;
  locale: Locale;
  highVolumeEvents: boolean;
}

function defaults(): UiSettings {
  return { theme: "system", locale: detectLocale(), highVolumeEvents: false };
}

function storageAvailable(): boolean {
  try {
    return typeof localStorage !== "undefined";
  } catch {
    return false;
  }
}

function read(): UiSettings {
  const base = defaults();
  if (!storageAvailable()) return base;
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return base;
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return base;
    const p = parsed as Partial<UiSettings>;
    return {
      theme: p.theme === "light" || p.theme === "dark" || p.theme === "system" ? p.theme : base.theme,
      locale: typeof p.locale === "string" && (p.locale === "en" || p.locale === "hi" || p.locale === "ta") ? p.locale : base.locale,
      highVolumeEvents: typeof p.highVolumeEvents === "boolean" ? p.highVolumeEvents : base.highVolumeEvents,
    };
  } catch {
    return base;
  }
}

export const settingsStore = createStore<UiSettings>(read());

function persist(state: UiSettings): void {
  if (!storageAvailable()) return;
  try {
    localStorage.setItem(KEY, JSON.stringify(state));
  } catch {
    // ignore (private mode / quota)
  }
}

export function setTheme(theme: Theme): void {
  settingsStore.setState((s) => {
    const next = { ...s, theme };
    persist(next);
    return next;
  });
}

export function setLocale(locale: Locale): void {
  settingsStore.setState((s) => {
    const next = { ...s, locale };
    persist(next);
    return next;
  });
}

export function setHighVolumeEvents(enabled: boolean): void {
  settingsStore.setState((s) => {
    const next = { ...s, highVolumeEvents: enabled };
    persist(next);
    return next;
  });
}

/** Resolve "system" to an actual light/dark value using the OS preference. */
export function resolveTheme(theme: Theme): "light" | "dark" {
  if (theme !== "system") return theme;
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return "light";
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}
