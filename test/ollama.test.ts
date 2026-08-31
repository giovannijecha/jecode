import { test } from "node:test";
import assert from "node:assert/strict";
import type { Message, StreamEvent } from "../src/types.ts";
import { assembleOllama } from "../src/providers/ollama-stream.ts";
import { fromWireReply, toWireMessages } from "../src/providers/ollama-wire.ts";
import { supportsAdaptiveThinking } from "../src/providers/anthropic.ts";
import { MAX_TOOL_ARGUMENT_CHARS } from "../src/providers/stream-limits.ts";

async function* feed(events: unknown[]): AsyncGenerator<unknown> {
  for (const event of events) yield event;
}

function chunk(delta: unknown, finish: string | null = null): unknown {
  return { choices: [{ delta, finish_reason: finish }] };
}

test("accumulates text deltas and streams them as they land", async () => {
  const seen: StreamEvent[] = [];
  const reply = await assembleOllama(
    feed([chunk({ content: "Ciao" }), chunk({ content: " mondo" }), chunk({}, "stop")]),
    (event) => seen.push(event),
  );

  assert.equal(reply.content, "Ciao mondo");
  assert.equal(reply.finishReason, "stop");
  assert.deepEqual(seen, [
    { kind: "text", text: "Ciao" },
    { kind: "text", text: " mondo" },
  ]);
});

test("rebuilds a tool call split across chunks", async () => {
  const reply = await assembleOllama(
    feed([
      chunk({ tool_calls: [{ index: 0, id: "c1", function: { name: "read_file", arguments: "" } }] }),
      chunk({ tool_calls: [{ index: 0, function: { arguments: '{"path"' } }] }),
      chunk({ tool_calls: [{ index: 0, function: { arguments: ':"a.ts"}' } }] }),
      chunk({}, "tool_calls"),
    ]),
  );

  const message = fromWireReply(reply);
  assert.deepEqual(message.content, [
    { kind: "tool_call", id: "c1", name: "read_file", input: { path: "a.ts" } },
  ]);
});

test("bounds tool arguments accumulated across Ollama calls", async () => {
  await assert.rejects(
    assembleOllama(
      feed([
        chunk({
          tool_calls: [{ index: 0, function: { arguments: "x".repeat(MAX_TOOL_ARGUMENT_CHARS) } }],
        }),
        chunk({ tool_calls: [{ index: 1, function: { arguments: "x" } }] }),
      ]),
    ),
    /streamed tool arguments exceeded/,
  );
});

test("keeps parallel calls apart by index and invents an id when none is sent", async () => {
  const reply = await assembleOllama(
    feed([
      chunk({
        tool_calls: [
          { index: 0, function: { name: "list_dir", arguments: "{}" } },
          { index: 1, function: { name: "read_file", arguments: '{"path":"b.ts"}' } },
        ],
      }),
      chunk({}, "tool_calls"),
    ]),
  );

  assert.deepEqual(
    reply.toolCalls.map((call) => [call.id, call.name]),
    [
      ["call_0", "list_dir"],
      ["call_1", "read_file"],
    ],
  );
});

test("streams reasoning under either field name", async () => {
  const seen: StreamEvent[] = [];
  await assembleOllama(
    feed([chunk({ reasoning: "hm" }), chunk({ reasoning_content: "ok" }), chunk({}, "stop")]),
    (event) => seen.push(event),
  );

  assert.deepEqual(seen, [
    { kind: "thinking", text: "hm" },
    { kind: "thinking", text: "ok" },
  ]);
});

test("malformed arguments degrade to an empty object rather than throwing", () => {
  const message = fromWireReply({
    content: "",
    toolCalls: [{ id: "c1", name: "read_file", args: "{not json" }],
  });

  assert.deepEqual(message.content, [
    { kind: "tool_call", id: "c1", name: "read_file", input: {} },
  ]);
});

test("lifts every tool result into its own message, in order", () => {
  const history: Message[] = [
    { role: "user", content: [{ kind: "text", text: "leggi a.ts" }] },
    {
      role: "assistant",
      content: [{ kind: "tool_call", id: "c1", name: "read_file", input: { path: "a.ts" } }],
    },
    {
      role: "user",
      content: [{ kind: "tool_result", id: "c1", output: "export {}", isError: false }],
    },
  ];

  assert.deepEqual(toWireMessages("be useful", history), [
    { role: "system", content: "be useful" },
    { role: "user", content: "leggi a.ts" },
    {
      role: "assistant",
      content: "",
      tool_calls: [
        { id: "c1", type: "function", function: { name: "read_file", arguments: '{"path":"a.ts"}' } },
      ],
    },
    { role: "tool", tool_call_id: "c1", content: "export {}" },
  ]);
});

test("omits tool_calls entirely when the assistant only spoke", () => {
  const wire = toWireMessages("", [
    { role: "assistant", content: [{ kind: "text", text: "fatto" }] },
  ]);

  assert.deepEqual(wire, [{ role: "assistant", content: "fatto" }]);
});

test("adaptive thinking is sent only to the models that accept it", () => {
  assert.equal(supportsAdaptiveThinking("claude-sonnet-5"), true);
  assert.equal(supportsAdaptiveThinking("claude-opus-5"), true);
  assert.equal(supportsAdaptiveThinking("claude-opus-4-6"), true);
  assert.equal(supportsAdaptiveThinking("claude-haiku-4-5-20251001"), false);
  assert.equal(supportsAdaptiveThinking("claude-sonnet-4-5"), false);
});

test("normalizes usage from the final compatible chunk", async () => {
  const reply = await assembleOllama(
    feed([
      chunk({}, "stop"),
      { choices: [], usage: { prompt_tokens: 14, completion_tokens: 4 } },
    ]),
  );

  assert.deepEqual(fromWireReply(reply).usage, {
    inputTokens: 14,
    outputTokens: 4,
    cachedInputTokens: 0,
    cacheWriteInputTokens: 0,
    reasoningTokens: 0,
  });
});

test("rejects a stream that ends before a finish reason", async () => {
  await assert.rejects(
    assembleOllama(feed([chunk({ content: "partial" })])),
    /ended before a finish reason/,
  );
});
