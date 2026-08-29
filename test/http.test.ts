import { test } from "node:test";
import assert from "node:assert/strict";
import { getJson } from "../src/providers/http.ts";

test("retries a rate limit and reports the wait", async (context) => {
  const previousFetch = globalThis.fetch;
  let calls = 0;
  const statuses: string[] = [];

  globalThis.fetch = (async () => {
    calls += 1;
    return calls === 1
      ? new Response("busy", { status: 429, headers: { "retry-after": "0" } })
      : new Response('{"ok":true}', { status: 200 });
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
  });

  const body = await getJson("https://example.test/models", {}, undefined, (status) => {
    statuses.push(status);
  });

  assert.deepEqual(body, { ok: true });
  assert.equal(calls, 2);
  assert.deepEqual(statuses, ["Rate limited · retrying in 0ms"]);
});

test("an abort stops a pending retry", async (context) => {
  const previousFetch = globalThis.fetch;
  const control = new AbortController();

  globalThis.fetch = (async () =>
    new Response("busy", { status: 503, headers: { "retry-after": "10" } })) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
  });

  const pending = getJson("https://example.test/models", {}, control.signal, () => {
    control.abort(new Error("cancelled"));
  });
  await assert.rejects(pending, /cancelled/);
});

test("a non-retryable response keeps status and a bounded body", async (context) => {
  const previousFetch = globalThis.fetch;
  globalThis.fetch = (async () =>
    new Response("bad key", { status: 401, statusText: "Unauthorized" })) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
  });

  await assert.rejects(
    getJson("https://example.test/models", {}),
    (error: Error & { status?: number; body?: string }) => {
      assert.equal(error.status, 401);
      assert.equal(error.body, "bad key");
      return true;
    },
  );
});

test("an error response cannot accumulate an unbounded body", async (context) => {
  const previousFetch = globalThis.fetch;
  globalThis.fetch = (async () =>
    new Response("x".repeat(20_000), { status: 400, statusText: "Bad Request" })) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
  });

  await assert.rejects(
    getJson("https://example.test/models", {}),
    (error: Error & { body?: string }) => {
      assert.equal(error.body?.length, 2_000);
      return true;
    },
  );
});
