import { test } from "node:test";
import assert from "node:assert/strict";
import { openai, openAIContextWindow, openAIEfforts, supportsOpenAIModel } from "../src/providers/openai.ts";

test("offers only reasoning-capable Responses models and keeps their real effort levels", () => {
  assert.equal(supportsOpenAIModel("gpt-6-astra"), true);
  assert.equal(supportsOpenAIModel("gpt-6-astra-2026-09-03"), true);
  assert.equal(supportsOpenAIModel("gpt-6-other"), false);
  assert.equal(supportsOpenAIModel("gpt-5.6-sol"), true);
  assert.equal(supportsOpenAIModel("o4-mini"), true);
  assert.equal(supportsOpenAIModel("gpt-5.4-pro"), true);
  assert.equal(supportsOpenAIModel("gpt-5-chat-latest"), false);
  assert.equal(supportsOpenAIModel("gpt-5.1-chat-latest"), false);
  assert.equal(supportsOpenAIModel("gpt-5.2-chat-latest"), false);
  assert.equal(supportsOpenAIModel("gpt-5.3-chat-latest"), false);
  assert.equal(supportsOpenAIModel("gpt-5.5-pro"), false);
  assert.equal(supportsOpenAIModel("o1-mini"), false);
  assert.equal(supportsOpenAIModel("o1-pro"), false);
  assert.equal(supportsOpenAIModel("o3-pro"), false);
  assert.equal(supportsOpenAIModel("o3-deep-research"), false);
  assert.equal(supportsOpenAIModel("o4-mini-deep-research"), false);
  assert.equal(supportsOpenAIModel("gpt-4.1"), false);
  assert.equal(supportsOpenAIModel("chatgpt-image-latest"), false);
  assert.deepEqual(openAIEfforts("gpt-5.6-sol"), ["low", "medium", "high", "xhigh", "max"]);
  assert.deepEqual(openAIEfforts("gpt-6-astra"), ["low", "medium", "high", "xhigh", "max"]);
  assert.deepEqual(openAIEfforts("gpt-5.4"), ["low", "medium", "high", "xhigh"]);
  assert.deepEqual(openAIEfforts("gpt-5"), ["low", "medium", "high"]);
  assert.deepEqual(openAIEfforts("gpt-5-pro"), ["high"]);
  assert.deepEqual(openAIEfforts("gpt-5.4-pro"), ["medium", "high", "xhigh"]);
  assert.deepEqual(openAIEfforts("gpt-5-chat-latest"), []);
  assert.deepEqual(openAIEfforts("o1-mini"), []);
  assert.deepEqual(openAIEfforts("gpt-5.5-pro"), []);
  assert.deepEqual(openAIContextWindow("gpt-5.6-sol"), { tokens: 997_500 });
  assert.deepEqual(openAIContextWindow("gpt-6-astra"), { tokens: 997_500 });
  assert.deepEqual(openAIContextWindow("gpt-5"), { tokens: 380_000 });
  assert.deepEqual(openAIContextWindow("o4-mini"), { tokens: 190_000 });
});

test("sends a stateless Responses request with encrypted reasoning included", async (context) => {
  const previousFetch = globalThis.fetch;
  const previousKey = process.env.OPENAI_API_KEY;
  let requestBody: Record<string, unknown> | undefined;
  let requestHeaders = new Headers();
  process.env.OPENAI_API_KEY = "test-key";

  globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
    requestBody = JSON.parse(String(init?.body)) as Record<string, unknown>;
    requestHeaders = new Headers(init?.headers);
    return new Response(
      'data: {"type":"response.completed","response":{"status":"completed","output":[]}}\n\n',
      { status: 200, headers: { "content-type": "text/event-stream" } },
    );
  }) as typeof fetch;

  context.after(() => {
    globalThis.fetch = previousFetch;
    if (previousKey === undefined) delete process.env.OPENAI_API_KEY;
    else process.env.OPENAI_API_KEY = previousKey;
  });

  await openai.send({
    model: "gpt-5",
    system: "be useful",
    messages: [],
    tools: [],
    maxTokens: 100,
    effort: "high",
    identity: {
      conversationId: "11111111-1111-4111-8111-111111111111",
      cacheKey: "jecode-stable-cache",
      purpose: "turn",
    },
  });

  assert.equal(requestBody?.store, false);
  assert.deepEqual(requestBody?.include, ["reasoning.encrypted_content"]);
  assert.equal(requestBody?.stream, true);
  assert.equal(requestBody?.prompt_cache_key, "jecode-stable-cache");
  assert.match(requestHeaders.get("user-agent") ?? "", /^jecode\//);
  assert.match(
    requestHeaders.get("x-client-request-id") ?? "",
    /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u,
  );
});

test("omits prompt cache routing from compaction requests", async (context) => {
  const previousFetch = globalThis.fetch;
  const previousKey = process.env.OPENAI_API_KEY;
  let requestBody: Record<string, unknown> = {};
  process.env.OPENAI_API_KEY = "test-key";
  globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
    requestBody = JSON.parse(String(init?.body)) as Record<string, unknown>;
    return new Response(
      'data: {"type":"response.completed","response":{"status":"completed","output":[]}}\n\n',
      { status: 200, headers: { "content-type": "text/event-stream" } },
    );
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    if (previousKey === undefined) delete process.env.OPENAI_API_KEY;
    else process.env.OPENAI_API_KEY = previousKey;
  });

  await openai.send({
    model: "gpt-5",
    system: "summarize",
    messages: [],
    tools: [],
    maxTokens: 100,
    effort: "high",
    identity: {
      conversationId: "11111111-1111-4111-8111-111111111111",
      cacheKey: "jecode-stable-cache",
      purpose: "compaction",
    },
  });

  assert.equal(requestBody["prompt_cache_key"], undefined);
});

test("passes max effort through to a model that supports it", async (context) => {
  const previousFetch = globalThis.fetch;
  const previousKey = process.env.OPENAI_API_KEY;
  let requestBody: Record<string, unknown> | undefined;
  process.env.OPENAI_API_KEY = "test-key";
  globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
    requestBody = JSON.parse(String(init?.body)) as Record<string, unknown>;
    return new Response(
      'data: {"type":"response.completed","response":{"status":"completed","output":[]}}\n\n',
      { status: 200, headers: { "content-type": "text/event-stream" } },
    );
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    if (previousKey === undefined) delete process.env.OPENAI_API_KEY;
    else process.env.OPENAI_API_KEY = previousKey;
  });

  await openai.send({
    model: "gpt-5.6-sol",
    system: "be useful",
    messages: [],
    tools: [],
    maxTokens: 100,
    effort: "max",
  });

  assert.deepEqual(requestBody?.reasoning, { effort: "max", summary: "auto" });
});

test("normalizes an in-stream credit exhaustion as a hard billing failure", async (context) => {
  const previousFetch = globalThis.fetch;
  const previousKey = process.env.OPENAI_API_KEY;
  let calls = 0;
  process.env.OPENAI_API_KEY = "test-key";
  globalThis.fetch = (async () => {
    calls += 1;
    return new Response(
      `data: ${JSON.stringify({
        type: "response.failed",
        response: {
          status: "failed",
          error: {
            code: "billing_hard_limit_reached",
            message: "You have no credits remaining.",
          },
        },
      })}\n\n`,
      { status: 200, headers: { "content-type": "text/event-stream" } },
    );
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    if (previousKey === undefined) delete process.env.OPENAI_API_KEY;
    else process.env.OPENAI_API_KEY = previousKey;
  });

  await assert.rejects(
    openai.send({
      model: "gpt-5",
      system: "be useful",
      messages: [],
      tools: [],
      maxTokens: 100,
      effort: "high",
    }),
    (error: Error & { kind?: string; providerId?: string }) => {
      assert.equal(error.kind, "billing");
      assert.equal(error.providerId, "openai");
      return true;
    },
  );
  assert.equal(calls, 1);
});
