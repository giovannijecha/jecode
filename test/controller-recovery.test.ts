import { test } from "node:test";
import assert from "node:assert/strict";
import type { Message } from "../src/types.ts";
import { runTurn } from "../src/controller.ts";
import type { Tool } from "../src/tools/index.ts";
import {
  scripted,
  echo,
  destroy,
  options,
  events,
  assistantText,
  aborted,
} from "../dev/test-support/controller.ts";

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

test("an interrupted shared batch preserves calls completed before cancellation", async () => {
  const control = new AbortController();
  const partiallyInterrupted: Tool = {
    ...echo,
    async run(args) {
      if (args.text === "complete") {
        return { output: "completed before interruption", summary: "complete" };
      }
      await new Promise((resolve) => setImmediate(resolve));
      const error = new Error("interrupted");
      control.abort(error);
      throw error;
    },
  };
  const provider = scripted([{
    role: "assistant",
    content: [
      { kind: "tool_call", id: "a", name: "echo", input: { text: "complete" } },
      { kind: "tool_call", id: "b", name: "echo", input: { text: "stop" } },
    ],
  }]);
  const history: Message[] = [];
  const shown: string[] = [];
  const sink = events();
  sink.onToolResult = (call, _result, summary) => shown.push(`${call.id}:${summary}`);

  await assert.rejects(
    runTurn(history, options(provider, { tools: [partiallyInterrupted] }), sink, control.signal),
    /interrupted/,
  );

  assert.deepEqual(history[1]?.content, [
    { kind: "tool_result", id: "a", output: "completed before interruption", isError: false },
    { kind: "tool_result", id: "b", output: "interrupted before completion", isError: true },
  ]);
  assert.deepEqual(shown, ["a:complete", "b:interrupted"]);
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

test("a result-surface failure preserves every completed tool result", async () => {
  const provider = scripted([{
    role: "assistant",
    content: [
      { kind: "tool_call", id: "a", name: "echo", input: { text: "completed" } },
      { kind: "tool_call", id: "b", name: "echo", input: { text: "pending" } },
    ],
  }]);
  const history: Message[] = [];
  const sink = events();
  let surfaced = 0;
  sink.onToolResult = () => {
    surfaced++;
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
    output: "pending",
    isError: false,
  });
  assert.equal(surfaced, 2);
});
