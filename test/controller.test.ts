import { test } from "node:test";
import assert from "node:assert/strict";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import type { ControllerEvents, ControllerOptions } from "../src/controller.ts";
import { MAX_TOOL_CALLS_PER_STEP, runTurn } from "../src/controller.ts";
import type { Tool } from "../src/tools/index.ts";
import { runCommand } from "../src/tools/shell.ts";

type FakeProvider = Provider & { seen: SendRequest[] };

function scripted(replies: Message[]): FakeProvider {
  const seen: SendRequest[] = [];
  let next = 0;
  return {
    id: "fake",
    defaultModel: "fake-1",
    auth: { kind: "api-key", keyVar: "FAKE_API_KEY" },
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

test("rejects an empty provider response instead of ending the turn silently", async () => {
  const provider = scripted([{ role: "assistant", content: [] }]);

  await assert.rejects(
    runTurn([], options(provider), events()),
    /completed without an answer or tool call/,
  );
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

test("refuses an oversized batch of tool calls before executing it", async () => {
  let runs = 0;
  const counted: Tool = {
    ...echo,
    async run() {
      runs++;
      return { output: "ran" };
    },
  };
  const calls = Array.from({ length: MAX_TOOL_CALLS_PER_STEP + 1 }, (_, index) => ({
    kind: "tool_call" as const,
    id: String(index),
    name: "echo",
    input: { text: "many" },
  }));
  const provider = scripted([{ role: "assistant", content: calls }]);
  const history: Message[] = [];

  await assert.rejects(
    runTurn(history, options(provider, { tools: [counted] }), events()),
    /tool calls in one step/,
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

test("cancelling one approval stops the remaining tool batch", async () => {
  const control = new AbortController();
  let approvals = 0;
  let runs = 0;
  const counted: Tool = {
    ...destroy,
    async run() {
      runs++;
      return { output: "destroyed" };
    },
  };
  const provider = scripted([{
    role: "assistant",
    content: [
      { kind: "tool_call", id: "a", name: "destroy", input: {} },
      { kind: "tool_call", id: "b", name: "destroy", input: {} },
    ],
  }]);
  const history: Message[] = [];
  const sink = events();
  const shown: string[] = [];
  sink.onToolResult = (_call, _result, summary) => shown.push(summary ?? "");
  sink.approve = async () => {
    approvals++;
    control.abort(new Error("interrupted"));
    return false;
  };

  await assert.rejects(
    runTurn(history, options(provider, { tools: [counted] }), sink, control.signal),
    /interrupted/,
  );

  assert.equal(approvals, 1);
  assert.equal(runs, 0);
  assert.equal(history.length, 2);
  assert.deepEqual(
    history[1]?.content.map((block) => block.kind === "tool_result" && block.isError),
    [true, true],
  );
  assert.deepEqual(shown, ["interrupted"]);
});

test("an interrupted tool leaves one result for every assistant call", async () => {
  const control = new AbortController();
  const interrupted: Tool = {
    ...echo,
    async run() {
      const error = new Error("interrupted");
      control.abort(error);
      throw error;
    },
  };
  const provider = scripted([{
    role: "assistant",
    content: [{ kind: "tool_call", id: "a", name: "echo", input: { text: "wait" } }],
  }]);
  const history: Message[] = [];

  await assert.rejects(
    runTurn(history, options(provider, { tools: [interrupted] }), events(), control.signal),
    /interrupted/,
  );

  assert.equal(history.length, 2);
  assert.deepEqual(history[1], {
    role: "user",
    content: [{
      kind: "tool_result",
      id: "a",
      output: "interrupted before completion",
      isError: true,
    }],
  });
});

test("an unexpected approval failure leaves tool history consistent", async () => {
  const provider = scripted([{
    role: "assistant",
    content: [{ kind: "tool_call", id: "a", name: "destroy", input: {} }],
  }]);
  const history: Message[] = [];
  const sink = events();
  sink.approve = async () => {
    throw new Error("approval surface failed");
  };

  await assert.rejects(
    runTurn(history, options(provider), sink),
    /approval surface failed/,
  );

  assert.equal(history.length, 2);
  assert.deepEqual(history[1], {
    role: "user",
    content: [{
      kind: "tool_result",
      id: "a",
      output: "tool processing stopped before completion",
      isError: true,
    }],
  });
});

test("a result-surface failure preserves completed and pending tool results", async () => {
  const provider = scripted([{
    role: "assistant",
    content: [
      { kind: "tool_call", id: "a", name: "echo", input: { text: "completed" } },
      { kind: "tool_call", id: "b", name: "echo", input: { text: "pending" } },
    ],
  }]);
  const history: Message[] = [];
  const sink = events();
  sink.onToolResult = () => {
    throw new Error("result surface failed");
  };

  await assert.rejects(
    runTurn(history, options(provider), sink),
    /result surface failed/,
  );

  const blocks = history[1]?.content ?? [];
  assert.equal(blocks[0]?.kind === "tool_result" ? blocks[0].output : undefined, "completed");
  assert.deepEqual(blocks[1], {
    kind: "tool_result",
    id: "b",
    output: "tool processing stopped before completion",
    isError: true,
  });
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

test("a shell credential cannot reach the provider follow-up or display events", async (context) => {
  const secret = "fixture-controller-credential-4902";
  const keyBefore = process.env["OPENAI_API_KEY"];
  const visibleBefore = process.env["JECODE_REVIEW_VISIBLE"];
  process.env["OPENAI_API_KEY"] = secret;
  process.env["JECODE_REVIEW_VISIBLE"] = secret;
  context.after(() => {
    restoreEnvironment("OPENAI_API_KEY", keyBefore);
    restoreEnvironment("JECODE_REVIEW_VISIBLE", visibleBefore);
  });

  const provider = scripted([
    {
      role: "assistant",
      content: [{
        kind: "tool_call",
        id: "shell",
        name: "run_command",
        input: { command: "node -e \"process.stdout.write(process.env.JECODE_REVIEW_VISIBLE ?? '')\"" },
      }],
    },
    assistantText("done"),
  ]);
  const history: Message[] = [];
  const shown: string[] = [];
  const live: string[] = [];
  const sink = events();
  sink.onToolResult = (_call, result) => shown.push(result.output);
  sink.onToolOutput = (_call, output) => live.push(output);

  await runTurn(history, options(provider, { tools: [runCommand] }), sink);

  const result = history[1]?.content[0];
  assert.equal(result?.kind, "tool_result");
  const sent = result?.kind === "tool_result" ? result.output : "";
  assert.match(sent, /\[credential redacted\]/);
  assert.doesNotMatch(sent, /fixture-controller-credential-4902/);
  assert.deepEqual(shown, [sent]);
  assert.ok(live.length > 0);
  assert.ok(live.every((output) => !output.includes(secret)));
  assert.ok(live.some((output) => output.includes("[credential redacted]")));
});

function restoreEnvironment(name: string, value: string | undefined): void {
  if (value === undefined) delete process.env[name];
  else process.env[name] = value;
}
