import { test } from "node:test";
import assert from "node:assert/strict";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import type { ControllerEvents, ControllerOptions } from "../src/controller.ts";
import { resolveContextPolicy } from "../src/context/capacity.ts";
import { estimateRequestInputTokens } from "../src/context/budget.ts";
import { policyForContextWindow } from "../src/context/policy.ts";
import {
  MAX_CONCURRENT_TOOL_CALLS,
  MAX_TOOL_CALLS_PER_STEP,
  runTurn,
} from "../src/controller.ts";
import { steeringInbox } from "../src/steering.ts";
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
  concurrency: "shared",
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
  concurrency: "exclusive",
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
    contextPolicy: () => Promise.resolve(policyForContextWindow(undefined, 85)),
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
      if (event.kind !== "tool") texts.push(event.text);
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

test("steering queued during a final response continues the same turn", async () => {
  const provider = scripted([assistantText("first answer"), assistantText("revised answer")]);
  const sent: Message[][] = [];
  const send = provider.send.bind(provider);
  provider.send = async (request) => {
    sent.push(structuredClone(request.messages));
    return send(request);
  };
  const inbox = steeringInbox();
  const history: Message[] = [
    { role: "user", content: [{ kind: "text", text: "initial request" }] },
  ];
  const settlements: string[] = [];
  const steered: string[] = [];
  const sink = events();
  sink.onStream = (event) => {
    if (event.kind !== "text") return;
    sink.texts.push(event.text);
    if (event.text === "first answer") inbox.offer("change direction");
  };
  sink.onSteering = (text) => steered.push(text);
  sink.onCheckpoint = async (_checkpoint, settlement) => {
    settlements.push(settlement);
  };

  await runTurn(history, options(provider, { steering: inbox }), sink);

  assert.equal(provider.seen.length, 2);
  assert.deepEqual(texts(sent[1] ?? []), [
    "initial request",
    "first answer",
    "change direction",
  ]);
  assert.deepEqual(texts(history), [
    "initial request",
    "first answer",
    "change direction",
    "revised answer",
  ]);
  assert.deepEqual(steered, ["change direction"]);
  assert.deepEqual(settlements, ["checkpointed", "completed"]);
  assert.equal(inbox.accepting, false);
});

test("steering waits for the complete issued tool batch", async () => {
  const inbox = steeringInbox();
  const steeringTool: Tool = {
    ...echo,
    concurrency: "exclusive",
    async run(args) {
      inbox.offer("do not touch the README");
      return { output: String(args.text) };
    },
  };
  const provider = scripted([
    {
      role: "assistant",
      content: [{ kind: "tool_call", id: "a", name: "echo", input: { text: "done" } }],
    },
    assistantText("acknowledged"),
  ]);

  await runTurn([], options(provider, { tools: [steeringTool], steering: inbox }), events());

  const followup = provider.seen[1]?.messages ?? [];
  assert.equal(followup[0]?.role, "assistant");
  assert.equal(followup[1]?.content[0]?.kind, "tool_result");
  assert.equal(followup[2]?.role, "user");
  assert.equal(texts([followup[2] as Message])[0], "do not touch the README");
});

test("clamps output so the complete request fits a small model window", async () => {
  const provider = scripted([assistantText("all done")]);
  let capacityChecks = 0;
  let sentInputTokens = 0;
  const send = provider.send.bind(provider);
  provider.contextWindow = async () => {
    capacityChecks++;
    return { tokens: 4_096 };
  };
  provider.send = async (request) => {
    sentInputTokens = estimateRequestInputTokens(request);
    return send(request);
  };
  const history: Message[] = [{ role: "user", content: [{ kind: "text", text: "hi" }] }];
  const configured = 64_000;
  const contextPolicy = () => resolveContextPolicy({
    provider,
    model: "fake-1",
    compactionPercent: 85,
  });

  await runTurn(history, options(provider, { maxTokens: configured, contextPolicy }), events());

  const request = provider.seen[0] as SendRequest;
  assert.equal(capacityChecks, 1);
  assert.ok(request.maxTokens < configured);
  assert.ok(sentInputTokens + request.maxTokens <= 4_096);
});

test("rejects an oversized tool envelope before opening a provider request", async () => {
  const provider = scripted([assistantText("unreachable")]);
  const oversized: Tool = {
    ...echo,
    description: "x".repeat(15_000),
  };
  const policy = policyForContextWindow({ tokens: 4_096 }, 85);

  await assert.rejects(
    runTurn(
      [{ role: "user", content: [{ kind: "text", text: "hi" }] }],
      options(provider, {
        tools: [oversized],
        maxTokens: 64_000,
        contextPolicy: () => Promise.resolve(policy),
      }),
      events(),
    ),
    /request input needs approximately .* at least 256 output tokens are required/,
  );

  assert.equal(provider.seen.length, 0);
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

test("refreshes model capacity between provider steps in a turn", async () => {
  const provider = scripted([
    {
      role: "assistant",
      content: [{ kind: "tool_call", id: "a", name: "echo", input: { text: "one" } }],
    },
    assistantText("done"),
  ]);
  let resolutions = 0;

  await runTurn([], options(provider, {
    maxTokens: 64_000,
    contextPolicy: async () => {
      resolutions++;
      return policyForContextWindow({ tokens: resolutions === 1 ? 32_000 : 4_096 }, 85);
    },
  }), events());

  assert.equal(provider.seen.length, 2);
  assert.equal(resolutions, 2);
  assert.ok((provider.seen[1]?.maxTokens ?? Infinity) < 4_096);
});

test("awaits the durable tool checkpoint before asking the provider again", async () => {
  const provider = scripted([
    {
      role: "assistant",
      content: [{ kind: "tool_call", id: "a", name: "echo", input: { text: "one" } }],
    },
    assistantText("done"),
  ]);
  let release = (): void => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  let entered = (): void => {};
  const checkpointStarted = new Promise<void>((resolve) => {
    entered = resolve;
  });
  const settlements: string[] = [];
  const sink = events();
  sink.onCheckpoint = async (_history, settlement) => {
    settlements.push(settlement);
    if (settlement === "checkpointed") {
      entered();
      await gate;
    }
  };

  const running = runTurn([], options(provider), sink);
  await checkpointStarted;
  assert.equal(provider.seen.length, 1);
  release();
  await running;

  assert.equal(provider.seen.length, 2);
  assert.deepEqual(settlements, ["checkpointed", "completed"]);
});

test("keeps canonical turn history while replacing only the provider context", async () => {
  const provider = scripted([
    {
      role: "assistant",
      content: [{ kind: "tool_call", id: "a", name: "echo", input: { text: "one" } }],
    },
    assistantText("done"),
  ]);
  const history: Message[] = [
    { role: "user", content: [{ kind: "text", text: "full request" }] },
  ];
  const modelHistory: Message[] = [
    { role: "user", content: [{ kind: "text", text: "projected request" }] },
  ];
  const sink = events();
  sink.onCheckpoint = async (_history, settlement) => settlement === "checkpointed"
    ? [{ role: "user", content: [{ kind: "text", text: "compacted checkpoint" }] }]
    : undefined;

  await runTurn(history, options(provider), sink, undefined, modelHistory);

  assert.equal(texts(provider.seen[1]?.messages ?? [])[0], "compacted checkpoint");
  assert.doesNotMatch(texts(provider.seen[1]?.messages ?? []).join("\n"), /full request/);
  assert.deepEqual(texts(history), ["full request", "done"]);
  assert.equal(history.length, 4);
});

test("retries one definite context rejection only after the context hook replaces it", async () => {
  const seen: Message[][] = [];
  const requests: SendRequest[] = [];
  let calls = 0;
  const provider: Provider = {
    id: "fake",
    defaultModel: "fake-1",
    auth: { kind: "api-key", keyVar: "FAKE_API_KEY" },
    blocked: () => undefined,
    models: () => Promise.resolve(["fake-1"]),
    async send(request) {
      requests.push(request);
      seen.push(structuredClone(request.messages));
      calls++;
      if (calls === 1) {
        throw Object.assign(new Error("request rejected"), {
          status: 400,
          body: '{"error":{"code":"context_length_exceeded"}}',
        });
      }
      return assistantText("recovered");
    },
  };
  const history: Message[] = [{ role: "user", content: [{ kind: "text", text: "hello" }] }];
  const reasons: string[] = [];
  let resolutions = 0;
  const sink = events();
  sink.onContext = async (_history, _context, request) => {
    reasons.push(request.reason);
    if (request.reason !== "overflow" || request.error === undefined) return undefined;
    return [{ role: "user", content: [{ kind: "text", text: "safe summary" }] }];
  };

  await runTurn(history, options(provider, {
    maxTokens: 64_000,
    contextPolicy: async () => {
      resolutions++;
      return policyForContextWindow({ tokens: resolutions === 1 ? 32_000 : 4_096 }, 85);
    },
  }), sink);

  assert.equal(calls, 2);
  assert.equal(resolutions, 2);
  assert.ok((requests[1]?.maxTokens ?? Infinity) < 4_096);
  assert.deepEqual(reasons, ["budget", "overflow"]);
  assert.deepEqual(texts(seen[1] ?? []), ["safe summary"]);
  assert.deepEqual(texts(history), ["hello", "recovered"]);
});

test("runs shared calls concurrently while preserving result order", async () => {
  let active = 0;
  let peak = 0;
  const shared: Tool = {
    ...echo,
    async run(args) {
      active++;
      peak = Math.max(peak, active);
      await delay(args.text === "one" ? 30 : 5);
      active--;
      return { output: String(args.text) };
    },
  };
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
  const completed: string[] = [];
  const sink = events();
  sink.onToolResult = (call) => completed.push(call.id);

  await runTurn(history, options(provider, { tools: [shared] }), sink);

  assert.equal(peak, 2);
  assert.deepEqual(completed, ["a", "b"]);
  assert.deepEqual(
    history[1]?.content.map((block) => block.kind === "tool_result" ? block.output : ""),
    ["one", "two"],
  );
});

test("reports preparation before preview and execution only when the tool starts", async () => {
  const timeline: string[] = [];
  const inspected: Tool = {
    ...echo,
    async preview() {
      timeline.push("preview");
      return { before: "old", after: "new" };
    },
    async run(args) {
      timeline.push("run");
      return { output: String(args.text) };
    },
  };
  const provider = scripted([
    {
      role: "assistant",
      content: [{ kind: "tool_call", id: "a", name: "echo", input: { text: "one" } }],
    },
    assistantText("done"),
  ]);
  const sink = events();
  sink.onToolPreparing = (call, current, total) => {
    timeline.push(`preparing:${call.id}:${current}/${total}`);
  };
  sink.onToolCall = (call) => timeline.push(`ready:${call.id}`);
  sink.onToolStart = (call, current, total) => {
    timeline.push(`running:${call.id}:${current}/${total}`);
  };

  await runTurn([], options(provider, { tools: [inspected] }), sink);

  assert.deepEqual(timeline, [
    "preparing:a:1/1",
    "preview",
    "ready:a",
    "running:a:1/1",
    "run",
  ]);
});

test("bounds shared-call concurrency", async () => {
  let active = 0;
  let peak = 0;
  const shared: Tool = {
    ...echo,
    async run(args) {
      active++;
      peak = Math.max(peak, active);
      await delay(10);
      active--;
      return { output: String(args.text) };
    },
  };
  const calls = Array.from({ length: MAX_CONCURRENT_TOOL_CALLS + 3 }, (_, index) => ({
    kind: "tool_call" as const,
    id: String(index),
    name: "echo",
    input: { text: String(index) },
  }));
  const provider = scripted([
    { role: "assistant", content: calls },
    assistantText("done"),
  ]);

  await runTurn([], options(provider, { tools: [shared] }), events());

  assert.equal(peak, MAX_CONCURRENT_TOOL_CALLS);
});

test("keeps exclusive calls as ordered barriers between shared batches", async () => {
  const timeline: string[] = [];
  const shared: Tool = {
    ...echo,
    async run(args) {
      timeline.push(`start:${String(args.text)}`);
      await delay(10);
      timeline.push(`end:${String(args.text)}`);
      return { output: String(args.text) };
    },
  };
  const exclusive: Tool = {
    ...echo,
    name: "exclusive",
    concurrency: "exclusive",
    async run() {
      timeline.push("start:exclusive");
      await delay(5);
      timeline.push("end:exclusive");
      return { output: "exclusive" };
    },
  };
  const provider = scripted([
    {
      role: "assistant",
      content: [
        { kind: "tool_call", id: "a", name: "echo", input: { text: "one" } },
        { kind: "tool_call", id: "b", name: "echo", input: { text: "two" } },
        { kind: "tool_call", id: "c", name: "exclusive", input: {} },
        { kind: "tool_call", id: "d", name: "echo", input: { text: "three" } },
      ],
    },
    assistantText("done"),
  ]);

  await runTurn([], options(provider, { tools: [shared, exclusive] }), events());

  assert.ok(timeline.indexOf("start:exclusive") > timeline.indexOf("end:one"));
  assert.ok(timeline.indexOf("start:exclusive") > timeline.indexOf("end:two"));
  assert.ok(timeline.indexOf("start:three") > timeline.indexOf("end:exclusive"));
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
  const checkpoints: string[] = [];
  const sink = events();
  sink.onCheckpoint = async (_current, settlement) => {
    checkpoints.push(settlement);
  };

  await assert.rejects(
    runTurn(history, options(provider, { tools: [interrupted] }), sink, control.signal),
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
  assert.deepEqual(checkpoints, ["checkpointed"]);
});

test("an interrupted shared batch repairs every call in deterministic order", async () => {
  const control = new AbortController();
  const interrupted: Tool = {
    ...echo,
    async run(args, ctx) {
      if (args.text === "stop") {
        await new Promise((resolve) => setImmediate(resolve));
        const error = new Error("interrupted");
        control.abort(error);
        throw error;
      }
      await aborted(ctx.signal);
      return { output: "unreachable" };
    },
  };
  const provider = scripted([{
    role: "assistant",
    content: [
      { kind: "tool_call", id: "a", name: "echo", input: { text: "stop" } },
      { kind: "tool_call", id: "b", name: "echo", input: { text: "wait" } },
    ],
  }]);
  const history: Message[] = [];
  const shown: string[] = [];
  const sink = events();
  sink.onToolResult = (call, _result, summary) => shown.push(`${call.id}:${summary}`);

  await assert.rejects(
    runTurn(history, options(provider, { tools: [interrupted] }), sink, control.signal),
    /interrupted/,
  );

  assert.deepEqual(history[1]?.content, [
    { kind: "tool_result", id: "a", output: "interrupted before completion", isError: true },
    { kind: "tool_result", id: "b", output: "interrupted before completion", isError: true },
  ]);
  assert.deepEqual(shown, ["a:interrupted", "b:interrupted"]);
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
  sink.onToolPreparing = (call, current, total) =>
    progress.push(`preparing:${call.id}:${current}/${total}`);
  sink.onToolStart = (call, current, total) =>
    progress.push(`running:${call.id}:${current}/${total}`);

  await runTurn([], options(provider), sink);

  assert.deepEqual(progress, [
    "step:1",
    "usage:10",
    "preparing:a:1/1",
    "running:a:1/1",
    "step:2",
  ]);
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

function texts(messages: readonly Message[]): string[] {
  return messages.flatMap((message) => message.content)
    .filter((block) => block.kind === "text")
    .map((block) => block.text);
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function aborted(signal: AbortSignal | undefined): Promise<never> {
  return new Promise((_resolve, reject) => {
    const stop = () => reject(
      signal?.reason instanceof Error ? signal.reason : new Error("interrupted"),
    );
    if (signal?.aborted === true) stop();
    else signal?.addEventListener("abort", stop, { once: true });
  });
}
