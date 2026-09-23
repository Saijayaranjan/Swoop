import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHash, buildHash, DEFAULT_ROUTE } from "./router.ts";

test("parseHash defaults to downloads for empty/unknown hash", () => {
  assert.deepEqual(parseHash(""), DEFAULT_ROUTE);
  assert.deepEqual(parseHash("#"), DEFAULT_ROUTE);
  assert.deepEqual(parseHash("#/nonsense"), DEFAULT_ROUTE);
});

test("parseHash parses top-level views", () => {
  assert.deepEqual(parseHash("#/add"), { view: "add", taskId: null });
  assert.deepEqual(parseHash("#/dashboard"), { view: "dashboard", taskId: null });
  assert.deepEqual(parseHash("#/speed"), { view: "speed", taskId: null });
  assert.deepEqual(parseHash("#/settings"), { view: "settings", taskId: null });
  assert.deepEqual(parseHash("#/pair"), { view: "pair", taskId: null });
  assert.deepEqual(parseHash("#/downloads"), { view: "downloads", taskId: null });
});

test("parseHash parses task detail routes", () => {
  assert.deepEqual(parseHash("#/downloads/abc-123"), { view: "detail", taskId: "abc-123" });
});

test("parseHash decodes URI-encoded task ids", () => {
  assert.deepEqual(parseHash("#/downloads/a%2Fb"), { view: "detail", taskId: "a/b" });
});

test("buildHash is the inverse of parseHash for simple views", () => {
  for (const view of ["downloads", "add", "dashboard", "speed", "settings", "pair"] as const) {
    const route = { view, taskId: null };
    assert.deepEqual(parseHash(buildHash(route)), route);
  }
});

test("buildHash builds detail routes", () => {
  assert.equal(buildHash({ view: "detail", taskId: "xyz" }), "#/downloads/xyz");
  assert.deepEqual(parseHash(buildHash({ view: "detail", taskId: "xyz" })), { view: "detail", taskId: "xyz" });
});

test("buildHash encodes special characters in task ids", () => {
  const hash = buildHash({ view: "detail", taskId: "a/b c" });
  assert.deepEqual(parseHash(hash), { view: "detail", taskId: "a/b c" });
});
