import type { MaybePromise } from "@tysel/types";

export async function invokeFetch(
  handler: (request: Request) => MaybePromise<Response>,
  input: RequestInfo | URL,
  init?: RequestInit,
): Promise<Response> {
  const request = input instanceof Request ? input : new Request(input, init);
  return handler(request);
}

export async function invokeFetchWithRuntime<Runtime>(
  handler: (request: Request, runtime: Runtime) => MaybePromise<Response>,
  runtime: Runtime,
  input: RequestInfo | URL,
  init?: RequestInit,
): Promise<Response> {
  const request = input instanceof Request ? input : new Request(input, init);
  return handler(request, runtime);
}

export type TestBody = () => unknown | Promise<unknown>;
export type TestFunction = (name: string, body: TestBody) => void;

export interface Assert {
  (condition: unknown, message?: string): asserts condition;
  equal(actual: unknown, expected: unknown, message?: string): void;
  deepEqual(actual: unknown, expected: unknown, message?: string): void;
}

type TestRuntime = typeof globalThis & {
  __tysel_test_register?: TestFunction;
  __tysel_assert?: Assert;
};

function runtimeTest(): TestFunction {
  const register = (globalThis as TestRuntime).__tysel_test_register;
  if (!register) throw new Error("test() is only available under `tysel test`");
  return register;
}

function runtimeAssert(): Assert {
  const value = (globalThis as TestRuntime).__tysel_assert;
  if (!value) throw new Error("assert is only available under `tysel test`");
  return value;
}

export const test: TestFunction = (name, body) => runtimeTest()(name, body);

export const assert: Assert = Object.assign(
  function assert(condition: unknown, message?: string): asserts condition {
    const runtime: Assert = runtimeAssert();
    runtime(condition, message);
  },
  {
    equal(actual: unknown, expected: unknown, message?: string) {
      runtimeAssert().equal(actual, expected, message);
    },
    deepEqual(actual: unknown, expected: unknown, message?: string) {
      runtimeAssert().deepEqual(actual, expected, message);
    },
  },
);

declare global {
  var test: TestFunction;
  var assert: Assert;
}
