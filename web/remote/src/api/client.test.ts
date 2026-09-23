import { test } from "node:test";
import assert from "node:assert/strict";
import { ApiClient, taskFilterToQuery } from "./client.ts";
import { ApiError } from "./errors.ts";

type FetchArgs = [input: string | URL | Request, init?: RequestInit];

function installFetch(handler: (...args: FetchArgs) => Promise<Response> | Response): () => void {
  const original = globalThis.fetch;
  const stub = ((...args: FetchArgs) => Promise.resolve(handler(...args))) as typeof fetch;
  globalThis.fetch = stub;
  return () => {
    globalThis.fetch = original;
  };
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
}

test("client attaches bearer token and parses a JSON success response", async () => {
  let seenAuth: string | null = null;
  const restore = installFetch((_input, init) => {
    seenAuth = new Headers(init?.headers).get("Authorization");
    return jsonResponse(200, { version: "1.0.0" });
  });
  try {
    const client = new ApiClient({ getToken: () => "tok123", onUnauthorized: () => {} });
    const info = await client.info();
    assert.equal(seenAuth, "Bearer tok123");
    assert.equal((info as unknown as { version: string }).version, "1.0.0");
  } finally {
    restore();
  }
});

test("client omits Authorization header when there is no token", async () => {
  let seenAuth: string | null = "unset";
  const restore = installFetch((_input, init) => {
    seenAuth = new Headers(init?.headers).get("Authorization");
    return jsonResponse(200, {});
  });
  try {
    const client = new ApiClient({ getToken: () => null, onUnauthorized: () => {} });
    await client.info();
    assert.equal(seenAuth, null);
  } finally {
    restore();
  }
});

test("client calls onUnauthorized and throws on a 401", async () => {
  const restore = installFetch(() => jsonResponse(401, { error: { type: "unauthorized", message: "expired" } }));
  let calledUnauthorized = false;
  try {
    const client = new ApiClient({ getToken: () => "stale", onUnauthorized: () => { calledUnauthorized = true; } });
    await assert.rejects(() => client.info(), (err: unknown) => {
      assert.ok(err instanceof ApiError);
      assert.equal(err.kind, "unauthorized");
      assert.equal(err.status, 401);
      return true;
    });
    assert.equal(calledUnauthorized, true);
  } finally {
    restore();
  }
});

test("client maps a non-OK response with a well-formed error envelope", async () => {
  const restore = installFetch(() => jsonResponse(409, { error: { type: "conflict", message: "duplicate task" } }));
  try {
    const client = new ApiClient({ getToken: () => null, onUnauthorized: () => {} });
    await assert.rejects(() => client.getTask("t1"), (err: unknown) => {
      assert.ok(err instanceof ApiError);
      assert.equal(err.kind, "conflict");
      assert.equal(err.message, "duplicate task");
      return true;
    });
  } finally {
    restore();
  }
});

test("client maps a thrown network failure", async () => {
  const restore = installFetch(() => {
    throw new TypeError("Failed to fetch");
  });
  try {
    const client = new ApiClient({ getToken: () => null, onUnauthorized: () => {} });
    await assert.rejects(() => client.dashboard(), (err: unknown) => {
      assert.ok(err instanceof ApiError);
      assert.equal(err.kind, "network");
      return true;
    });
  } finally {
    restore();
  }
});

test("client treats a 204 response as a void success", async () => {
  const restore = installFetch(() => new Response(null, { status: 204 }));
  try {
    const client = new ApiClient({ getToken: () => null, onUnauthorized: () => {} });
    await assert.doesNotReject(() => client.deleteTask("t1", false));
  } finally {
    restore();
  }
});

test("client posts a JSON body for mutating requests", async () => {
  let seenBody: unknown;
  const restore = installFetch(async (_input, init) => {
    seenBody = JSON.parse(String(init?.body ?? "{}"));
    return jsonResponse(200, { task: {}, duplicate: null });
  });
  try {
    const client = new ApiClient({ getToken: () => null, onUnauthorized: () => {} });
    await client.addTask({ url: "https://example.com/a.zip", mirrors: [], tags: [], options: {}, start: true, origin: "remote" });
    assert.equal((seenBody as { url: string }).url, "https://example.com/a.zip");
  } finally {
    restore();
  }
});

test("taskFilterToQuery serializes repeatable state params and scalars", () => {
  const qs = taskFilterToQuery({ state: ["downloading", "queued"], text: "movie", limit: 50, desc: true });
  const params = new URLSearchParams(qs.slice(1));
  assert.deepEqual(params.getAll("state"), ["downloading", "queued"]);
  assert.equal(params.get("text"), "movie");
  assert.equal(params.get("limit"), "50");
  assert.equal(params.get("desc"), "true");
});

test("taskFilterToQuery omits undefined fields and empty filter yields empty string", () => {
  assert.equal(taskFilterToQuery({}), "");
});
