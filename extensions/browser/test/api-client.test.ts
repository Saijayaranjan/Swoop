import { test } from 'node:test';
import assert from 'node:assert/strict';
import { ApiError, NativeApiClient, TimeoutError } from '../src/shared/api-client.ts';
import type { HostOutgoingMessage } from '../src/shared/native-protocol.ts';

class FakeTransport {
  sent: HostOutgoingMessage[] = [];
  postMessage(message: HostOutgoingMessage): void {
    this.sent.push(message);
  }
}

test('request/response are correlated by id', async () => {
  const transport = new FakeTransport();
  const client = new NativeApiClient(transport);

  const promise = client.request('GET', '/api/v1/info');
  assert.equal(transport.sent.length, 1);
  const sent = transport.sent[0] as { id: number; type: string; method: string; path: string };
  assert.equal(sent.type, 'request');
  assert.equal(sent.method, 'GET');
  assert.equal(sent.path, '/api/v1/info');

  client.handleMessage({ id: sent.id, ok: true, status: 200, body: { version: '0.1.0' } });
  const result = await promise;
  assert.deepEqual(result, { version: '0.1.0' });
});

test('two concurrent requests resolve independently by id', async () => {
  const transport = new FakeTransport();
  const client = new NativeApiClient(transport);

  const first = client.get('/a');
  const second = client.get('/b');
  const ids = transport.sent.map((m) => (m as { id: number }).id);
  assert.equal(new Set(ids).size, 2, 'ids must be unique');

  // Resolve out of order to prove correlation isn't positional.
  client.handleMessage({ id: ids[1], ok: true, status: 200, body: 'b-result' });
  client.handleMessage({ id: ids[0], ok: true, status: 200, body: 'a-result' });

  assert.equal(await first, 'a-result');
  assert.equal(await second, 'b-result');
});

test('an error response rejects with ApiError carrying kind/status/message', async () => {
  const transport = new FakeTransport();
  const client = new NativeApiClient(transport);

  const promise = client.post('/api/v1/tasks', { url: 'https://example.com/a.zip' });
  const id = (transport.sent[0] as { id: number }).id;
  client.handleMessage({
    id,
    ok: false,
    error: { type: 'unavailable', message: 'Swoop is not running' },
  });

  await assert.rejects(promise, (err: unknown) => {
    assert.ok(err instanceof ApiError);
    assert.equal(err.kind, 'unavailable');
    assert.equal(err.message, 'Swoop is not running');
    return true;
  });
});

test('a request that never gets a response times out', async () => {
  const transport = new FakeTransport();
  const client = new NativeApiClient(transport);

  await assert.rejects(client.request('GET', '/api/v1/info', undefined, 20), TimeoutError);
});

test('a late response after timeout is ignored (no crash, no dangling resolve)', async () => {
  const transport = new FakeTransport();
  const client = new NativeApiClient(transport);

  await assert.rejects(client.request('GET', '/api/v1/info', undefined, 10), TimeoutError);
  const id = (transport.sent[0] as { id: number }).id;
  // Must not throw even though nothing is pending for this id any more.
  assert.doesNotThrow(() => client.handleMessage({ id, ok: true, status: 200, body: 'late' }));
});

test('rejectAllPending fails every in-flight request (used on port disconnect)', async () => {
  const transport = new FakeTransport();
  const client = new NativeApiClient(transport);

  const a = client.get('/a');
  const b = client.get('/b');
  client.rejectAllPending(new Error('native host disconnected'));

  await assert.rejects(a, /disconnected/);
  await assert.rejects(b, /disconnected/);
});

test('onEvent fans out pushed {type:"event"} frames to all listeners', () => {
  const transport = new FakeTransport();
  const client = new NativeApiClient(transport);

  const received: unknown[] = [];
  const unsubscribe = client.onEvent((event) => received.push(event));
  client.onEvent((event) => received.push({ second: event }));

  client.handleMessage({ type: 'event', event: { type: 'global_stats', data: { active: 3 } } });
  assert.equal(received.length, 2);

  unsubscribe();
  client.handleMessage({ type: 'event', event: { type: 'global_stats', data: { active: 4 } } });
  assert.equal(received.length, 3); // only the still-subscribed listener fired again
});

test('ping resolves from an un-correlated pong frame', async () => {
  const transport = new FakeTransport();
  const client = new NativeApiClient(transport);

  const promise = client.ping(1000);
  client.handleMessage({ type: 'pong', version: '0.1.0', running: true });
  const pong = await promise;
  assert.deepEqual(pong, { type: 'pong', version: '0.1.0', running: true });
});

test('subscribe sends the events/tasks payload and awaits an ack', async () => {
  const transport = new FakeTransport();
  const client = new NativeApiClient(transport);

  const promise = client.subscribe(['task_state_changed', 'progress'], ['task-1']);
  const sent = transport.sent[0] as {
    id: number;
    type: string;
    events: string[];
    tasks: string[];
  };
  assert.equal(sent.type, 'subscribe');
  assert.deepEqual(sent.events, ['task_state_changed', 'progress']);
  assert.deepEqual(sent.tasks, ['task-1']);

  client.handleMessage({ id: sent.id, ok: true, status: 200, body: null });
  await promise;
});
