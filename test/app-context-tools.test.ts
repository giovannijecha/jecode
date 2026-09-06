import { test } from "node:test";
import assert from "node:assert/strict";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import type { Tool } from "../src/tools/index.ts";
import { estimateRequestInputTokens } from "../src/context/budget.ts";
import { runApp } from "../src/tui/app.ts";
import { provider, session } from "../dev/test-support/app.ts";
import { virtualScreen, waitFor, waitForIdle } from "../dev/test-support/app-harness.ts";
import { channel } from "node:diagnostics_channel";
import { CONTEXT_DIAGNOSTIC_CHANNEL, safeDiagnostic } from "../src/context/diagnostics.ts";
import type { RequestDiagnostic } from "../src/context/diagnostics.ts";

test("a new TUI turn keeps provider calibration when local estimates exceed the trigger", async () => {
  const evidence = "export function checked(value) { return value !== undefined ? value : null; }\n"
    .repeat(230).slice(0, 16_384);
  let reads = 0;
  let summaries = 0;
  const observed: RequestDiagnostic[] = [];
  const receive = (value: unknown) => {
    const event = safeDiagnostic(value);
    if (event?.kind === "request") observed.push(event);
  };
  const diagnostics = channel(CONTEXT_DIAGNOSTIC_CHANNEL);
  diagnostics.subscribe(receive);
  const current = session({ ...provider(), contextWindow: async () => ({ tokens: 64_000 }),
    async send(request) {
      if (request.identity?.purpose === "compaction") {
        summaries++; return provider("Earlier source reviewed.").send(request);
      }
      return { role: "assistant", content: reads < 9
        ? [{ kind: "tool_call", id: `read-${reads}`, name: "fixture", input: {} }]
        : [{ kind: "text", text: "Done." }],
        usage: { inputTokens: Math.max(1, Math.floor(estimateRequestInputTokens(request) / 2.3)),
          outputTokens: 10, reasoningTokens: 0, cachedInputTokens: 0, cacheWriteInputTokens: 0 } };
    },
  });
  current.tools = [{ name: "fixture", description: "Read inert source", dangerous: false, concurrency: "shared",
    input: { type: "object", properties: {} },
    async run() { reads++; return { output: evidence }; } }];
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();
  try {
    feed("inspect\r");
    await waitFor(() => current.conversation.activeNode?.settlement === "completed", "calibrated first turn");
    await waitForIdle(harness, "first turn idle");
    feed("continue\r");
    await waitFor(() => current.conversation.activeNodeId === 2 &&
      current.conversation.activeNode?.settlement === "completed", "calibrated second turn");
    assert.equal(summaries, 0);
    assert.equal(observed.length, 11);
    const next = observed.at(-1)!;
    assert.equal(next.source, "provider-prefix");
    assert.ok(next.estimatedTokens > 54_400);
    assert.ok(next.inputTokens < 54_400);
    assert.equal(current.conversation.activeNode?.context, undefined);
  } finally {
    diagnostics.unsubscribe(receive);
    feed("/exit\r");
    await running;
  }
});

test("TUI keeps twelve substantial tool reads intact across turns without premature compact", async () => {
  const evidence = "export function checked(value) { return value !== undefined ? value : null; }\n"
    .repeat(230).slice(0, 16_384);
  let runs = 0;
  let summaries = 0;
  const requests: SendRequest[] = [];
  const from: Provider = {
    ...provider(),
    contextWindow: async () => ({ tokens: 258_400, compactAtTokens: 244_800 }),
    async send(request) {
      if (request.identity?.purpose === "compaction") {
        summaries++;
        return provider("Unexpected summary").send(request);
      }
      requests.push({ ...request, messages: structuredClone(request.messages) });
      const outputs = request.messages.flatMap((m) => m.content).filter((b) => b.kind === "tool_result");
      assert.ok(outputs.every((b) => b.output === evidence), "full file evidence reaches the model");
      const done = runs === 6 || runs === 12;
      const previousAnswer = request.messages.at(-2)?.content.some((b) => b.kind === "text" && b.text === "Done.");
      const finish = done && !previousAnswer;
      const content: Message["content"] = finish
        ? [{ kind: "text", text: "Done." }]
        : [{ kind: "tool_call", id: `read-${runs + 1}`, name: "fixture", input: {} }];
      request.onStream?.({ kind: "text", text: finish ? "Done." : "" });
      return {
        role: "assistant", content,
        usage: {
          inputTokens: estimateRequestInputTokens(request), outputTokens: 30,
          cachedInputTokens: 0, cacheWriteInputTokens: 0, reasoningTokens: 0,
        },
      };
    },
  };
  const fixture: Tool = {
    name: "fixture", description: "Read inert source", dangerous: false, concurrency: "shared",
    input: { type: "object", properties: {} },
    async run() { runs++; return { output: evidence }; },
  };
  const current = session(from);
  current.config.compactionPercent = 95;
  current.tools = [fixture];
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();
  try {
    feed("inspect six files\r");
    await waitFor(() => current.conversation.activeNode?.settlement === "completed", "first six reads");
    await waitForIdle(harness, "first tool turn idle");
    feed("inspect six more files\r");
    await waitFor(() => current.conversation.activeNodeId === 2 &&
      current.conversation.activeNode?.settlement === "completed", "second six reads");
    assert.equal(runs, 12);
    assert.equal(summaries, 0);
    assert.equal(requests.length, 14);
    assert.equal(current.conversation.history.flatMap((m) => m.content)
      .filter((b) => b.kind === "tool_result").length, 12);
  } finally {
    feed("/exit\r");
    await running;
  }
});

test("completed TUI responses do not start a trailing compaction", async () => {
  let summaries = 0;
  const current = session({
    ...provider(),
    contextWindow: async () => ({ tokens: 4_096 }),
    async send(request) {
      if (request.identity?.purpose === "compaction") summaries++;
      return {
        role: "assistant", content: [{ kind: "text", text: "A long answer ".repeat(600) }],
        usage: { inputTokens: 3_700, outputTokens: 2_000, cachedInputTokens: 0,
          cacheWriteInputTokens: 0, reasoningTokens: 0 },
      };
    },
  });
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();
  try {
    feed("answer\r");
    await waitFor(() => current.conversation.activeNode?.settlement === "completed", "completed answer");
    await waitForIdle(harness, "settled without compact");
    assert.equal(summaries, 0);
  } finally {
    feed("/exit\r");
    await running;
  }
});
