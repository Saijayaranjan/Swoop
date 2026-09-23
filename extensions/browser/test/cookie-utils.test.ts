import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildCookieHeader } from '../src/shared/cookie-utils.ts';

test('buildCookieHeader joins name=value pairs with "; "', () => {
  const header = buildCookieHeader([
    { name: 'session', value: 'abc123' },
    { name: 'theme', value: 'dark' },
  ]);
  assert.equal(header, 'session=abc123; theme=dark');
});

test('buildCookieHeader returns an empty string for no cookies', () => {
  assert.equal(buildCookieHeader([]), '');
});

test('buildCookieHeader skips cookies with an empty name', () => {
  const header = buildCookieHeader([
    { name: '', value: 'ignored' },
    { name: 'a', value: '1' },
  ]);
  assert.equal(header, 'a=1');
});
