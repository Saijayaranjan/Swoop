import { test } from "node:test";
import assert from "node:assert/strict";
import { formatPairingCode, isValidPairingCode } from "./pairingCode.ts";

test("formatPairingCode inserts dash after 4 chars", () => {
  assert.equal(formatPairingCode("abcdefgh"), "ABCD-EFGH");
  assert.equal(formatPairingCode("ABCD"), "ABCD");
  assert.equal(formatPairingCode("abc"), "ABC");
  assert.equal(formatPairingCode(""), "");
});

test("formatPairingCode strips non-alphanumeric and truncates", () => {
  assert.equal(formatPairingCode("ab-cd ef!gh"), "ABCD-EFGH");
  assert.equal(formatPairingCode("abcdefghijkl"), "ABCD-EFGH");
});

test("formatPairingCode is idempotent on already-formatted input", () => {
  const once = formatPairingCode("wxyz1234");
  assert.equal(formatPairingCode(once), once);
});

test("isValidPairingCode", () => {
  assert.equal(isValidPairingCode("ABCD-EFGH"), true);
  assert.equal(isValidPairingCode("ABCD-EFG"), false);
  assert.equal(isValidPairingCode("abcd-efgh"), false);
  assert.equal(isValidPairingCode("ABCDEFGH"), false);
});
