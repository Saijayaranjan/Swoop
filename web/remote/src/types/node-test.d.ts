// Minimal ambient types for the `node:test` / `node:assert/strict` APIs we use in tests.
// We intentionally avoid depending on @types/node (the brief limits devDependencies to esbuild
// and typescript) -- this project only uses a small, stable slice of the test runner API.

declare module "node:test" {
  export type TestFn = () => void | Promise<void>;
  export function test(name: string, fn: TestFn): void;
  export function describe(name: string, fn: () => void): void;
  const _default: { test: typeof test; describe: typeof describe };
  export default _default;
}

declare module "node:assert/strict" {
  type ErrorMatcher =
    | RegExp
    | (new (...args: never[]) => Error)
    | ((error: unknown) => boolean)
    | Record<string, unknown>
    | Error;

  function ok(value: unknown, message?: string | Error): asserts value;
  function equal<T>(actual: T, expected: T, message?: string | Error): void;
  function notEqual(actual: unknown, expected: unknown, message?: string | Error): void;
  function deepEqual(actual: unknown, expected: unknown, message?: string | Error): void;
  function notDeepEqual(actual: unknown, expected: unknown, message?: string | Error): void;
  function match(value: string, regex: RegExp, message?: string | Error): void;
  function throws(fn: () => unknown, error?: ErrorMatcher, message?: string | Error): void;
  function rejects(
    fn: (() => Promise<unknown>) | Promise<unknown>,
    error?: ErrorMatcher,
    message?: string | Error,
  ): Promise<void>;
  function doesNotReject(fn: (() => Promise<unknown>) | Promise<unknown>, message?: string | Error): Promise<void>;
  function fail(message?: string | Error): never;

  const assert: {
    ok: typeof ok;
    equal: typeof equal;
    notEqual: typeof notEqual;
    deepEqual: typeof deepEqual;
    notDeepEqual: typeof notDeepEqual;
    match: typeof match;
    throws: typeof throws;
    rejects: typeof rejects;
    doesNotReject: typeof doesNotReject;
    fail: typeof fail;
  };
  export default assert;
  export { ok, equal, notEqual, deepEqual, notDeepEqual, match, throws, rejects, doesNotReject, fail };
}
