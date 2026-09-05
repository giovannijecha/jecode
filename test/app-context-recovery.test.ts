import { test } from "node:test";
import assert from "node:assert/strict";
import { ConversationTree } from "../src/conversation.ts";
import type { Tool } from "../src/tools/index.ts";
import type { Message, Provider } from "../src/types.ts";
import { runApp } from "../src/tui/app.ts";
import { provider, session } from "../dev/test-support/app.ts";
import { virtualScreen, waitFor, waitForIdle } from "../dev/test-support/app-harness.ts";

function pressure(repetitions = 10_000): ConversationTree {
  return ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-04T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: "old context ".repeat(repetitions) }] },
      { role: "assistant", content: [{ kind: "text", text: "Old answer." }] },
    ],
    blocks: [],
  }, "completed");
}

function answer(text: string): Message {
  return { role: "assistant", content: [{ kind: "text", text }] };
}

test("a TUI provider overflow can compact again after budget compaction", async () => {
  const requests: string[] = [];
  const projections: Message[][] = [];
  let summaries = 0;
  let turns = 0;
  const compacting: Provider = {
    ...provider(),
    contextWindow: () => Promise.resolve({ tokens: 64_000 }),
    async send(request) {
      if (request.system.includes("durable working memory")) {
        requests.push(`summary-${++summaries}`);
        return answer(summaries === 1 ? "budget memory ".repeat(500) : "overflow memory");
      }
      requests.push(`turn-${++turns}`);
      projections.push(structuredClone(request.messages));
      if (turns === 1) {
        throw Object.assign(new Error("request rejected"), {
          status: 400,
          body: '{"error":{"code":"context_length_exceeded"}}',
        });
      }
      return provider("Recovered after the second compaction.").send(request);
    },
  };
  const current = session(compacting);
  current.conversation = pressure();
  current.usage.lastInputTokens = 65_000;
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  try {
    feed("next request\r");
    await waitFor(() => current.conversation.activeNodeId === 2, "recovered TUI turn");
    assert.deepEqual(requests, ["summary-1", "turn-1", "summary-2", "turn-2"]);
    assert.match(JSON.stringify(projections[0]), /budget memory/);
    assert.match(JSON.stringify(projections[1]), /overflow memory/);
    assert.doesNotMatch(JSON.stringify(projections[1]), /budget memory|old context/);
    assert.match(JSON.stringify(current.conversation.contextHistory), /overflow memory/);
    assert.match(JSON.stringify(current.conversation.history), /old context/);
    assert.equal(current.conversation.activeNode?.settlement, "completed");
  } finally {
    feed("/exit\r");
    await running;
  }
});

test("TUI replies without usage replace stale context pressure with the sent estimate", async () => {
  const requests: string[] = [];
  const withoutUsage: Provider = {
    ...provider(),
    contextWindow: () => Promise.resolve({ tokens: 3_891 }),
    async send(request) {
      const summary = request.system.includes("durable working memory");
      requests.push(summary ? "summary" : "normal");
      return answer(summary ? "Earlier work." : "Answer.");
    },
  };
  const current = session(withoutUsage);
  current.conversation = pressure(350);
  current.usage.lastInputTokens = 3_500;
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  try {
    feed("first\r");
    await waitFor(() => current.conversation.activeNodeId === 2, "first turn without usage");
    await waitForIdle(harness, "first turn idle state");
    assert.ok(current.usage.lastInputTokens > 0);
    assert.ok(current.usage.lastInputTokens < 3_500);
    feed("second\r");
    await waitFor(() => current.conversation.activeNodeId === 3, "second turn without usage");
    assert.deepEqual(requests, ["summary", "normal", "normal"]);
    assert.ok(current.usage.lastInputTokens < 3_500);
    assert.match(JSON.stringify(current.conversation.history), /old context/);
    assert.match(JSON.stringify(current.conversation.contextHistory), /Earlier work/);
  } finally {
    feed("/exit\r");
    await running;
  }
});

test("a failed TUI summary is not retried before the unchanged tool follow-up", async () => {
  const requests: string[] = [];
  let normalRequests = 0;
  let toolRuns = 0;
  const compacting: Provider = {
    ...provider(),
    contextWindow: () => Promise.resolve({ tokens: 64_000 }),
    async send(request) {
      const summary = request.system.includes("durable working memory");
      requests.push(summary ? "summary" : "normal");
      if (summary) throw new Error("summary unavailable");
      if (normalRequests++ === 0) {
        return {
          role: "assistant",
          content: [{ kind: "tool_call", id: "inspect", name: "fixture", input: {} }],
          usage: {
            inputTokens: 60_000,
            outputTokens: 10,
            cachedInputTokens: 0,
            cacheWriteInputTokens: 0,
            reasoningTokens: 0,
          },
        };
      }
      assert.match(JSON.stringify(request.messages), /fixture evidence/);
      return provider("Recovered without duplicate compaction.").send(request);
    },
  };
  const fixture: Tool = {
    name: "fixture",
    description: "Return bounded fixture evidence",
    dangerous: false,
    concurrency: "shared",
    input: { type: "object", properties: {} },
    run: async () => {
      toolRuns++;
      return { output: "fixture evidence" };
    },
  };
  const current = session(compacting);
  current.tools = [fixture];
  current.conversation = pressure(2_000);
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  try {
    feed("inspect once\r");
    await waitFor(
      () => current.conversation.activeNodeId === 2 &&
        current.conversation.activeNode?.settlement === "completed",
      "tool follow-up after failed compaction",
    );
    assert.deepEqual(requests, ["normal", "summary", "normal"]);
    assert.equal(toolRuns, 1);
    assert.match(JSON.stringify(current.conversation.history), /old context/);
    assert.equal(current.conversation.activeNode?.context, undefined);
  } finally {
    feed("/exit\r");
    await running;
  }
});

test("TUI /new resets the automatic-compaction breaker", async () => {
  const requests: string[] = [];
  const compacting: Provider = {
    ...provider(),
    contextWindow: () => Promise.resolve({ tokens: 64_000 }),
    async send(request) {
      const summary = request.system.includes("durable working memory");
      requests.push(summary ? "summary" : "normal");
      if (summary) throw new Error("summary unavailable");
      return provider("Continue.").send(request);
    },
  };
  const current = session(compacting);
  const original = pressure();
  current.conversation = original;
  current.usage.lastInputTokens = 65_000;
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  try {
    feed("first\r");
    await waitFor(() => current.conversation.activeNodeId === 2, "first summary failure");
    await waitForIdle(harness, "first failure idle state");
    feed("/new\r");
    await waitFor(() => current.conversation.activeNodeId === 0, "new conversation");
    await waitForIdle(harness, "new conversation idle state");
    assert.equal(current.usage.requests, 0);
    assert.deepEqual(current.conversation.history, []);
    // Reuse the same node and message counts so the failed generation key collides.
    current.conversation = original;
    current.usage.lastInputTokens = 65_000;
    feed("second\r");
    await waitFor(() => current.conversation.activeNodeId === 2, "summary retry after /new");
    assert.deepEqual(requests, ["summary", "normal", "summary", "normal"]);
  } finally {
    feed("/exit\r");
    await running;
  }
});

for (const phase of ["metadata lookup", "automatic compaction"] as const) {
  test(`TUI shutdown interrupts ${phase} before normal generation`, async () => {
    const shutdown = new AbortController();
    const reason = new Error("received SIGTERM");
    let requestSignal: AbortSignal | undefined;
    let release = (): void => {};
    let normalRequests = 0;
    const waitForCancellation = (signal: AbortSignal | undefined): Promise<never> => {
      requestSignal = signal;
      return new Promise((_resolve, reject) => {
        const abort = (): void => reject(signal?.reason);
        release = () => {
          signal?.removeEventListener("abort", abort);
          reject(reason);
        };
        signal?.addEventListener("abort", abort, { once: true });
      });
    };
    const waiting: Provider = {
      ...provider(),
      async contextWindow(_model, signal) {
        if (phase === "metadata lookup") return waitForCancellation(signal);
        return { tokens: 64_000 };
      },
      async send(request) {
        if (request.system.includes("durable working memory")) {
          return waitForCancellation(request.signal);
        }
        normalRequests++;
        return provider().send(request);
      },
    };
    const current = session(waiting);
    current.conversation = pressure();
    current.usage.lastInputTokens = 65_000;
    const before = current.conversation.history;
    const harness = virtualScreen();
    const running = runApp(current, process.cwd(), {
      ...harness.environment,
      shutdownSignal: shutdown.signal,
    });
    const feed = await harness.input();

    try {
      feed("continue\r");
      await waitFor(() => requestSignal !== undefined, `TUI ${phase}`);
      shutdown.abort(reason);
      await running;

      assert.equal(requestSignal?.aborted, true);
      assert.equal(requestSignal?.reason, reason);
      assert.equal(normalRequests, 0);
      assert.deepEqual(current.conversation.history.slice(0, before.length), before);
      assert.equal(current.conversation.activeNode?.settlement, "interrupted");
      assert.equal(current.conversation.activeNode?.context, undefined);
      assert.equal(current.usage.requests, 0);
      assert.match(JSON.stringify(current.conversation.transcript), /interrupted/);
      assert.equal(harness.left(), true);
    } finally {
      shutdown.abort(reason);
      release();
      await running;
    }
  });
}
