import { test } from "node:test";
import assert from "node:assert/strict";
import { createTranslator, interpolate, isSupportedLocale, SUPPORTED_LOCALES } from "./index.ts";
import en from "./en.ts";
import hi from "./hi.ts";
import ta from "./ta.ts";

test("interpolate substitutes {{vars}} and leaves unknown ones untouched", () => {
  assert.equal(interpolate("Hello {{name}}", { name: "World" }), "Hello World");
  assert.equal(interpolate("{{count}} selected", { count: 3 }), "3 selected");
  assert.equal(interpolate("No vars here"), "No vars here");
  assert.equal(interpolate("{{missing}} thing", {}), "{{missing}} thing");
});

test("English translator returns the exact en.ts strings", () => {
  const t = createTranslator("en");
  assert.equal(t("nav.downloads"), en["nav.downloads"]);
  assert.equal(t("action.pause"), "Pause");
});

test("Hindi translator uses real translations for common keys and falls back to English otherwise", () => {
  const t = createTranslator("hi");
  assert.equal(t("nav.downloads"), hi["nav.downloads"]);
  assert.notEqual(t("nav.downloads"), en["nav.downloads"]);
  // A key hi.ts does not translate must fall back to English, not a placeholder.
  assert.equal(t("add.error_empty"), en["add.error_empty"]);
  assert.ok(!t("add.error_empty").includes("TODO"));
});

test("Tamil translator uses real translations for common keys and falls back to English otherwise", () => {
  const t = createTranslator("ta");
  assert.equal(t("nav.settings"), ta["nav.settings"]);
  assert.notEqual(t("nav.settings"), en["nav.settings"]);
  assert.equal(t("speed.saved"), en["speed.saved"]);
});

test("hi and ta dictionaries cover at least 40 keys with real (non-English, non-placeholder) text", () => {
  for (const dict of [hi, ta]) {
    const entries = Object.entries(dict);
    assert.ok(entries.length >= 40, `expected >= 40 translated keys, got ${entries.length}`);
    for (const [key, value] of entries) {
      assert.ok(value.length > 0, `empty translation for ${key}`);
      assert.ok(!/todo/i.test(value), `placeholder translation for ${key}`);
      const englishValue = en[key as keyof typeof en];
      assert.notEqual(value, englishValue, `${key} looks untranslated (identical to English)`);
    }
  }
});

test("interpolation works through the translator for keys with placeholders", () => {
  const t = createTranslator("en");
  assert.equal(t("downloads.selected_count", { count: 5 }), "5 selected");
  assert.equal(t("toast.completed", { name: "movie.mkv" }), "movie.mkv finished downloading");
});

test("isSupportedLocale / SUPPORTED_LOCALES", () => {
  assert.deepEqual(SUPPORTED_LOCALES, ["en", "hi", "ta"]);
  assert.equal(isSupportedLocale("hi"), true);
  assert.equal(isSupportedLocale("fr"), false);
});
