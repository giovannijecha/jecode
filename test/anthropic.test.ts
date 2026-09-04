import { test } from "node:test";
import assert from "node:assert/strict";
import {
  anthropic,
  anthropicEfforts,
  supportsAdaptiveThinking,
} from "../src/providers/anthropic.ts";

test("Anthropic effort follows the selected model independently from adaptive thinking", () => {
  assert.deepEqual(anthropicEfforts("claude-sonnet-4-6"), ["low", "medium", "high", "max"]);
  assert.deepEqual(anthropicEfforts("claude-opus-4-5"), ["low", "medium", "high"]);
  assert.deepEqual(
    anthropicEfforts("claude-opus-5"),
    ["low", "medium", "high", "xhigh", "max"],
  );
  assert.deepEqual(
    anthropicEfforts("claude-mythos-5"),
    ["low", "medium", "high", "xhigh", "max"],
  );
  assert.deepEqual(
    anthropicEfforts("claude-mythos-preview"),
    ["low", "medium", "high", "max"],
  );
  assert.equal(supportsAdaptiveThinking("claude-mythos-preview"), true);
  assert.equal(supportsAdaptiveThinking("claude-opus-4-5"), false);
});

test("Anthropic retains live input capacity from its model catalogue", async (context) => {
  const previousFetch = globalThis.fetch;
  const previousKey = process.env.ANTHROPIC_API_KEY;
  process.env.ANTHROPIC_API_KEY = "test-key";
  globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
    assert.equal(new Headers(init?.headers).get("x-api-key"), "test-key");
    return new Response(JSON.stringify({
      data: [{ id: "claude-context-fixture", max_input_tokens: 1_000_000 }],
    }), { status: 200, headers: { "content-type": "application/json" } });
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    if (previousKey === undefined) delete process.env.ANTHROPIC_API_KEY;
    else process.env.ANTHROPIC_API_KEY = previousKey;
  });

  assert.deepEqual(await anthropic.models(), ["claude-context-fixture"]);
  assert.deepEqual(
    await anthropic.contextWindow?.("claude-context-fixture"),
    { tokens: 1_000_000 },
  );
});

test("Anthropic rejects an unsupported effort before making a request", async (context) => {
  const previousFetch = globalThis.fetch;
  const previousKey = process.env.ANTHROPIC_API_KEY;
  let requests = 0;
  process.env.ANTHROPIC_API_KEY = "test-key";
  globalThis.fetch = (async () => {
    requests++;
    throw new Error("request should not be made");
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    if (previousKey === undefined) delete process.env.ANTHROPIC_API_KEY;
    else process.env.ANTHROPIC_API_KEY = previousKey;
  });

  await assert.rejects(
    anthropic.send({
      model: "claude-sonnet-4-6",
      system: "be useful",
      messages: [],
      tools: [],
      maxTokens: 100,
      effort: "xhigh",
    }),
    /does not support effort "xhigh"/,
  );
  assert.equal(requests, 0);
});

test("Anthropic can send effort without enabling adaptive thinking", async (context) => {
  const previousFetch = globalThis.fetch;
  const previousKey = process.env.ANTHROPIC_API_KEY;
  let body: Record<string, unknown> = {};
  process.env.ANTHROPIC_API_KEY = "test-key";
  globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
    body = JSON.parse(String(init?.body)) as Record<string, unknown>;
    return new Response("data: {\"type\":\"message_stop\"}\n\n", {
      status: 200,
      headers: { "content-type": "text/event-stream" },
    });
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    if (previousKey === undefined) delete process.env.ANTHROPIC_API_KEY;
    else process.env.ANTHROPIC_API_KEY = previousKey;
  });

  await anthropic.send({
    model: "claude-opus-4-5",
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

  assert.equal(body["thinking"], undefined);
  assert.deepEqual(body["output_config"], { effort: "high" });
  assert.deepEqual(body["cache_control"], { type: "ephemeral" });
});

test("Anthropic omits automatic cache creation from compaction requests", async (context) => {
  const previousFetch = globalThis.fetch;
  const previousKey = process.env.ANTHROPIC_API_KEY;
  let body: Record<string, unknown> = {};
  process.env.ANTHROPIC_API_KEY = "test-key";
  globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
    body = JSON.parse(String(init?.body)) as Record<string, unknown>;
    return new Response("data: {\"type\":\"message_stop\"}\n\n", {
      status: 200,
      headers: { "content-type": "text/event-stream" },
    });
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    if (previousKey === undefined) delete process.env.ANTHROPIC_API_KEY;
    else process.env.ANTHROPIC_API_KEY = previousKey;
  });

  await anthropic.send({
    model: "claude-opus-4-5",
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

  assert.equal(body["cache_control"], undefined);
});

test("Anthropic enables adaptive thinking and max effort for Mythos Preview", async (context) => {
  const previousFetch = globalThis.fetch;
  const previousKey = process.env.ANTHROPIC_API_KEY;
  let body: Record<string, unknown> = {};
  process.env.ANTHROPIC_API_KEY = "test-key";
  globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
    body = JSON.parse(String(init?.body)) as Record<string, unknown>;
    return new Response("data: {\"type\":\"message_stop\"}\n\n", {
      status: 200,
      headers: { "content-type": "text/event-stream" },
    });
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    if (previousKey === undefined) delete process.env.ANTHROPIC_API_KEY;
    else process.env.ANTHROPIC_API_KEY = previousKey;
  });

  await anthropic.send({
    model: "claude-mythos-preview",
    system: "be useful",
    messages: [],
    tools: [],
    maxTokens: 100,
    effort: "max",
  });

  assert.deepEqual(body["thinking"], { type: "adaptive", display: "summarized" });
  assert.deepEqual(body["output_config"], { effort: "max" });
});
