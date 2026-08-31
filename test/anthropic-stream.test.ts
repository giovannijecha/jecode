import { test } from "node:test";
import assert from "node:assert/strict";
import type { StreamEvent } from "../src/types.ts";
import { assembleAnthropic } from "../src/providers/anthropic-stream.ts";
import { fromWireResponse } from "../src/providers/anthropic-wire.ts";
import { MAX_TOOL_ARGUMENT_CHARS } from "../src/providers/stream-limits.ts";

async function* feed(events: unknown[]): AsyncGenerator<unknown> {
  for (const event of events) yield event;
}

function sink(): { events: StreamEvent[]; push: (event: StreamEvent) => void } {
  const events: StreamEvent[] = [];
  return { events, push: (event) => events.push(event) };
}

test("accumulates text deltas into one block and streams them as they land", async () => {
  const seen = sink();
  const data = await assembleAnthropic(
    feed([
      { type: "message_start", message: {} },
      { type: "content_block_start", index: 0, content_block: { type: "text", text: "" } },
      { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "Hel" } },
      { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "lo" } },
      { type: "content_block_stop", index: 0 },
      { type: "message_delta", delta: { stop_reason: "end_turn" } },
      { type: "message_stop" },
    ]),
    seen.push,
  );

  assert.deepEqual(data.content, [{ type: "text", text: "Hello" }]);
  assert.deepEqual(seen.events, [
    { kind: "text", text: "Hel" },
    { kind: "text", text: "lo" },
  ]);
});

test("rebuilds tool arguments from streamed JSON fragments", async () => {
  const data = await assembleAnthropic(
    feed([
      {
        type: "content_block_start",
        index: 0,
        content_block: { type: "tool_use", id: "tu_1", name: "read_file", input: {} },
      },
      { type: "content_block_delta", index: 0, delta: { type: "input_json_delta", partial_json: '{"pa' } },
      { type: "content_block_delta", index: 0, delta: { type: "input_json_delta", partial_json: 'th":"a.ts"}' } },
      { type: "content_block_stop", index: 0 },
      { type: "message_delta", delta: { stop_reason: "tool_use" } },
      { type: "message_stop" },
    ]),
  );

  const message = fromWireResponse(data);
  assert.deepEqual(message.content, [
    { kind: "tool_call", id: "tu_1", name: "read_file", input: { path: "a.ts" } },
  ]);
});

test("bounds tool arguments accumulated across Anthropic events", async () => {
  await assert.rejects(
    assembleAnthropic(
      feed([
        {
          type: "content_block_start",
          index: 0,
          content_block: { type: "tool_use", id: "tu_1", name: "read_file", input: {} },
        },
        {
          type: "content_block_delta",
          index: 0,
          delta: {
            type: "input_json_delta",
            partial_json: "x".repeat(MAX_TOOL_ARGUMENT_CHARS),
          },
        },
        {
          type: "content_block_delta",
          index: 0,
          delta: { type: "input_json_delta", partial_json: "x" },
        },
      ]),
    ),
    /streamed tool arguments exceeded/,
  );
});

test("treats an empty argument stream as a call with no arguments", async () => {
  const data = await assembleAnthropic(
    feed([
      {
        type: "content_block_start",
        index: 0,
        content_block: { type: "tool_use", id: "tu_1", name: "list_dir", input: {} },
      },
      { type: "content_block_delta", index: 0, delta: { type: "input_json_delta", partial_json: "" } },
      { type: "content_block_stop", index: 0 },
      { type: "message_stop" },
    ]),
  );

  const call = fromWireResponse(data).content[0];
  assert.equal(call?.kind === "tool_call" && call.name, "list_dir");
  assert.deepEqual(call?.kind === "tool_call" ? call.input : undefined, {});
});

test("keeps thinking text and its signature in the raw block for echo-back", async () => {
  const seen = sink();
  const data = await assembleAnthropic(
    feed([
      { type: "content_block_start", index: 0, content_block: { type: "thinking", thinking: "" } },
      { type: "content_block_delta", index: 0, delta: { type: "thinking_delta", thinking: "hm" } },
      { type: "content_block_delta", index: 0, delta: { type: "signature_delta", signature: "sig-1" } },
      { type: "content_block_stop", index: 0 },
      { type: "content_block_start", index: 1, content_block: { type: "text", text: "" } },
      { type: "content_block_delta", index: 1, delta: { type: "text_delta", text: "done" } },
      { type: "content_block_stop", index: 1 },
      { type: "message_stop" },
    ]),
    seen.push,
  );

  assert.deepEqual(data.content, [
    { type: "thinking", thinking: "hm", signature: "sig-1" },
    { type: "text", text: "done" },
  ]);
  assert.deepEqual(seen.events, [
    { kind: "thinking", text: "hm" },
    { kind: "text", text: "done" },
  ]);

  // The thinking block must survive into `raw`, but stay out of the
  // normalized content the controller reads.
  const message = fromWireResponse(data);
  assert.deepEqual(message.content, [{ kind: "text", text: "done" }]);
  assert.equal((message.raw as unknown[]).length, 2);
});

test("orders blocks by index regardless of arrival order", async () => {
  const data = await assembleAnthropic(
    feed([
      { type: "content_block_start", index: 1, content_block: { type: "text", text: "second" } },
      { type: "content_block_start", index: 0, content_block: { type: "text", text: "first" } },
      { type: "message_stop" },
    ]),
  );
  assert.deepEqual(data.content, [
    { type: "text", text: "first" },
    { type: "text", text: "second" },
  ]);
});

test("surfaces a refusal as text the user can read", async () => {
  const data = await assembleAnthropic(
    feed([
      {
        type: "message_delta",
        delta: { stop_reason: "refusal", stop_details: { category: "cyber", explanation: "no" } },
      },
      { type: "message_stop" },
    ]),
  );

  assert.deepEqual(fromWireResponse(data).content, [
    { kind: "text", text: "[refused: cyber] no" },
  ]);
});

test("reports a truncated response", async () => {
  const data = await assembleAnthropic(
    feed([
      { type: "message_delta", delta: { stop_reason: "max_tokens" } },
      { type: "message_stop" },
    ]),
  );
  const block = fromWireResponse(data).content[0];
  assert.match(block?.kind === "text" ? block.text : "", /truncated/);
});

test("throws when the stream carries an error event", async () => {
  await assert.rejects(
    assembleAnthropic(feed([{ type: "error", error: { message: "overloaded" } }])),
    /overloaded/,
  );
});

test("combines input and output usage from separate stream events", async () => {
  const data = await assembleAnthropic(
    feed([
      {
        type: "message_start",
        message: { usage: { input_tokens: 20, cache_read_input_tokens: 8 } },
      },
      { type: "message_delta", delta: {}, usage: { output_tokens: 6 } },
      { type: "message_stop" },
    ]),
  );

  assert.deepEqual(fromWireResponse(data).usage, {
    inputTokens: 20,
    outputTokens: 6,
    cachedInputTokens: 8,
    cacheWriteInputTokens: 0,
    reasoningTokens: 0,
  });
});

test("rejects a stream that ends before message_stop", async () => {
  await assert.rejects(
    assembleAnthropic(feed([
      { type: "content_block_start", index: 0, content_block: { type: "text", text: "partial" } },
    ])),
    /ended before message_stop/,
  );
});
