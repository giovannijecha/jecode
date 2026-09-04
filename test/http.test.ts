import { test } from "node:test";
import assert from "node:assert/strict";
import { getJson, postJson, postSse } from "../src/providers/http.ts";

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

test("does not replay a POST after a retryable response", async (context) => {
  const previousFetch = globalThis.fetch;
  let calls = 0;
  const statuses: string[] = [];
  globalThis.fetch = (async () => {
    calls += 1;
    return new Response("busy", { status: 503, statusText: "Unavailable" });
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
  });

  await assert.rejects(
    postJson("https://example.test/generate", {}, { prompt: "hello" }, undefined, (status) => {
      statuses.push(status);
    }),
    (error: Error & { status?: number }) => {
      assert.equal(error.status, 503);
      return true;
    },
  );
  assert.equal(calls, 1);
  assert.deepEqual(statuses, []);
});

test("does not replay a POST after an ambiguous network error", async (context) => {
  const previousFetch = globalThis.fetch;
  let calls = 0;
  globalThis.fetch = (async () => {
    calls += 1;
    throw new Error("socket reset");
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
  });

  await assert.rejects(
    postJson("https://example.test/generate", {}, { prompt: "hello" }),
    /network error calling .*socket reset/,
  );
  assert.equal(calls, 1);
});

test("times out a response handshake even without a caller signal", async (context) => {
  const previousFetch = globalThis.fetch;
  context.mock.timers.enable({ apis: ["setTimeout"] });
  globalThis.fetch = ((_url, init) => new Promise((_resolve, reject) => {
    const signal = init?.signal;
    signal?.addEventListener("abort", () => reject(signal.reason), { once: true });
  })) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    context.mock.timers.reset();
  });

  const pending = getJson("https://example.test/models", {});
  context.mock.timers.tick(60_000);

  await assert.rejects(pending, /timed out waiting for response headers after 60000ms/);
});

test("times out an idle SSE event stream", async (context) => {
  const previousFetch = globalThis.fetch;
  context.mock.timers.enable({ apis: ["setTimeout"] });
  globalThis.fetch = (async () => new Response(new ReadableStream<Uint8Array>(), {
    status: 200,
  })) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    context.mock.timers.reset();
  });

  const events = await postSse("https://example.test/generate", {}, {}, 256, undefined);
  const pending = events.next();
  await new Promise<void>((resolve) => setImmediate(resolve));
  context.mock.timers.tick(120_000);

  await assert.rejects(pending, /SSE stream was idle for 120000ms without an event/);
});

test("SSE comments cannot keep a response alive without model events", async (context) => {
  const previousFetch = globalThis.fetch;
  const encoder = new TextEncoder();
  let stream: ReadableStreamDefaultController<Uint8Array> | undefined;
  let cancelled = false;
  context.mock.timers.enable({ apis: ["setTimeout"] });
  globalThis.fetch = (async () => new Response(new ReadableStream<Uint8Array>({
    start(controller) {
      stream = controller;
    },
    cancel() {
      cancelled = true;
    },
  }), { status: 200 })) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    context.mock.timers.reset();
  });

  const events = await postSse("https://example.test/generate", {}, {}, 256);
  const pending = events.next();
  await new Promise<void>((resolve) => setImmediate(resolve));
  for (let heartbeat = 0; heartbeat < 4; heartbeat++) {
    context.mock.timers.tick(25_000);
    stream?.enqueue(encoder.encode(": keep-alive\n\n"));
    await new Promise<void>((resolve) => setImmediate(resolve));
  }
  context.mock.timers.tick(20_000);

  await assert.rejects(pending, /SSE stream was idle for 120000ms without an event/);
  assert.equal(cancelled, true);
});

test("keeps caller cancellation attached after response headers", async (context) => {
  const previousFetch = globalThis.fetch;
  const control = new AbortController();
  globalThis.fetch = (async (_url, init) => new Response(new ReadableStream<Uint8Array>({
    start(stream) {
      const signal = init?.signal;
      signal?.addEventListener("abort", () => stream.error(signal.reason), { once: true });
    },
  }), { status: 200 })) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
  });

  const events = await postSse("https://example.test/generate", {}, {}, 256, control.signal);
  const pending = events.next();
  control.abort(new Error("cancelled after headers"));

  await assert.rejects(pending, /cancelled after headers/);
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

test("rejects redirects without following or retrying them", async (context) => {
  const previousFetch = globalThis.fetch;
  let calls = 0;
  let redirect: RequestRedirect | undefined;
  globalThis.fetch = (async (_url, init) => {
    calls += 1;
    redirect = init?.redirect;
    return new Response(null, {
      status: 302,
      headers: { location: "https://redirected.example.test/models" },
    });
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
  });

  await assert.rejects(
    getJson("https://example.test/models", { authorization: "Bearer fixture" }),
    (error: Error & { status?: number }) => {
      assert.equal(error.status, 302);
      assert.match(error.message, /redirect rejected/);
      return true;
    },
  );
  assert.equal(redirect, "manual");
  assert.equal(calls, 1);
});

test("an error response cannot accumulate an unbounded body", async (context) => {
  const previousFetch = globalThis.fetch;
  const prefix = "x".repeat(1_999);
  const emoji = String.fromCodePoint(0x1f600);
  globalThis.fetch = (async () =>
    new Response(`${prefix}${emoji}${"x".repeat(20_000)}`, {
      status: 400,
      statusText: "Bad Request",
    })) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
  });

  await assert.rejects(
    getJson("https://example.test/models", {}),
    (error: Error & { body?: string }) => {
      assert.equal(error.body, prefix);
      assert.equal(error.body?.isWellFormed(), true);
      return true;
    },
  );
});

test("a non-JSON preview ends before a grapheme that crosses its boundary", async (context) => {
  const previousFetch = globalThis.fetch;
  const prefix = "x".repeat(499);
  const emoji = String.fromCodePoint(0x1f600);
  globalThis.fetch = (async () => new Response(`${prefix}${emoji}tail`)) as typeof fetch;
  context.after(() => { globalThis.fetch = previousFetch; });

  await assert.rejects(
    getJson("https://example.test/models", {}),
    (error: Error & { body?: string }) => {
      assert.equal(error.body, prefix);
      assert.equal(error.body?.isWellFormed(), true);
      return true;
    },
  );
});
