import { test } from "node:test";
import assert from "node:assert/strict";
import type { StreamEvent } from "../src/types.ts";
import { assembleOpenAI } from "../src/providers/openai-stream.ts";
import {
  fromWireResponse,
  normalizeEffort,
  toWireItems,
  toWireTool,
} from "../src/providers/openai-wire.ts";
import { openai } from "../src/providers/openai.ts";

async function* feed(events: unknown[]): AsyncGenerator<unknown> {
  for (const event of events) yield event;
}

test("takes the final completed response as authoritative", async () => {
  const data = await assembleOpenAI(
    feed([
      { type: "response.output_text.delta", delta: "Hel" },
      { type: "response.output_item.done", item: { type: "message", content: [] } },
      {
        type: "response.completed",
        response: {
          status: "completed",
          output: [
            {
              type: "message",
              content: [{ type: "output_text", text: "Hello" }],
            },
          ],
          usage: {
            input_tokens: 12,
            output_tokens: 5,
            input_tokens_details: { cached_tokens: 3, cache_write_tokens: 2 },
            output_tokens_details: { reasoning_tokens: 1 },
          },
        },
      },
    ]),
  );

  const message = fromWireResponse(data);
  assert.deepEqual(message.content, [{ kind: "text", text: "Hello" }]);
  assert.deepEqual(message.usage, {
    inputTokens: 12,
    outputTokens: 5,
    cachedInputTokens: 3,
    cacheWriteInputTokens: 2,
    reasoningTokens: 1,
  });
});

test("keeps completed stream items when the Codex final envelope has empty output", async () => {
  const data = await assembleOpenAI(
    feed([
      {
        type: "response.output_item.done",
        item: { type: "reasoning", encrypted_content: "opaque" },
      },
      {
        type: "response.output_item.done",
        item: {
          type: "function_call",
          call_id: "call-1",
          name: "list_dir",
          arguments: '{"path":"."}',
        },
      },
      {
        type: "response.completed",
        response: { status: "completed", output: [] },
      },
    ]),
  );

  assert.deepEqual(fromWireResponse(data).content, [
    { kind: "tool_call", id: "call-1", name: "list_dir", input: { path: "." } },
  ]);
  assert.deepEqual(data.output, [
    { type: "reasoning", encrypted_content: "opaque" },
    {
      type: "function_call",
      call_id: "call-1",
      name: "list_dir",
      arguments: '{"path":"."}',
    },
  ]);
});

test("accepts the Codex response.done alias and stops reading after the terminal event", async () => {
  let transportClosed = false;
  async function* heldOpen(): AsyncGenerator<unknown> {
    try {
      yield {
        type: "response.output_item.done",
        item: {
          type: "function_call",
          call_id: "call-1",
          name: "list_dir",
          arguments: "{}",
        },
      };
      yield { type: "response.done", response: { status: "completed", output: [] } };
      await new Promise<never>(() => undefined);
    } finally {
      transportClosed = true;
    }
  }

  const data = await assembleOpenAI(heldOpen());

  assert.equal(transportClosed, true);
  assert.deepEqual(fromWireResponse(data).content, [
    { kind: "tool_call", id: "call-1", name: "list_dir", input: {} },
  ]);
});

test("keeps an incomplete final response and explains why it stopped", async () => {
  const data = await assembleOpenAI(
    feed([
      {
        type: "response.incomplete",
        response: {
          status: "incomplete",
          incomplete_details: { reason: "max_output_tokens" },
          output: [{ type: "message", content: [{ type: "output_text", text: "partial" }] }],
        },
      },
    ]),
  );

  assert.deepEqual(fromWireResponse(data).content, [
    { kind: "text", text: "partial" },
    { kind: "text", text: "[truncated: hit max_output_tokens — raise --max-tokens]" },
  ]);
});

test("streams and preserves refusal text", async () => {
  const streamed: StreamEvent[] = [];
  const data = await assembleOpenAI(
    feed([
      { type: "response.refusal.delta", delta: "I cannot" },
      { type: "response.refusal.delta", delta: " help" },
      {
        type: "response.completed",
        response: {
          output: [{ type: "message", content: [{ type: "refusal", refusal: "I cannot help" }] }],
        },
      },
    ]),
    (event) => streamed.push(event),
  );

  assert.deepEqual(streamed, [
    { kind: "text", text: "[refused] I cannot" },
    { kind: "text", text: " help" },
  ]);
  assert.deepEqual(fromWireResponse(data).content, [
    { kind: "text", text: "[refused] I cannot help" },
  ]);
});

test("uses the error nested in a failed response", async () => {
  await assert.rejects(
    assembleOpenAI(
      feed([
        {
          type: "response.failed",
          response: { status: "failed", error: { code: "server_error", message: "overloaded" } },
        },
      ]),
    ),
    /overloaded/,
  );
});

test("keeps raw reasoning items when echoing OpenAI history", () => {
  const raw = [{ type: "reasoning", encrypted_content: "opaque" }];
  assert.equal(
    toWireItems({ role: "assistant", content: [], raw, rawFrom: "openai" }),
    raw,
  );
});

test("translates normalized messages and tool declarations to Responses items", () => {
  assert.equal(normalizeEffort("max"), "high");
  assert.equal(normalizeEffort("xhigh"), "high");
  assert.equal(normalizeEffort("medium"), "medium");
  assert.deepEqual(
    toWireTool({
      name: "read_file",
      description: "Read one file",
      input: { type: "object", properties: { path: { type: "string" } } },
    }),
    {
      type: "function",
      name: "read_file",
      description: "Read one file",
      parameters: { type: "object", properties: { path: { type: "string" } } },
    },
  );

  assert.deepEqual(
    toWireItems({
      role: "assistant",
      content: [
        { kind: "text", text: "First" },
        { kind: "text", text: "Second" },
        { kind: "tool_call", id: "call-1", name: "read_file", input: { path: "README.md" } },
        { kind: "tool_result", id: "call-0", output: "done", isError: false },
      ],
    }),
    [
      { role: "assistant", content: [{ type: "output_text", text: "First\nSecond" }] },
      {
        type: "function_call",
        call_id: "call-1",
        name: "read_file",
        arguments: '{"path":"README.md"}',
      },
      { type: "function_call_output", call_id: "call-0", output: "done" },
    ],
  );
});

test("normalizes tool calls and sparse usage from a completed response", () => {
  const message = fromWireResponse({
    output: [
      { type: "function_call", call_id: "valid", name: "read_file", arguments: '{"path":"a.ts"}' },
      { type: "function_call", call_id: "empty", name: "list_dir", arguments: "" },
      { type: "function_call", call_id: "broken", name: "edit_file", arguments: "{" },
      { type: "future_item", value: "retained only in raw" },
    ],
    usage: { input_tokens: 4 },
  });

  assert.deepEqual(message.content, [
    { kind: "tool_call", id: "valid", name: "read_file", input: { path: "a.ts" } },
    { kind: "tool_call", id: "empty", name: "list_dir", input: {} },
    { kind: "tool_call", id: "broken", name: "edit_file", input: {} },
  ]);
  assert.deepEqual(message.usage, {
    inputTokens: 4,
    outputTokens: 0,
    cachedInputTokens: 0,
    cacheWriteInputTokens: 0,
    reasoningTokens: 0,
  });
  assert.equal(message.rawFrom, "openai");
  assert.ok(Array.isArray(message.raw));
  assert.equal(message.raw.length, 4);
});

test("sends a stateless Responses request with encrypted reasoning included", async (context) => {
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
    model: "gpt-5",
    system: "be useful",
    messages: [],
    tools: [],
    maxTokens: 100,
    effort: "high",
  });

  assert.equal(requestBody?.store, false);
  assert.deepEqual(requestBody?.include, ["reasoning.encrypted_content"]);
  assert.equal(requestBody?.stream, true);
});
