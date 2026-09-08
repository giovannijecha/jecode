import { test } from "node:test";
import assert from "node:assert/strict";
import type { Message } from "../src/types.ts";
import { MAX_CONCURRENT_TOOL_CALLS, runTurn } from "../src/controller.ts";
import type { Tool } from "../src/tools/index.ts";
import { scripted, echo, options, events, assistantText, delay } from "../dev/test-support/controller.ts";

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

test("a free read slot starts the fifth call before a slow first call settles", async () => {
  const started: string[] = [];
  let release!: () => void;
  let firstFinished = false;
  let refilledBeforeFirstFinished = false;
  const slow = new Promise<void>((resolve) => { release = resolve; });
  const shared: Tool = { ...echo, async run(args) {
    const text = String(args.text);
    started.push(text);
    if (text === "0") { await slow; firstFinished = true; }
    if (text === "4") { refilledBeforeFirstFinished = !firstFinished; release(); }
    return { output: text };
  } };
  const calls = Array.from({ length: 6 }, (_, index) => ({ kind: "tool_call" as const,
    id: String(index), name: "echo", input: { text: String(index) } }));
  const provider = scripted([{ role: "assistant", content: calls }, assistantText("done")]);
  const history: Message[] = [];
  const timer = setTimeout(() => release(), 2000);
  try {
    await runTurn(history, options(provider, { tools: [shared] }), events());
    assert.equal(refilledBeforeFirstFinished, true, "the fifth read must not wait for the first wave");
    assert.deepEqual(started, ["0", "1", "2", "3", "4", "5"]);
    assert.deepEqual(history[1]?.content.map(block => block.kind === "tool_result" && block.output), started);
  } finally { clearTimeout(timer); release(); }
});

test("turn transport scopes close after completion, failure, cancellation, and checkpoint failure", async () => {
  for (const mode of ["completed", "failure", "cancelled", "checkpoint"] as const) {
    const control = new AbortController();
    let closed = 0;
    const provider = scripted([]);
    provider.openTurn = () => ({ async send() {
      if (mode === "cancelled") control.abort(new Error("interrupted"));
      if (mode === "failure") throw new Error("failed");
      return assistantText("done");
    }, close() { closed++; } });
    const sink = events();
    if (mode === "checkpoint") sink.onCheckpoint = async () => { throw new Error("checkpoint failed"); };
    const turn = runTurn([], options(provider), sink, control.signal);
    if (mode === "completed") await turn;
    else await assert.rejects(turn);
    assert.equal(closed, 1, mode);
    assert.equal(provider.seen.length, 0, "the unscoped send is not used");
  }
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
