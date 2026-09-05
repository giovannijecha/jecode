import { test } from "node:test";
import assert from "node:assert/strict";
import type { Message } from "../src/types.ts";
import { MAX_TOOL_CALLS_PER_RESPONSE, runTurn } from "../src/controller.ts";
import type { Tool } from "../src/tools/index.ts";
import { fromWireResponse as fromOpenAIWire } from "../src/providers/openai-wire.ts";
import { fromWireResponse as fromAnthropicWire } from "../src/providers/anthropic-wire.ts";
import { fromWireReply as fromOllamaWire } from "../src/providers/ollama-wire.ts";
import { scripted, echo, options, events, assistantText } from "../dev/test-support/controller.ts";

test("never executes provider tool calls from truncated responses", async (context) => {
  const cases: Array<{ name: string; reply: Message }> = [
    {
      name: "OpenAI Responses",
      reply: fromOpenAIWire({
        status: "incomplete",
        incomplete_details: { reason: "max_output_tokens" },
        output: [{
          type: "function_call",
          status: "incomplete",
          call_id: "partial-openai",
          name: "trap",
          arguments: "{",
        }],
      }),
    },
    {
      name: "Anthropic Messages",
      reply: fromAnthropicWire({
        stop_reason: "max_tokens",
        content: [{ type: "tool_use", id: "partial-anthropic", name: "trap", input: {} }],
      }),
    },
    {
      name: "Ollama Chat Completions",
      reply: fromOllamaWire({
        content: "",
        reasoning: "",
        finishReason: "length",
        toolCalls: [{ id: "partial-ollama", name: "trap", args: "{" }],
      }),
    },
  ];

  for (const fixture of cases) {
    await context.test(fixture.name, async () => {
      let executions = 0;
      const trap: Tool = {
        name: "trap",
        description: "must not run",
        dangerous: false,
        concurrency: "exclusive",
        input: { type: "object", properties: {}, required: [] },
        async run() {
          executions += 1;
          return { output: "unexpected" };
        },
      };
      const provider = scripted([fixture.reply]);

      await runTurn([], options(provider, { tools: [trap] }), events());

      assert.equal(executions, 0);
      assert.equal(provider.seen.length, 1);
    });
  }
});

test("never previews or executes invalid tool arguments from completed responses", async (context) => {
  const cases: Array<{ name: string; reply: Message }> = [
    {
      name: "OpenAI Responses",
      reply: fromOpenAIWire({
        status: "completed",
        output: [{
          type: "function_call",
          call_id: "invalid-openai",
          name: "list_dir",
          arguments: "{",
        }],
      }),
    },
    {
      name: "Anthropic Messages",
      reply: fromAnthropicWire({
        stop_reason: "tool_use",
        content: [{
          type: "tool_use",
          id: "invalid-anthropic",
          name: "list_dir",
          input: [],
        }],
      }),
    },
    {
      name: "Ollama Chat Completions",
      reply: fromOllamaWire({
        content: "",
        reasoning: "",
        toolCalls: [{ id: "invalid-ollama", name: "list_dir", args: "42" }],
        finishReason: "tool_calls",
      }),
    },
  ];

  for (const current of cases) {
    await context.test(current.name, async () => {
      let previews = 0;
      let approvals = 0;
      let runs = 0;
      const guardedTool: Tool = {
        name: "list_dir",
        description: "lists a directory",
        dangerous: true,
        concurrency: "exclusive",
        input: { type: "object", properties: {}, required: [] },
        async preview() {
          previews++;
          return { before: "before", after: "after" };
        },
        async run() {
          runs++;
          return { output: "unexpected execution" };
        },
      };
      const provider = scripted([current.reply, assistantText("recovered")]);
      const history: Message[] = [{
        role: "user",
        content: [{ kind: "text", text: "inspect" }],
      }];
      const checkpoints: string[] = [];
      const sink = events();
      sink.approve = async () => {
        approvals++;
        return true;
      };
      sink.onCheckpoint = async (_messages, settlement) => {
        checkpoints.push(settlement);
      };

      await runTurn(history, options(provider, { tools: [guardedTool] }), sink);

      assert.equal(previews, 0);
      assert.equal(approvals, 0);
      assert.equal(runs, 0);
      assert.equal(provider.seen.length, 2);
      const result = history[2]?.content[0];
      assert.equal(result?.kind === "tool_result" && result.isError, true);
      assert.match(result?.kind === "tool_result" ? result.output : "", /tool arguments/);
      assert.deepEqual(checkpoints, ["checkpointed", "completed"]);
    });
  }
});

test("refuses an oversized batch of tool calls before executing it", async () => {
  let runs = 0;
  const counted: Tool = {
    ...echo,
    async run() {
      runs++;
      return { output: "ran" };
    },
  };
  const calls = Array.from({ length: MAX_TOOL_CALLS_PER_RESPONSE + 1 }, (_, index) => ({
    kind: "tool_call" as const,
    id: String(index),
    name: "echo",
    input: { text: "many" },
  }));
  const provider = scripted([{ role: "assistant", content: calls }]);
  const history: Message[] = [];

  await assert.rejects(
    runTurn(history, options(provider, { tools: [counted] }), events()),
    /tool calls in one response/,
  );
  assert.equal(runs, 0);
  assert.equal(history.length, 0);
});

test("rejects duplicate tool call ids before executing or changing history", async () => {
  let runs = 0;
  const counted: Tool = {
    ...echo,
    async run() {
      runs++;
      return { output: "ran" };
    },
  };
  const provider = scripted([{
    role: "assistant",
    content: [
      { kind: "tool_call", id: "same", name: "echo", input: { text: "one" } },
      { kind: "tool_call", id: "same", name: "echo", input: { text: "two" } },
    ],
  }]);
  const history: Message[] = [];

  await assert.rejects(
    runTurn(history, options(provider, { tools: [counted] }), events()),
    /duplicate tool call ids/,
  );
  assert.equal(runs, 0);
  assert.deepEqual(history, []);
});

test("rejects a blank tool call id before executing or changing history", async () => {
  let runs = 0;
  const counted: Tool = {
    ...echo,
    async run() {
      runs++;
      return { output: "ran" };
    },
  };
  const provider = scripted([{
    role: "assistant",
    content: [{ kind: "tool_call", id: " ", name: "echo", input: { text: "one" } }],
  }]);
  const history: Message[] = [];

  await assert.rejects(
    runTurn(history, options(provider, { tools: [counted] }), events()),
    /tool call without an id/,
  );
  assert.equal(runs, 0);
  assert.deepEqual(history, []);
});
