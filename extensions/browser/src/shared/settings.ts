/**
 * Settings store for `ExtensionSettings` (which embeds the `BrowserSettings` fields the desktop
 * app also knows about — see src/shared/types.ts). Backed by `browser.storage.local` so it
 * survives service-worker suspension and syncs instantly between background/popup/options.
 *
 * Takes a minimal storage-area shape rather than `typeof browser.storage.local` directly so it
 * can be unit tested with an in-memory fake (test/settings.test.ts).
 */

import { defaultExtensionSettings, type ExtensionSettings } from './types.ts';

export interface StorageAreaLike {
  get(keys: string | string[] | null): Promise<Record<string, unknown>>;
  set(items: Record<string, unknown>): Promise<void>;
  onChanged?: {
    addListener(cb: (changes: Record<string, { newValue?: unknown }>) => void): void;
  };
}

const STORAGE_KEY = 'ospreySettings';

function mergeWithDefaults(stored: unknown): ExtensionSettings {
  const defaults = defaultExtensionSettings();
  if (typeof stored !== 'object' || stored === null) return defaults;
  return { ...defaults, ...(stored as Partial<ExtensionSettings>) };
}

export class SettingsStore {
  private area: StorageAreaLike;
  private cache: ExtensionSettings | undefined;
  private listeners = new Set<(settings: ExtensionSettings) => void>();

  constructor(area: StorageAreaLike) {
    this.area = area;
    this.area.onChanged?.addListener((changes) => {
      const change = changes[STORAGE_KEY];
      if (!change) return;
      this.cache = mergeWithDefaults(change.newValue);
      for (const listener of this.listeners) listener(this.cache);
    });
  }

  async get(): Promise<ExtensionSettings> {
    if (this.cache) return this.cache;
    const stored = await this.area.get(STORAGE_KEY);
    this.cache = mergeWithDefaults(stored[STORAGE_KEY]);
    return this.cache;
  }

  async set(settings: ExtensionSettings): Promise<void> {
    this.cache = settings;
    await this.area.set({ [STORAGE_KEY]: settings });
    for (const listener of this.listeners) listener(settings);
  }

  async update(patch: Partial<ExtensionSettings>): Promise<ExtensionSettings> {
    const current = await this.get();
    const next = { ...current, ...patch };
    await this.set(next);
    return next;
  }

  async addAlwaysBrowserDomain(domain: string): Promise<ExtensionSettings> {
    const current = await this.get();
    if (current.always_browser_domains.includes(domain)) return current;
    return this.update({ always_browser_domains: [...current.always_browser_domains, domain] });
  }

  async removeAlwaysBrowserDomain(domain: string): Promise<ExtensionSettings> {
    const current = await this.get();
    return this.update({
      always_browser_domains: current.always_browser_domains.filter((d) => d !== domain),
    });
  }

  async addExcludedDomain(domain: string): Promise<ExtensionSettings> {
    const current = await this.get();
    if (current.excluded_domains.includes(domain)) return current;
    return this.update({ excluded_domains: [...current.excluded_domains, domain] });
  }

  async removeExcludedDomain(domain: string): Promise<ExtensionSettings> {
    const current = await this.get();
    return this.update({
      excluded_domains: current.excluded_domains.filter((d) => d !== domain),
    });
  }

  /** Fires whenever settings change, whether from this context or another (popup/options/bg). */
  onChange(listener: (settings: ExtensionSettings) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
}
