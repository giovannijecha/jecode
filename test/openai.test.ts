import { test } from "node:test";
import assert from "node:assert/strict";
import type { StreamEvent } from "../src/types.ts";
import {
  assembleOpenAI,
  openAIStreamProgress,
} from "../src/providers/openai-stream.ts";
import {
  fromWireResponse,
  toWireItems,
  toWireTool,
} from "../src/providers/openai-wire.ts";
import {
  openai,
  openAIContextWindow,
  openAIEfforts,
  supportsOpenAIModel,
} from "../src/providers/openai.ts";

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
  const streamed: StreamEvent[] = [];
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
    (event) => streamed.push(event),
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
  assert.deepEqual(streamed, [{ kind: "tool", name: "list_dir" }]);
});

test("announces one tool phase while Responses arguments stream", async () => {
  const streamed: StreamEvent[] = [];
  const statuses: string[] = [];
  await assembleOpenAI(
    feed([
      { type: "response.created", response: {} },
      { type: "response.in_progress", response: {} },
      { type: "response.reasoning_summary_part.added", summary_index: 0 },
      { type: "response.reasoning_summary_text.delta", delta: "Inspecting" },
      { type: "response.reasoning_summary_text.done", text: "Inspecting" },
      {
        type: "response.output_item.added",
        output_index: 0,
        item: { type: "function_call", id: "fc-1", call_id: "call-1", name: "read_file" },
      },
      {
        type: "response.function_call_arguments.delta",
        item_id: "fc-1",
        output_index: 0,
        delta: '{"path"',
      },
      {
        type: "response.function_call_arguments.done",
        item_id: "fc-1",
        output_index: 0,
        arguments: '{"path":"a.ts"}',
      },
      {
        type: "response.output_item.done",
        output_index: 0,
        item: {
          type: "function_call",
          id: "fc-1",
          call_id: "call-1",
          name: "read_file",
          arguments: '{"path":"a.ts"}',
        },
      },
      { type: "response.completed", response: { status: "completed", output: [] } },
    ]),
    (event) => streamed.push(event),
    (status) => statuses.push(status),
  );

  assert.deepEqual(streamed, [
    { kind: "thinking", text: "Inspecting" },
    { kind: "tool", name: "read_file" },
  ]);
  assert.deepEqual(statuses, [
    "Working",
    "Thinking",
    "Working",
    "Preparing read_file",
  ]);
});

test("leaves the thinking phase when a reasoning item finishes", async () => {
  const statuses: string[] = [];

  await assembleOpenAI(
    feed([
      {
        type: "response.output_item.added",
        item: { type: "reasoning", id: "reasoning-1" },
      },
      {
        type: "response.output_item.done",
        item: { type: "reasoning", id: "reasoning-1", encrypted_content: "opaque" },
      },
      { type: "response.completed", response: { status: "completed", output: [] } },
    ]),
    undefined,
    (status) => statuses.push(status),
  );

  assert.deepEqual(statuses, ["Thinking", "Working"]);
});

test("recognizes only substantive Responses events as stream progress", () => {
  assert.equal(openAIStreamProgress({ type: "response.in_progress" }), false);
  assert.equal(openAIStreamProgress({ type: "ping" }), false);
  assert.equal(openAIStreamProgress({ type: "response.reasoning_summary_text.delta", delta: "x" }), true);
  assert.equal(openAIStreamProgress({ type: "response.function_call_arguments.delta", delta: "{" }), true);
  assert.equal(openAIStreamProgress({ type: "response.completed", response: {} }), true);
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
          incomplete_details: { reason: "max_output_tokens" },
          output: [
            { type: "message", content: [{ type: "output_text", text: "partial" }] },
            {
              type: "function_call",
              status: "incomplete",
              call_id: "partial-call",
              name: "write_file",
              arguments: '{"path":"partial',
            },
          ],
        },
      },
    ]),
  );

  const message = fromWireResponse(data);
  assert.equal(data.status, "incomplete");
  assert.deepEqual(message.content, [
    { kind: "text", text: "partial" },
    { kind: "text", text: "[truncated: hit max_output_tokens — raise --max-tokens]" },
  ]);
  assert.equal(message.raw, undefined);
  assert.equal(message.rawFrom, undefined);
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

test("rejects a stream that ends before a terminal response event", async () => {
  await assert.rejects(
    assembleOpenAI(feed([
      {
        type: "response.output_item.done",
        item: {
          type: "function_call",
          call_id: "call-1",
          name: "write_file",
          arguments: '{"path":"partial.txt","content":"unsafe"}',
        },
      },
    ])),
    /ended before a terminal response event/,
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

test("orders steering after completed function outputs", () => {
  const items = [
    {
      role: "user" as const,
      content: [{ kind: "tool_result" as const, id: "call-1", output: "done", isError: false }],
    },
    {
      role: "user" as const,
      content: [{ kind: "text" as const, text: "change direction" }],
    },
  ].flatMap((message) => toWireItems(message));

  assert.deepEqual(items, [
    { type: "function_call_output", call_id: "call-1", output: "done" },
    { role: "user", content: [{ type: "input_text", text: "change direction" }] },
  ]);
});

test("offers only reasoning-capable Responses models and keeps their real effort levels", () => {
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
  assert.deepEqual(openAIEfforts("gpt-5.4"), ["low", "medium", "high", "xhigh"]);
  assert.deepEqual(openAIEfforts("gpt-5"), ["low", "medium", "high"]);
  assert.deepEqual(openAIEfforts("gpt-5-pro"), ["high"]);
  assert.deepEqual(openAIEfforts("gpt-5.4-pro"), ["medium", "high", "xhigh"]);
  assert.deepEqual(openAIEfforts("gpt-5-chat-latest"), []);
  assert.deepEqual(openAIEfforts("o1-mini"), []);
  assert.deepEqual(openAIEfforts("gpt-5.5-pro"), []);
  assert.deepEqual(openAIContextWindow("gpt-5.6-sol"), { tokens: 997_500 });
  assert.deepEqual(openAIContextWindow("gpt-5"), { tokens: 380_000 });
  assert.deepEqual(openAIContextWindow("o4-mini"), { tokens: 190_000 });
});

test("normalizes tool calls and sparse usage from a completed response", () => {
  const message = fromWireResponse({
    status: "completed",
    output: [
      {
        type: "function_call",
        status: "completed",
        call_id: "valid",
        name: "read_file",
        arguments: '{"path":"a.ts"}',
      },
      { type: "function_call", call_id: "empty", name: "list_dir", arguments: "" },
      { type: "function_call", call_id: "broken", name: "edit_file", arguments: "{" },
      { type: "function_call", call_id: "array", name: "list_dir", arguments: "[]" },
      { type: "function_call", call_id: "scalar", name: "list_dir", arguments: "42" },
      { type: "future_item", value: "retained only in raw" },
    ],
    usage: { input_tokens: 4 },
  });

  assert.deepEqual(message.content, [
    { kind: "tool_call", id: "valid", name: "read_file", input: { path: "a.ts" } },
    { kind: "tool_call", id: "empty", name: "list_dir", input: {} },
    {
      kind: "tool_call",
      id: "broken",
      name: "edit_file",
      input: {},
      inputError: "tool arguments were not valid JSON",
    },
    {
      kind: "tool_call",
      id: "array",
      name: "list_dir",
      input: {},
      inputError: "tool arguments must be a JSON object",
    },
    {
      kind: "tool_call",
      id: "scalar",
      name: "list_dir",
      input: {},
      inputError: "tool arguments must be a JSON object",
    },
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
  assert.equal(message.raw.length, 6);
});

test("rejects an incomplete call item and its unsafe continuation payload", () => {
  const message = fromWireResponse({
    status: "completed",
    output: [
      { type: "message", content: [{ type: "output_text", text: "partial" }] },
      {
        type: "function_call",
        status: "incomplete",
        call_id: "partial",
        name: "write_file",
        arguments: '{"path":"partial',
      },
    ],
  });

  assert.deepEqual(message.content, [{ kind: "text", text: "partial" }]);
  assert.equal(message.raw, undefined);
  assert.equal(message.rawFrom, undefined);
});

test("explains an incomplete response even when the provider omits its reason", () => {
  const message = fromWireResponse({
    status: "incomplete",
    incomplete_details: null,
    output: [{
      type: "function_call",
      status: "incomplete",
      call_id: "partial",
      name: "write_file",
      arguments: "{",
    }],
  });

  assert.deepEqual(message.content, [{ kind: "text", text: "[incomplete response]" }]);
  assert.equal(message.raw, undefined);
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
