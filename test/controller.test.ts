import { test } from "node:test";
import assert from "node:assert/strict";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import type { ControllerEvents, ControllerOptions } from "../src/controller.ts";
import { runTurn } from "../src/controller.ts";
import type { Tool } from "../src/tools/index.ts";

type FakeProvider = Provider & { seen: SendRequest[] };

function scripted(replies: Message[]): FakeProvider {
  const seen: SendRequest[] = [];
  let next = 0;
  return {
    id: "fake",
    defaultModel: "fake-1",
    keyVar: "FAKE_API_KEY",
    seen,
    blocked: () => undefined,
    models: () => Promise.resolve(["fake-1"]),
    async send(request: SendRequest): Promise<Message> {
      seen.push(request);
      const reply = replies[next];
      next += 1;
      if (reply === undefined) throw new Error("the script ran out of replies");
      // Stand in for a provider streaming its text before it resolves.
      for (const block of reply.content) {
        if (block.kind === "text") request.onStream?.({ kind: "text", text: block.text });
      }
      return reply;
    },
  };
}

const echo: Tool = {
  name: "echo",
  description: "echoes",
  dangerous: false,
  input: { type: "object", properties: { text: { type: "string" } }, required: ["text"] },
  async run(args) {
    if (args.text === "boom") throw new Error("exploded");
    return { output: String(args.text) };
  },
};

const destroy: Tool = {
  name: "destroy",
  description: "needs approval",
  dangerous: true,
  input: { type: "object", properties: {}, required: [] },
  async run() {
    return { output: "destroyed" };
  },
};

function options(provider: Provider, overrides: Partial<ControllerOptions> = {}): ControllerOptions {
  return {
    provider,
    tools: [echo, destroy],
    model: "fake-1",
    system: "be useful",
    maxTokens: 100,
    effort: "high",
    maxSteps: 10,
    toolContext: { root: process.cwd() },
    ...overrides,
  };
}

function events(approve = true): ControllerEvents & { texts: string[] } {
  const texts: string[] = [];
  return {
    texts,
    onStream(event) {
      texts.push(event.text);
    },
    onToolCall() {},
    onToolResult() {},
    async approve() {
      return approve;
    },
  };
}

function assistantText(text: string): Message {
  return { role: "assistant", content: [{ kind: "text", text }] };
}

test("returns as soon as the model stops asking for tools", async () => {
  const provider = scripted([assistantText("all done")]);
  const history: Message[] = [{ role: "user", content: [{ kind: "text", text: "hi" }] }];
  const sink = events();

  await runTurn(history, options(provider), sink);

  assert.deepEqual(sink.texts, ["all done"]);
  assert.equal(provider.seen.length, 1);
  assert.equal(history.length, 2);
});

test("returns every result of one step in a single message", async () => {
  const provider = scripted([
    {
      role: "assistant",
      content: [
        { kind: "tool_call", id: "a", name: "echo", input: { text: "one" } },
        { kind: "tool_call", id: "b", name: "echo", input: { text: "two" } },
      ],
    },
    assistantText("done"),
  ]);
  const history: Message[] = [];

  await runTurn(history, options(provider), events());

  const results = history[1];
  assert.equal(results?.role, "user");
  assert.equal(results?.content.length, 2);
  assert.deepEqual(
    results?.content.map((block) => (block.kind === "tool_result" ? block.output : "")),
    ["one", "two"],
  );
});

test("a declined call comes back as an error result, and the loop continues", async () => {
  const provider = scripted([
    { role: "assistant", content: [{ kind: "tool_call", id: "a", name: "destroy", input: {} }] },
    assistantText("understood"),
  ]);
  const history: Message[] = [];

  await runTurn(history, options(provider), events(false));

  const result = history[1]?.content[0];
  assert.equal(result?.kind, "tool_result");
  assert.equal(result?.kind === "tool_result" && result.isError, true);
  assert.match(result?.kind === "tool_result" ? result.output : "", /declined/);
});

test("a throwing tool is reported to the model rather than crashing the turn", async () => {
  const provider = scripted([
    { role: "assistant", content: [{ kind: "tool_call", id: "a", name: "echo", input: { text: "boom" } }] },
    assistantText("recovered"),
  ]);
  const history: Message[] = [];

  await runTurn(history, options(provider), events());

  const result = history[1]?.content[0];
  assert.equal(result?.kind === "tool_result" && result.isError, true);
  assert.match(result?.kind === "tool_result" ? result.output : "", /exploded/);
});

test("an unknown tool is reported to the model", async () => {
  const provider = scripted([
    { role: "assistant", content: [{ kind: "tool_call", id: "a", name: "nope", input: {} }] },
    assistantText("ok"),
  ]);
  const history: Message[] = [];

  await runTurn(history, options(provider), events());

  const result = history[1]?.content[0];
  assert.match(result?.kind === "tool_result" ? result.output : "", /no such tool/);
});

test("gives up at the step limit instead of looping forever", async () => {
  const looping = Array.from({ length: 5 }, (): Message => ({
    role: "assistant",
    content: [{ kind: "tool_call", id: "a", name: "echo", input: { text: "again" } }],
  }));
  const provider = scripted(looping);

  await assert.rejects(
    runTurn([], options(provider, { maxSteps: 3 }), events()),
    /gave up after 3 steps/,
  );
});

test("reports provider usage and controller progress", async () => {
  const provider = scripted([
    {
      role: "assistant",
      content: [{ kind: "tool_call", id: "a", name: "echo", input: { text: "one" } }],
      usage: {
        inputTokens: 10,
        outputTokens: 2,
        cachedInputTokens: 4,
        cacheWriteInputTokens: 0,
        reasoningTokens: 1,
      },
    },
    assistantText("done"),
  ]);
  const progress: string[] = [];
  const sink = events();
  sink.onUsage = (usage) => progress.push(`usage:${usage.inputTokens}`);
  sink.onStep = (step) => progress.push(`step:${step}`);
  sink.onToolProgress = (current, total) => progress.push(`tool:${current}/${total}`);

  await runTurn([], options(provider), sink);

  assert.deepEqual(progress, ["step:1", "usage:10", "tool:1/1", "step:2"]);
});
