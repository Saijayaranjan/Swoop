import { test } from "node:test";
import assert from "node:assert/strict";
import { ApiError, mapResponseError, mapNetworkError } from "./errors.ts";

test("mapResponseError maps a well-formed error envelope", () => {
  const err = mapResponseError(404, { error: { type: "not_found", message: "task not found" } }, false);
  assert.equal(err.status, 404);
  assert.equal(err.kind, "not_found");
  assert.equal(err.message, "task not found");
});

test("mapResponseError maps unauthorized and exposes isUnauthorized", () => {
  const err = mapResponseError(401, { error: { type: "unauthorized", message: "bad token" } }, false);
  assert.equal(err.isUnauthorized, true);
});

test("mapResponseError treats a 401 status as unauthorized even without a matching kind", () => {
  const err = new ApiError(401, "internal", "boom");
  assert.equal(err.isUnauthorized, true);
});

test("mapResponseError falls back to internal for a malformed envelope", () => {
  const err = mapResponseError(500, { oops: true }, false);
  assert.equal(err.kind, "internal");
  assert.equal(err.status, 500);
});

test("mapResponseError handles unparsable bodies", () => {
  const err = mapResponseError(500, undefined, true);
  assert.equal(err.kind, "parse");
  assert.match(err.message, /unparsable/);
});

test("mapNetworkError wraps thrown Error instances", () => {
  const err = mapNetworkError(new TypeError("Failed to fetch"));
  assert.equal(err.kind, "network");
  assert.equal(err.status, 0);
  assert.equal(err.message, "Failed to fetch");
});

test("mapNetworkError handles non-Error throwables", () => {
  const err = mapNetworkError("weird failure");
  assert.equal(err.kind, "network");
  assert.equal(err.message, "weird failure");
});
