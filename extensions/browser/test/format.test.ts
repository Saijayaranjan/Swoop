import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formatBytes, formatDuration, formatEta, formatPercent, formatSpeed, truncateMiddle } from '../src/shared/format.ts';

test('formatBytes scales through units', () => {
  assert.equal(formatBytes(0), '0 B');
  assert.equal(formatBytes(512), '512 B');
  assert.equal(formatBytes(1536), '1.5 KB');
  assert.equal(formatBytes(5 * 1024 * 1024), '5.0 MB');
  assert.equal(formatBytes(null), '–');
  assert.equal(formatBytes(undefined), '–');
});

test('formatSpeed appends /s', () => {
  assert.equal(formatSpeed(1024), '1.0 KB/s');
  assert.equal(formatSpeed(null), '–');
});

test('formatEta scales seconds -> minutes -> hours -> days', () => {
  assert.equal(formatEta(30), '30s');
  assert.equal(formatEta(90), '2m');
  assert.equal(formatEta(3660), '1h 1m');
  assert.equal(formatEta(90000), '1d 1h');
  assert.equal(formatEta(null), '–');
  assert.equal(formatEta(-5), '–');
});

test('formatPercent guards against an unknown/zero total', () => {
  assert.equal(formatPercent(50, 100), '50%');
  assert.equal(formatPercent(5, 100), '5.0%');
  assert.equal(formatPercent(50, null), '–');
  assert.equal(formatPercent(50, 0), '–');
});

test('formatDuration renders h:mm:ss / m:ss', () => {
  assert.equal(formatDuration(65), '1:05');
  assert.equal(formatDuration(3725), '1:02:05');
  assert.equal(formatDuration(null), '–');
});

test('truncateMiddle keeps the string intact under the limit', () => {
  assert.equal(truncateMiddle('short.txt', 20), 'short.txt');
  const long = 'a-very-long-filename-indeed.zip';
  const truncated = truncateMiddle(long, 12);
  assert.equal(truncated.length, 12);
  assert.ok(truncated.includes('…'));
});
