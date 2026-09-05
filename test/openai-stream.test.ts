import { test } from "node:test";
import assert from "node:assert/strict";
import type { StreamEvent } from "../src/types.ts";
import { assembleOpenAI, openAIStreamProgress } from "../src/providers/openai-stream.ts";
import { fromWireResponse } from "../src/providers/openai-wire.ts";

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
    { kind: "text", text: "[truncated: hit max_output_tokens — raise max output tokens in /settings]" },
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
