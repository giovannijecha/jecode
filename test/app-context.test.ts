import { test } from "node:test";
import assert from "node:assert/strict";
import { ConversationTree } from "../src/conversation.ts";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import { runApp } from "../src/tui/app.ts";
import { provider, session } from "../dev/test-support/app.ts";
import { virtualScreen, waitFor, plainRow } from "../dev/test-support/app-harness.ts";

test("the footer reports compaction without adding it to the transcript", async () => {
  let started = (): void => {};
  const entered = new Promise<void>((resolve) => {
    started = resolve;
  });
  let release = (): void => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const compacting: Provider = {
    ...provider(),
    contextWindow: () => Promise.resolve({ tokens: 64_000 }),
    async send(request): Promise<Message> {
      if (request.system.includes("durable working memory")) {
        started();
        await gate;
        return {
          role: "assistant",
          content: [{ kind: "text", text: "Persisted summary." }],
          usage: {
            inputTokens: 30_000,
            outputTokens: 10,
            cachedInputTokens: 0,
            cacheWriteInputTokens: 0,
            reasoningTokens: 0,
          },
        };
      }
      request.onStream?.({ kind: "text", text: "Answer after compaction." });
      return {
        role: "assistant",
        content: [{ kind: "text", text: "Answer after compaction." }],
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
  current.conversation = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-01T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: "old context ".repeat(10_000) }] },
      { role: "assistant", content: [{ kind: "text", text: "Old answer." }] },
    ],
    blocks: [],
  }, "completed");
  current.usage.lastInputTokens = 65_000;
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  feed("next request\r");
  await entered;
  await waitFor(
    () => (harness.frames.at(-1) ?? []).join("\n").includes("Compacting"),
    "compaction footer state",
  );
  release();
  await waitFor(() => current.conversation.history.length === 4, "compacted TUI turn");
  assert.doesNotMatch(JSON.stringify(current.conversation.transcript), /Persisted summary/);

  feed("/exit\r");
  await running;
});

test("/compact revises context without adding its summary to the transcript", async () => {
  const requests: SendRequest[] = [];
  const compacting: Provider = {
    ...provider(),
    contextWindow: () => Promise.resolve({ tokens: 64_000 }),
    async send(request): Promise<Message> {
      requests.push(request);
      return {
        role: "assistant",
        content: [{ kind: "text", text: "Manual durable summary." }],
        usage: {
          inputTokens: 12_000,
          outputTokens: 12,
          cachedInputTokens: 0,
          cacheWriteInputTokens: 0,
          reasoningTokens: 0,
        },
      };
    },
  };
  const current = session(compacting);
  const source = "large canonical context ".repeat(2_000);
  current.conversation = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-02T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: source }] },
      { role: "assistant", content: [{ kind: "text", text: "Canonical answer." }] },
    ],
    blocks: [
      { kind: "user", text: "large canonical context" },
      { kind: "answer", text: "Canonical answer." },
    ],
  }, "completed");
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  feed("/compact\r");
  await waitFor(() => current.conversation.activeNode?.revision === 2, "manual compaction");
  await waitFor(
    () => (harness.frames.at(-1) ?? []).join("\n").includes("context compacted"),
    "manual compaction feedback",
  );
  const footer = plainRow(harness.frames.at(-1)?.at(-1) ?? "");
  assert.match(footer, /context compacted$/);
  assert.doesNotMatch(footer, /[.·]\s+context compacted/);
  assert.equal(requests.length, 1);
  assert.match(JSON.stringify(current.conversation.history), /large canonical context/);
  assert.doesNotMatch(JSON.stringify(current.conversation.transcript), /Manual durable summary/);
  assert.match(JSON.stringify(current.conversation.contextHistory), /Manual durable summary/);

  feed("/exit\r");
  await running;
});
