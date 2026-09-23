import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  decideInterception,
  domainMatches,
  extensionMatches,
  extensionOf,
  isExcludedByPattern,
  isExcludedDomain,
  isNeverInterceptUrl,
  urlPatternMatches,
} from '../src/shared/url-utils.ts';
import { defaultBrowserSettings } from '../src/shared/types.ts';

test('isNeverInterceptUrl recognises unsafe schemes', () => {
  assert.equal(isNeverInterceptUrl('blob:https://example.com/abcd'), true);
  assert.equal(isNeverInterceptUrl('data:text/plain;base64,aGk='), true);
  assert.equal(isNeverInterceptUrl('chrome://extensions'), true);
  assert.equal(isNeverInterceptUrl('moz-extension://abc/page.html'), true);
  assert.equal(isNeverInterceptUrl('https://example.com/file.zip'), false);
  assert.equal(isNeverInterceptUrl('not a url at all'), true);
});

test('domainMatches handles subdomains and wildcard patterns', () => {
  assert.equal(domainMatches('cdn.example.com', 'example.com'), true);
  assert.equal(domainMatches('example.com', 'example.com'), true);
  assert.equal(domainMatches('notexample.com', 'example.com'), false);
  assert.equal(domainMatches('a.b.example.com', '*.example.com'), true);
});

test('isExcludedDomain checks a list of patterns', () => {
  assert.equal(isExcludedDomain('https://cdn.example.com/x.zip', ['example.com']), true);
  assert.equal(isExcludedDomain('https://other.com/x.zip', ['example.com']), false);
});

test('urlPatternMatches supports glob wildcards', () => {
  assert.equal(urlPatternMatches('https://example.com/ads/banner.gif', 'https://*/ads/*'), true);
  assert.equal(urlPatternMatches('https://example.com/img/banner.gif', 'https://*/ads/*'), false);
});

test('isExcludedByPattern matches any pattern in the list', () => {
  const patterns = ['*/ads/*', '*tracking*'];
  assert.equal(isExcludedByPattern('https://x.com/ads/1.png', patterns), true);
  assert.equal(isExcludedByPattern('https://x.com/tracking/pixel.gif', patterns), true);
  assert.equal(isExcludedByPattern('https://x.com/content/1.png', patterns), false);
});

test('extensionOf extracts a lower-cased extension', () => {
  assert.equal(extensionOf('https://example.com/path/file.ZIP?x=1'), 'zip');
  assert.equal(extensionOf('https://example.com/path/'), null);
  assert.equal(extensionOf('https://example.com/.hidden'), null);
  assert.equal(extensionOf('file.tar.gz'), 'gz');
});

test('extensionMatches checks against a list', () => {
  assert.equal(extensionMatches('https://example.com/a.MP4', ['mp4', 'mkv']), true);
  assert.equal(extensionMatches('https://example.com/a.txt', ['mp4', 'mkv']), false);
});

test('decideInterception: matches a normal large zip', () => {
  const settings = defaultBrowserSettings();
  const decision = decideInterception({
    url: 'https://example.com/archive.zip',
    bytesKnown: 5 * 1024 * 1024,
    settings,
  });
  assert.equal(decision.intercept, true);
});

test('decideInterception: never intercepts blob/data URLs', () => {
  const settings = defaultBrowserSettings();
  const decision = decideInterception({
    url: 'blob:https://example.com/abcd-1234',
    bytesKnown: 10 * 1024 * 1024,
    settings,
  });
  assert.equal(decision.intercept, false);
  assert.equal(decision.reason, 'unsupported-scheme');
});

test('decideInterception: respects the disabled toggle', () => {
  const settings = { ...defaultBrowserSettings(), intercept_downloads: false };
  const decision = decideInterception({
    url: 'https://example.com/archive.zip',
    bytesKnown: 1024 * 1024,
    settings,
  });
  assert.equal(decision.intercept, false);
  assert.equal(decision.reason, 'disabled');
});

test('decideInterception: excluded domain wins over a matching extension', () => {
  const settings = { ...defaultBrowserSettings(), excluded_domains: ['example.com'] };
  const decision = decideInterception({
    url: 'https://example.com/archive.zip',
    bytesKnown: 5 * 1024 * 1024,
    settings,
  });
  assert.equal(decision.intercept, false);
  assert.equal(decision.reason, 'excluded-domain');
});

test('decideInterception: always-browser domain overrides settings entirely', () => {
  const settings = defaultBrowserSettings();
  const decision = decideInterception({
    url: 'https://example.com/archive.zip',
    bytesKnown: 5 * 1024 * 1024,
    settings,
    alwaysBrowserDomains: ['example.com'],
  });
  assert.equal(decision.intercept, false);
  assert.equal(decision.reason, 'always-browser-site');
});

test('decideInterception: below the minimum size is left to the browser', () => {
  const settings = { ...defaultBrowserSettings(), intercept_min_size: 10 * 1024 * 1024 };
  const decision = decideInterception({
    url: 'https://example.com/archive.zip',
    bytesKnown: 1024,
    settings,
  });
  assert.equal(decision.intercept, false);
  assert.equal(decision.reason, 'below-min-size');
});

test('decideInterception: unknown size (bytesKnown=null) is not blocked by the size rule', () => {
  const settings = { ...defaultBrowserSettings(), intercept_min_size: 10 * 1024 * 1024 };
  const decision = decideInterception({
    url: 'https://example.com/archive.zip',
    bytesKnown: null,
    settings,
  });
  assert.equal(decision.intercept, true);
});

test('decideInterception: extension not in the configured list is left to the browser', () => {
  const settings = defaultBrowserSettings();
  const decision = decideInterception({
    url: 'https://example.com/page.html',
    bytesKnown: 5 * 1024 * 1024,
    settings,
  });
  assert.equal(decision.intercept, false);
  assert.equal(decision.reason, 'extension-not-listed');
});
