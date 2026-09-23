import { test } from "node:test";
import assert from "node:assert/strict";
import { computeWindowedRange } from "./windowedList.ts";

test("computeWindowedRange at top of list", () => {
  const r = computeWindowedRange({ scrollTop: 0, viewportHeight: 500, itemHeight: 50, itemCount: 100, overscan: 2 });
  assert.equal(r.startIndex, 0);
  // visibleCount = ceil(500/50)+1 = 11; endIndex = 0+11+2=13
  assert.equal(r.endIndex, 13);
  assert.equal(r.topPadding, 0);
  assert.equal(r.totalHeight, 5000);
  assert.equal(r.bottomPadding, 5000 - 13 * 50);
});

test("computeWindowedRange scrolled into the middle", () => {
  const r = computeWindowedRange({ scrollTop: 1000, viewportHeight: 500, itemHeight: 50, itemCount: 100, overscan: 2 });
  // firstVisible = floor(1000/50) = 20; startIndex = 18
  assert.equal(r.startIndex, 18);
  assert.equal(r.topPadding, 18 * 50);
  assert.ok(r.endIndex > r.startIndex);
});

test("computeWindowedRange clamps end to itemCount", () => {
  const r = computeWindowedRange({ scrollTop: 4800, viewportHeight: 500, itemHeight: 50, itemCount: 100, overscan: 3 });
  assert.equal(r.endIndex, 100);
  assert.equal(r.bottomPadding, 0);
});

test("computeWindowedRange handles empty list", () => {
  const r = computeWindowedRange({ scrollTop: 0, viewportHeight: 500, itemHeight: 50, itemCount: 0 });
  assert.deepEqual(r, { startIndex: 0, endIndex: 0, topPadding: 0, bottomPadding: 0, totalHeight: 0 });
});

test("computeWindowedRange clamps negative scrollTop", () => {
  const r = computeWindowedRange({ scrollTop: -100, viewportHeight: 300, itemHeight: 40, itemCount: 20 });
  assert.equal(r.startIndex, 0);
});

test("computeWindowedRange default overscan", () => {
  const r = computeWindowedRange({ scrollTop: 0, viewportHeight: 100, itemHeight: 50, itemCount: 50 });
  // visibleCount = ceil(100/50)+1 = 3; default overscan 3 -> endIndex = 0+3+3=6
  assert.equal(r.endIndex, 6);
});
