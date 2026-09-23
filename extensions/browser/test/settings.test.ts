import { test } from 'node:test';
import assert from 'node:assert/strict';
import { SettingsStore, type StorageAreaLike } from '../src/shared/settings.ts';
import { defaultExtensionSettings } from '../src/shared/types.ts';

class FakeStorageArea implements StorageAreaLike {
  private data: Record<string, unknown> = {};
  private listeners: Array<(changes: Record<string, { newValue?: unknown }>) => void> = [];
  onChanged = {
    addListener: (cb: (changes: Record<string, { newValue?: unknown }>) => void): void => {
      this.listeners.push(cb);
    },
  };

  async get(keys: string | string[] | null): Promise<Record<string, unknown>> {
    if (keys === null) return { ...this.data };
    const list = Array.isArray(keys) ? keys : [keys];
    const out: Record<string, unknown> = {};
    for (const key of list) if (key in this.data) out[key] = this.data[key];
    return out;
  }

  async set(items: Record<string, unknown>): Promise<void> {
    this.data = { ...this.data, ...items };
    for (const [key, value] of Object.entries(items)) {
      for (const listener of this.listeners) listener({ [key]: { newValue: value } });
    }
  }
}

test('get() returns defaults when nothing is stored', async () => {
  const store = new SettingsStore(new FakeStorageArea());
  const settings = await store.get();
  assert.deepEqual(settings, defaultExtensionSettings());
});

test('set()/get() round-trip and cache in-memory', async () => {
  const area = new FakeStorageArea();
  const store = new SettingsStore(area);
  const next = { ...defaultExtensionSettings(), intercept_downloads: false };
  await store.set(next);
  assert.equal((await store.get()).intercept_downloads, false);

  // A second store instance reading the same area sees the persisted value too.
  const store2 = new SettingsStore(area);
  assert.equal((await store2.get()).intercept_downloads, false);
});

test('update() merges a partial patch onto the current settings', async () => {
  const store = new SettingsStore(new FakeStorageArea());
  const updated = await store.update({ show_confirmation: false });
  assert.equal(updated.show_confirmation, false);
  assert.equal(updated.detect_media, defaultExtensionSettings().detect_media);
});

test('addAlwaysBrowserDomain / removeAlwaysBrowserDomain are idempotent', async () => {
  const store = new SettingsStore(new FakeStorageArea());
  await store.addAlwaysBrowserDomain('example.com');
  await store.addAlwaysBrowserDomain('example.com'); // no duplicate
  let settings = await store.get();
  assert.deepEqual(settings.always_browser_domains, ['example.com']);

  await store.removeAlwaysBrowserDomain('example.com');
  settings = await store.get();
  assert.deepEqual(settings.always_browser_domains, []);
});

test('addExcludedDomain / removeExcludedDomain', async () => {
  const store = new SettingsStore(new FakeStorageArea());
  await store.addExcludedDomain('tracker.example');
  assert.deepEqual((await store.get()).excluded_domains, ['tracker.example']);
  await store.removeExcludedDomain('tracker.example');
  assert.deepEqual((await store.get()).excluded_domains, []);
});

test('onChange fires when storage.onChanged reports a new value from elsewhere', async () => {
  const area = new FakeStorageArea();
  const store = new SettingsStore(area);
  await store.get(); // prime the cache

  const seen: boolean[] = [];
  store.onChange((settings) => seen.push(settings.intercept_downloads));

  // Simulate another context (e.g. the popup) writing settings directly to the same area.
  await area.set({ ospreySettings: { ...defaultExtensionSettings(), intercept_downloads: false } });

  assert.deepEqual(seen, [false]);
  assert.equal((await store.get()).intercept_downloads, false);
});
