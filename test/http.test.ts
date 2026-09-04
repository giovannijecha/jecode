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

test("retries one rejected SSE generation after the provider delay", async (context) => {
  const previousFetch = globalThis.fetch;
  const statuses: string[] = [];
  let calls = 0;
  context.mock.timers.enable({ apis: ["setTimeout"] });

  globalThis.fetch = (async () => {
    calls += 1;
    return calls === 1
      ? new Response(
        JSON.stringify({
          error: { message: "Rate limit reached. Please try again in 6.386s." },
        }),
        { status: 429, statusText: "Too Many Requests" },
      )
      : new Response('data: {"type":"done"}\n\n', { status: 200 });
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    context.mock.timers.reset();
  });

  const pending = postSse(
    "https://example.test/generate",
    {},
    { prompt: "hello" },
    256,
    undefined,
    (status) => statuses.push(status),
  );
  await new Promise<void>((resolve) => setImmediate(resolve));

  assert.deepEqual(statuses, ["Connecting", "Rate limited · retrying in 7s"]);
  context.mock.timers.tick(6_386);
  const events = await pending;
  const received: unknown[] = [];
  for await (const event of events) received.push(event);

  assert.equal(calls, 2);
  assert.deepEqual(received, [{ type: "done" }]);
  assert.deepEqual(statuses, [
    "Connecting",
    "Rate limited · retrying in 7s",
    "Connecting",
    "Waiting for model",
  ]);
});

test("bounds an SSE generation to one rate-limit retry", async (context) => {
  const previousFetch = globalThis.fetch;
  const statuses: string[] = [];
  let calls = 0;

  globalThis.fetch = (async () => {
    calls += 1;
    return new Response("Please try again in 0s.", {
      status: 429,
      statusText: "Too Many Requests",
    });
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
  });

  await assert.rejects(
    postSse(
      "https://example.test/generate",
      {},
      { prompt: "hello" },
      256,
      undefined,
      (status) => statuses.push(status),
    ),
    (error: Error & { status?: number }) => {
      assert.equal(error.status, 429);
      return true;
    },
  );
  assert.equal(calls, 2);
  assert.deepEqual(statuses, [
    "Connecting",
    "Rate limited · retrying in 0ms",
    "Connecting",
  ]);
});

test("does not retry an SSE rate limit without a provider delay", async (context) => {
  const previousFetch = globalThis.fetch;
  let calls = 0;

  globalThis.fetch = (async () => {
    calls += 1;
    return new Response("usage quota exhausted", { status: 429 });
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
  });

  await assert.rejects(
    postSse("https://example.test/generate", {}, { prompt: "hello" }, 256),
    (error: Error & { status?: number }) => {
      assert.equal(error.status, 429);
      return true;
    },
  );
  assert.equal(calls, 1);
});

test("an abort stops a pending SSE rate-limit retry", async (context) => {
  const previousFetch = globalThis.fetch;
  const control = new AbortController();
  const statuses: string[] = [];
  let calls = 0;

  globalThis.fetch = (async () => {
    calls += 1;
    return new Response("busy", { status: 429, headers: { "retry-after": "999999" } });
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
  });

  const pending = postSse(
    "https://example.test/generate",
    {},
    { prompt: "hello" },
    256,
    control.signal,
    (status) => {
      statuses.push(status);
      if (status.startsWith("Rate limited")) control.abort(new Error("cancelled"));
    },
  );

  await assert.rejects(pending, /cancelled/);
  assert.equal(calls, 1);
  assert.deepEqual(statuses, ["Connecting", "Rate limited · retrying in 60s"]);
});

test("does not replay an SSE generation after a server failure", async (context) => {
  const previousFetch = globalThis.fetch;
  let calls = 0;

  globalThis.fetch = (async () => {
    calls += 1;
    return new Response("busy", { status: 503, statusText: "Unavailable" });
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
  });

  await assert.rejects(
    postSse("https://example.test/generate", {}, { prompt: "hello" }, 256),
    (error: Error & { status?: number }) => {
      assert.equal(error.status, 503);
      return true;
    },
  );
  assert.equal(calls, 1);
});

test("does not replay an SSE generation after an ambiguous network error", async (context) => {
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
    postSse("https://example.test/generate", {}, { prompt: "hello" }, 256),
    /network error calling .*socket reset/,
  );
  assert.equal(calls, 1);
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

test("does not replay a JSON POST after a retryable response", async (context) => {
  const previousFetch = globalThis.fetch;
  let calls = 0;
  const statuses: string[] = [];
  let responseStatus = 429;
  globalThis.fetch = (async () => {
    calls += 1;
    return new Response("busy", { status: responseStatus });
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
  });

  for (const status of [429, 503]) {
    responseStatus = status;
    await assert.rejects(
      postJson("https://example.test/generate", {}, { prompt: "hello" }, undefined, (text) => {
        statuses.push(text);
      }),
      (error: Error & { status?: number }) => {
        assert.equal(error.status, status);
        return true;
      },
    );
  }
  assert.equal(calls, 2);
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

test("complete non-progress events cannot keep a model stream alive", async (context) => {
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

  const events = await postSse(
    "https://example.test/generate",
    {},
    {},
    256,
    undefined,
    undefined,
    () => false,
  );
  const pending = (async () => {
    for await (const _event of events) {
      // Keep consuming protocol events while none count as model progress.
    }
  })();
  await new Promise<void>((resolve) => setImmediate(resolve));

  for (let heartbeat = 0; heartbeat < 5; heartbeat++) {
    context.mock.timers.tick(50_000);
    stream?.enqueue(encoder.encode('data: {"type":"response.in_progress"}\n\n'));
    await new Promise<void>((resolve) => setImmediate(resolve));
  }
  context.mock.timers.tick(50_000);

  await assert.rejects(pending, /made no model progress for 300000ms/);
  assert.equal(cancelled, true);
});

test("substantive progress resets the model stream deadline", async (context) => {
  const previousFetch = globalThis.fetch;
  const encoder = new TextEncoder();
  let stream: ReadableStreamDefaultController<Uint8Array> | undefined;
  context.mock.timers.enable({ apis: ["setTimeout"] });
  globalThis.fetch = (async () => new Response(new ReadableStream<Uint8Array>({
    start(controller) {
      stream = controller;
    },
  }), { status: 200 })) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    context.mock.timers.reset();
  });

  const events = await postSse(
    "https://example.test/generate",
    {},
    {},
    256,
    undefined,
    undefined,
    (event) => (event as { type?: string }).type === "response.output_text.delta",
  );
  let seen = 0;
  const pending = (async () => {
    for await (const _event of events) seen++;
  })();
  await new Promise<void>((resolve) => setImmediate(resolve));

  for (let heartbeat = 0; heartbeat < 4; heartbeat++) {
    context.mock.timers.tick(50_000);
    stream?.enqueue(encoder.encode('data: {"type":"response.in_progress"}\n\n'));
    await new Promise<void>((resolve) => setImmediate(resolve));
  }
  context.mock.timers.tick(50_000);
  stream?.enqueue(encoder.encode('data: {"type":"response.output_text.delta"}\n\n'));
  await new Promise<void>((resolve) => setImmediate(resolve));

  for (let heartbeat = 0; heartbeat < 5; heartbeat++) {
    context.mock.timers.tick(50_000);
    stream?.enqueue(encoder.encode('data: {"type":"response.in_progress"}\n\n'));
    await new Promise<void>((resolve) => setImmediate(resolve));
  }
  stream?.close();

  await pending;
  assert.equal(seen, 10);
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
