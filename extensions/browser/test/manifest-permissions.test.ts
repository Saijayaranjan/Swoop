import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

// Extension APIs that are undefined unless the manifest requests the matching permission. Using one
// without it throws at runtime (in a service worker, before any listener is registered), which unit
// tests with a mocked `browser` never notice.
const GATED_APIS: Record<string, string> = {
  contextMenus: 'contextMenus',
  cookies: 'cookies',
  downloads: 'downloads',
  notifications: 'notifications',
  storage: 'storage',
  webNavigation: 'webNavigation',
  webRequest: 'webRequest',
  alarms: 'alarms',
  scripting: 'scripting',
  declarativeNetRequest: 'declarativeNetRequest',
};

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sourceFiles(path);
    return path.endsWith('.ts') ? [path] : [];
  });
}

test('every permission-gated browser API used in src/ is requested by the manifest', () => {
  const manifest = JSON.parse(readFileSync('manifests/base.json', 'utf8')) as { permissions: string[] };
  const granted = new Set(manifest.permissions);
  const used = new Set<string>();
  for (const file of sourceFiles('src')) {
    for (const match of readFileSync(file, 'utf8').matchAll(/\b(?:browser|chrome)\.([a-z][A-Za-z]+)\./g)) {
      const api = match[1];
      if (api !== undefined && api in GATED_APIS) used.add(api);
    }
  }
  assert.ok(used.size > 0, 'expected to find browser API usage in src/');
  const missing = [...used].filter((api) => !granted.has(GATED_APIS[api] ?? api)).sort();
  assert.deepEqual(missing, [], `manifest is missing permissions for: ${missing.join(', ')}`);
});
