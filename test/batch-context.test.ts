import { test } from "node:test";
import assert from "node:assert/strict";
import { runBatch } from "../src/batch.ts";
import { ConversationTree } from "../src/conversation.ts";
import type { Tool } from "../src/tools/index.ts";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import { provider, session, input, messageText } from "../dev/test-support/app.ts";

test("batch mode compacts model context while retaining the complete conversation", async () => {
  const seen: { system: string; messages: Message[] }[] = [];
  const compacting: Provider = {
    ...provider(),
    contextWindow: () => Promise.resolve({ tokens: 64_000 }),
    async send(request): Promise<Message> {
      seen.push({ system: request.system, messages: structuredClone(request.messages) });
      const summary = request.system.includes("durable working memory");
      return {
        role: "assistant",
        content: [{ kind: "text", text: summary ? "Earlier work is summarized." : "Final answer." }],
        usage: {
          inputTokens: summary ? 30_000 : 100,
          outputTokens: 10,
          cachedInputTokens: 0,
          cacheWriteInputTokens: 0,
          reasoningTokens: 0,
        },
      };
    },
  };
  const current = session(compacting);
  const old = "old context ".repeat(10_000);
  current.conversation = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-01T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: old }] },
      { role: "assistant", content: [{ kind: "text", text: "Old answer." }] },
    ],
    blocks: [],
  }, "completed");
  current.usage.lastInputTokens = 65_000;

  await runBatch(current, { lines: input("next request"), write: () => {} });

  assert.equal(seen.length, 2);
  assert.match(seen[0]?.system ?? "", /durable working memory/);
  assert.match(messageText(seen[1]?.messages[0]), /Earlier conversation summary/);
  assert.doesNotMatch(JSON.stringify(seen[1]?.messages), /old context/);
  assert.match(JSON.stringify(current.conversation.history), /old context/);
  assert.equal(current.conversation.activeNode?.context?.throughNodeId, 2);
  assert.equal(current.conversation.activeNode?.context?.messageCount, 0);
  assert.equal(current.usage.requests, 2);
  assert.equal(current.usage.lastInputTokens, 100);
});

test("a provider overflow can compact again after budget compaction", async () => {
  const requests: string[] = [];
  let summaries = 0;
  let turns = 0;
  const compacting: Provider = {
    ...provider(),
    contextWindow: () => Promise.resolve({ tokens: 64_000 }),
    async send(request): Promise<Message> {
      if (request.system.includes("durable working memory")) {
        summaries++;
        requests.push(`summary-${summaries}`);
        return {
          role: "assistant",
          content: [{
            kind: "text",
            text: summaries === 1 ? "budget memory ".repeat(500) : "overflow memory",
          }],
        };
      }
      turns++;
      requests.push(`turn-${turns}`);
      if (turns === 1) {
        throw Object.assign(new Error("request rejected"), {
          status: 400,
          body: '{"error":{"code":"context_length_exceeded"}}',
        });
      }
      return {
        role: "assistant",
        content: [{ kind: "text", text: "Recovered after the second compaction." }],
      };
    },
  };
  const current = session(compacting);
  current.conversation = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-04T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: "old context ".repeat(10_000) }] },
      { role: "assistant", content: [{ kind: "text", text: "Old answer." }] },
    ],
    blocks: [],
  }, "completed");
  current.usage.lastInputTokens = 65_000;

  await runBatch(current, { lines: input("next request"), write: () => {} });

  assert.deepEqual(requests, ["summary-1", "turn-1", "summary-2", "turn-2"]);
  assert.match(JSON.stringify(current.conversation.contextHistory), /overflow memory/);
  assert.match(JSON.stringify(current.conversation.history), /old context/);
});

test("missing provider usage replaces stale context pressure with the sent estimate", async () => {
  const requests: string[] = [];
  const withoutUsage: Provider = {
    ...provider(),
    contextWindow: () => Promise.resolve({ tokens: 3_891 }),
    async send(request): Promise<Message> {
      const summary = request.system.includes("durable working memory");
      requests.push(summary ? "summary" : "normal");
      return {
        role: "assistant",
        content: [{ kind: "text", text: summary ? "Earlier work." : "Answer." }],
      };
    },
  };
  const current = session(withoutUsage);
  current.conversation = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-02T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: "old context ".repeat(350) }] },
      { role: "assistant", content: [{ kind: "text", text: "Old answer." }] },
    ],
    blocks: [],
  }, "completed");
  current.usage.lastInputTokens = 3_500;

  await runBatch(current, { lines: input("first", "second"), write: () => {} });

  assert.deepEqual(requests, ["summary", "normal", "normal"]);
  assert.ok(current.usage.lastInputTokens < 3_500);
});

test("a failed automatic summary is not retried before the unchanged tool follow-up", async () => {
  const requests: string[] = [];
  let normalRequests = 0;
  const compacting: Provider = {
    ...provider(),
    contextWindow: () => Promise.resolve({ tokens: 64_000 }),
    async send(request): Promise<Message> {
      const summary = request.system.includes("durable working memory");
      requests.push(summary ? "summary" : "normal");
      if (summary) throw new Error("summary unavailable");
      normalRequests++;
      if (normalRequests === 1) {
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
      return {
        role: "assistant",
        content: [{ kind: "text", text: "Recovered without duplicate compaction." }],
        usage: {
          inputTokens: 6_000,
          outputTokens: 10,
          cachedInputTokens: 0,
          cacheWriteInputTokens: 0,
          reasoningTokens: 0,
        },
      };
    },
  };
  const fixture: Tool = {
    name: "fixture",
    description: "Return bounded fixture evidence",
    dangerous: false,
    concurrency: "shared",
    input: { type: "object", properties: {} },
    run: () => Promise.resolve({ output: "fixture evidence" }),
  };
  const current = session(compacting);
  current.tools = [fixture];
  current.conversation = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-04T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: "old context ".repeat(2_000) }] },
      { role: "assistant", content: [{ kind: "text", text: "Old answer." }] },
    ],
    blocks: [],
  }, "completed");

  await runBatch(current, { lines: input("inspect once"), write: () => {} });

  assert.deepEqual(requests, ["normal", "summary", "normal"]);
  assert.equal(current.conversation.activeNode?.settlement, "completed");
});

test("batch /new resets the automatic-compaction breaker", async () => {
  const requests: string[] = [];
  const compacting: Provider = {
    ...provider(),
    contextWindow: () => Promise.resolve({ tokens: 64_000 }),
    async send(request): Promise<Message> {
      const summary = request.system.includes("durable working memory");
      requests.push(summary ? "summary" : "normal");
      if (summary) throw new Error("summary unavailable");
      return {
        role: "assistant",
        content: [{ kind: "text", text: "Continue." }],
        usage: {
          inputTokens: 100,
          outputTokens: 5,
          cachedInputTokens: 0,
          cacheWriteInputTokens: 0,
          reasoningTokens: 0,
        },
      };
    },
  };
  const current = session(compacting);
  const pressure = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-04T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: "old context ".repeat(10_000) }] },
      { role: "assistant", content: [{ kind: "text", text: "Old answer." }] },
    ],
    blocks: [],
  }, "completed");
  current.conversation = pressure;
  current.usage.lastInputTokens = 65_000;

  async function* lines(): AsyncIterable<string> {
    yield "first";
    yield "/new";
    current.conversation = pressure;
    current.usage.lastInputTokens = 65_000;
    yield "second";
  }

  await runBatch(current, { lines: lines(), write: () => {} });

  assert.deepEqual(requests, ["summary", "normal", "summary", "normal"]);
});

test("batch /compact updates the next provider projection without transcript noise", async () => {
  const seen: SendRequest[] = [];
  const compacting: Provider = {
    ...provider(),
    contextWindow: () => Promise.resolve({ tokens: 64_000 }),
    async send(request): Promise<Message> {
      seen.push(request);
      const summary = request.system.includes("durable working memory");
      return {
        role: "assistant",
        content: [{ kind: "text", text: summary ? "Manual batch summary." : "After compact." }],
        usage: {
          inputTokens: summary ? 10_000 : 80,
          outputTokens: 8,
          cachedInputTokens: 0,
          cacheWriteInputTokens: 0,
          reasoningTokens: 0,
        },
      };
    },
  };
  const current = session(compacting);
  current.conversation = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-02T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: "old batch context ".repeat(2_000) }] },
      { role: "assistant", content: [{ kind: "text", text: "Old answer." }] },
    ],
    blocks: [],
  }, "completed");
  const output: string[] = [];

  await runBatch(current, {
    lines: input("/compact", "next request", "/exit"),
    write: (text) => output.push(text),
  });

  assert.equal(seen.length, 2);
  assert.match(seen[0]?.system ?? "", /durable working memory/);
  assert.match(messageText(seen[1]?.messages[0]), /Earlier conversation summary/);
  assert.doesNotMatch(JSON.stringify(seen[1]?.messages), /old batch context/);
  assert.match(output.join(""), /context compacted/);
  assert.doesNotMatch(JSON.stringify(current.conversation.transcript), /Manual batch summary/);
});
