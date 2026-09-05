import { test } from "node:test";
import assert from "node:assert/strict";
import type { Message, Provider, RequestInput, Usage } from "../src/types.ts";
import { anthropic } from "../src/providers/anthropic.ts";
import { openai } from "../src/providers/openai.ts";
import { openaiCodex } from "../src/providers/openai-codex.ts";
import { ollama } from "../src/providers/ollama.ts";

const PROVIDERS = [anthropic, openai, openaiCodex, ollama];
const OPAQUE_PROVIDERS = [anthropic, openai, openaiCodex];

test("every provider measures system text, tools, and Unicode input without network access", async (context) => {
  const previousFetch = globalThis.fetch;
  globalThis.fetch = async () => { throw new Error("measurement must stay local"); };
  context.after(() => { globalThis.fetch = previousFetch; });

  for (const provider of PROVIDERS) {
    const baseline = await measure(provider, input([]));
    const request = input([{ role: "user", content: [{ kind: "text", text: "世界 👩🏽‍💻 ".repeat(100) }] }]);
    const text = await measure(provider, request);
    assert.ok(text > baseline + 100, provider.id);
    request.system = "system instructions ".repeat(100);
    const system = await measure(provider, request);
    assert.ok(system > text + 100, provider.id);
    request.tools.push({ name: "read", description: "tool description ".repeat(100), input: { type: "object" } });
    assert.ok(await measure(provider, request) > system + 100, provider.id);
  }
});

test("raw response text and arguments replace the normalized copy when measuring input", async () => {
  for (const provider of OPAQUE_PROVIDERS) {
    const raw = provider.id === "anthropic"
      ? [{ type: "text", text: "sent once" }, { type: "tool_use", id: "call", name: "read", input: { path: "file" } }]
      : [{ type: "message", role: "assistant", content: [{ type: "output_text", text: "sent once" }] },
        { type: "function_call", call_id: "call", name: "read", arguments: '{"path":"file"}' }];
    const message: Message = { role: "assistant", content: [], rawFrom: provider.id, raw };
    const expected = await measure(provider, input([message]));
    message.content = [{ kind: "text", text: "local copy ".repeat(10_000) }, {
      kind: "tool_call", id: "call", name: "read", input: { path: "another local copy ".repeat(10_000) },
    }];
    message.usage = usage(1_000_000);
    assert.equal(await measure(provider, input([message])), expected, provider.id);
  }
});

test("foreign raw and usage are ignored instead of entering another provider's input", async () => {
  for (const provider of PROVIDERS) {
    const message: Message = { role: "assistant", content: [{ kind: "text", text: "normalized answer" }] };
    const expected = await measure(provider, input([message]));
    message.rawFrom = "different-provider";
    message.raw = [{ type: "reasoning", encrypted_content: "opaque".repeat(20_000) }];
    message.usage = usage(1_000_000);
    assert.equal(await measure(provider, input([message])), expected, provider.id);
  }
});

test("own opaque reasoning reserves reported output once and leaves all raw evidence intact", async () => {
  for (const provider of OPAQUE_PROVIDERS) {
    const message = opaqueMessage(provider, "opaque".repeat(20_000), usage(10_000));
    const original = structuredClone(message);
    const measured = await measure(provider, input([message]));
    assert.ok(measured >= 10_000 && measured < 10_500, `${provider.id}: ${measured}`);
    assert.deepEqual(message, original, provider.id);
    const shorter = opaqueMessage(provider, "opaque", usage(10_000));
    assert.equal(await measure(provider, input([shorter])), measured, provider.id);
  }
});

test("missing and invalid opaque usage keep the conservative raw estimate", async () => {
  for (const provider of OPAQUE_PROVIDERS) {
    const raw = "opaque".repeat(20_000);
    const missing = opaqueMessage(provider, raw);
    const expected = await measure(provider, input([missing]));
    assert.ok(expected > 40_000, `${provider.id}: ${expected}`);
    for (const count of [0, -1, 1.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1]) {
      const message = opaqueMessage(provider, raw, usage(count));
      assert.equal(await measure(provider, input([message])), expected, `${provider.id}: ${count}`);
    }
  }
});

test("unknown raw fields remain measured even alongside recognized opaque reasoning", async () => {
  for (const provider of OPAQUE_PROVIDERS) {
    const message = opaqueMessage(provider, "opaque", usage(1));
    const baseline = await measure(provider, input([message]));
    (message.raw as unknown[]).push({ type: "future-item", data: "unknown content ".repeat(10_000) });
    assert.ok(await measure(provider, input([message])) > baseline + 10_000, provider.id);
  }
});

test("Ollama measures its replayed reasoning while omitting unsent raw properties", async () => {
  const message: Message = {
    role: "assistant", content: [{ kind: "tool_call", id: "call", name: "read", input: {} }],
    rawFrom: "ollama", raw: { reasoning: "inspect the file ".repeat(1_000) },
  };
  const measured = await measure(ollama, input([message]));
  (message.raw as Record<string, unknown>)["unused"] = "not sent ".repeat(10_000);
  message.usage = usage(1_000_000);
  assert.equal(await measure(ollama, input([message])), measured);
  message.raw = undefined;
  assert.ok(await measure(ollama, input([message])) < measured - 1_000);
});

test("provider measurement observes preexisting and in-progress cancellation", async () => {
  for (const provider of PROVIDERS) {
    const stopped = new AbortController();
    stopped.abort(new Error("already cancelled"));
    await assert.rejects(measure(provider, input([]), stopped.signal), /already cancelled/);

    const active = new AbortController();
    const request = input([{ role: "user", content: [{ kind: "text", text: "界".repeat(1_000_000) }] }]);
    setImmediate(() => active.abort(new Error("cancelled while measuring")));
    await assert.rejects(measure(provider, request, active.signal), /cancelled while measuring/);
  }
});

function measure(provider: Provider, request: RequestInput, signal?: AbortSignal): Promise<number> {
  assert.ok(provider.measureInput, provider.id);
  return provider.measureInput(request, signal);
}

function input(messages: Message[]): RequestInput {
  return { model: "fixture-model", effort: "high", system: "", messages, tools: [] };
}

function usage(outputTokens: number): Usage {
  return {
    inputTokens: 50_000, outputTokens, reasoningTokens: Math.max(0, outputTokens - 1),
    cachedInputTokens: 20_000, cacheWriteInputTokens: 10_000,
  };
}

function opaqueMessage(provider: Provider, opaque: string, reported?: Usage): Message {
  return {
    role: "assistant", content: [{ kind: "text", text: "answer" }], rawFrom: provider.id,
    raw: provider.id === "anthropic"
      ? [{ type: "thinking", thinking: "summary", signature: opaque },
        { type: "redacted_thinking", data: opaque }, { type: "text", text: "answer" }]
      : [{ type: "reasoning", summary: [], encrypted_content: opaque },
        { type: "reasoning", summary: [], encrypted_content: opaque },
        { type: "message", role: "assistant", content: [{ type: "output_text", text: "answer" }] }],
    ...(reported === undefined ? {} : { usage: reported }),
  };
}
