import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isHostEvent, isHostPong, isHostResponse } from '../src/shared/native-protocol.ts';

test('isHostResponse identifies {id, ok, ...} frames only', () => {
  assert.equal(isHostResponse({ id: 1, ok: true, status: 200, body: {} }), true);
  assert.equal(isHostResponse({ id: 2, ok: false, error: { type: 'x', message: 'y' } }), true);
  assert.equal(isHostResponse({ type: 'pong', version: '1', running: true }), false);
  assert.equal(isHostResponse({ type: 'event', event: {} }), false);
  assert.equal(isHostResponse(null), false);
  assert.equal(isHostResponse('nope'), false);
});

test('isHostPong identifies {type:"pong"} frames only', () => {
  assert.equal(isHostPong({ type: 'pong', version: '0.1.0', running: false }), true);
  assert.equal(isHostPong({ id: 1, ok: true, status: 200, body: null }), false);
});

test('isHostEvent identifies {type:"event"} frames only', () => {
  assert.equal(isHostEvent({ type: 'event', event: { type: 'progress', data: [] } }), true);
  assert.equal(isHostEvent({ type: 'pong', version: '0.1.0', running: true }), false);
});
