import { test } from "node:test";
import assert from "node:assert/strict";
import { formatBytes, formatSpeed, formatEta, formatPercent, formatRelativeTime } from "./format.ts";

test("formatBytes", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(512), "512 B");
  assert.equal(formatBytes(1024), "1.0 KB");
  assert.equal(formatBytes(1536), "1.5 KB");
  assert.equal(formatBytes(1024 * 1024 * 2.5), "2.5 MB");
  assert.equal(formatBytes(1024 * 1024 * 1024 * 3), "3.0 GB");
  assert.equal(formatBytes(-5), "0 B");
  assert.equal(formatBytes(1024 * 200), "200 KB"); // >= 100: no decimal place
});

test("formatSpeed", () => {
  assert.equal(formatSpeed(0), "0 B/s");
  assert.equal(formatSpeed(1024), "1.0 KB/s");
});

test("formatEta", () => {
  assert.equal(formatEta(null), "--");
  assert.equal(formatEta(undefined), "--");
  assert.equal(formatEta(-1), "--");
  assert.equal(formatEta(0), "0s");
  assert.equal(formatEta(45), "45s");
  assert.equal(formatEta(90), "1m 30s");
  assert.equal(formatEta(3661), "1h 1m");
  assert.equal(formatEta(90000), "1d 1h");
});

test("formatPercent", () => {
  assert.equal(formatPercent(null), "--");
  assert.equal(formatPercent(45.6), "46%");
  assert.equal(formatPercent(150), "100%");
  assert.equal(formatPercent(-5), "0%");
});

test("formatRelativeTime", () => {
  const now = 1_000_000_000;
  assert.equal(formatRelativeTime(now - 1000, now), "just now"); // < 5s
  assert.equal(formatRelativeTime(now - 10_000, now), "10s ago");
  assert.equal(formatRelativeTime(now - 60_000, now), "1m ago");
  assert.equal(formatRelativeTime(now - 3_600_000, now), "1h ago");
});
