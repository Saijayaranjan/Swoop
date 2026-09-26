import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';

// The ID the Swoop app and `swoop native-host` put in the host manifest's `allowed_origins`
// (CHROMIUM_EXTENSION_ID in native_host.rs, chromiumExtensionId in NativeMessaging.swift).
const FIXED_ID = 'hbfgocpejejjhpigpanikchicoplcfjb';

function idFromKey(key: string): string {
  const hex = createHash('sha256').update(Buffer.from(key, 'base64')).digest('hex').slice(0, 32);
  return [...hex].map((c) => String.fromCharCode(97 + parseInt(c, 16))).join('');
}

for (const target of ['chrome', 'edge']) {
  test(`${target} manifest key yields the fixed extension ID`, () => {
    const manifest = JSON.parse(readFileSync(new URL(`../manifests/${target}.json`, import.meta.url), 'utf8'));
    assert.equal(idFromKey(manifest.key), FIXED_ID);
  });
}
