import { test } from 'node:test';
import assert from 'node:assert/strict';
import { normalizePairingCode, normalizeRemoteUrl, remoteEventsUrl } from '../src/shared/remote.ts';

test('normalizeRemoteUrl adds https and strips paths', () => {
  assert.equal(normalizeRemoteUrl('nas.local:41780'), 'https://nas.local:41780');
  assert.equal(normalizeRemoteUrl(' https://Osprey.example.com/api/v1/ '), 'https://osprey.example.com');
});

test('normalizeRemoteUrl refuses clear-text http except on loopback', () => {
  assert.equal(normalizeRemoteUrl('http://192.168.1.20:41780'), null);
  assert.equal(normalizeRemoteUrl('http://127.0.0.1:41779'), 'http://127.0.0.1:41779');
  assert.equal(normalizeRemoteUrl('ftp://host'), null);
  assert.equal(normalizeRemoteUrl('https://user:pw@host'), null);
  assert.equal(normalizeRemoteUrl(''), null);
});

test('remoteEventsUrl switches to a WebSocket scheme and encodes the token', () => {
  assert.equal(remoteEventsUrl('https://h:1', 'a b'), 'wss://h:1/api/v1/events?token=a%20b');
});

test('normalizePairingCode accepts loose input', () => {
  assert.equal(normalizePairingCode('abcd efgh'), 'ABCD-EFGH');
  assert.equal(normalizePairingCode('ABC'), null);
});
