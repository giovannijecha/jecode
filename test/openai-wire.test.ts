import { test } from "node:test";
import assert from "node:assert/strict";
import { fromWireResponse, toWireItems, toWireTool } from "../src/providers/openai-wire.ts";

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
