import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { ConversationTree } from "../src/conversation.ts";
import { compactSession } from "../src/context/manual.ts";
import type { Session } from "../src/session.ts";
import { SessionPersistence } from "../src/sessions/runtime.ts";
import { DurableSessionStore } from "../src/sessions/store.ts";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import { STEEL } from "../src/ui/theme.ts";
import { emptyUsage } from "../src/usage.ts";

test("manual compaction revises only the active leaf context", async () => {
  const requests: SendRequest[] = [];
  const current = session(summarizer(requests));
  const old = "durable context ".repeat(2_000);
  current.conversation = completed(old, "Original answer.");
  const history = structuredClone(current.conversation.history);
  const transcript = structuredClone(current.conversation.transcript);
  const statuses: (string | undefined)[] = [];

  const result = await compactSession(current, { onStatus: (status) => statuses.push(status) });

  assert.equal(result, "compacted");
  assert.equal(requests.length, 1);
  assert.match(requests[0]?.system ?? "", /durable working memory/);
  assert.deepEqual(current.conversation.history, history);
  assert.deepEqual(current.conversation.transcript, transcript);
  assert.equal(current.conversation.activeNode?.revision, 2);
  assert.equal(current.conversation.activeNode?.context?.throughNodeId, 1);
  assert.match(JSON.stringify(current.conversation.contextHistory), /Manual summary/);
  assert.doesNotMatch(JSON.stringify(current.conversation.contextHistory), /durable context/);
  assert.deepEqual(statuses, ["Checking context", "Compacting", undefined]);
  assert.equal(current.usage.requests, 1);
  assert.equal(current.usage.lastInputTokens, 0);
});

test("manual compaction preserves a failed leaf's durable evidence", async () => {
  const current = session(summarizer([]));
  const prompt = "durable context ".repeat(2_000);
  const failure = { text: "provider disconnected", tone: "error" } as const;
  current.conversation = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-02T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: messages(prompt, "The previous attempt failed before completion."),
    blocks: [{ kind: "user", text: prompt }, { kind: "answer", text: "Partial answer." }],
    failure,
  }, "failed");

  assert.equal(await compactSession(current), "compacted");
  assert.equal(current.conversation.activeNode?.settlement, "failed");
  assert.deepEqual(current.conversation.activeNode?.failure, failure);
  assert.match(JSON.stringify(current.conversation.transcript), /provider disconnected/);
});

test("manual compaction remains useful below a large model's ordinary retention budget", async () => {
  const requests: SendRequest[] = [];
  const current = session({ ...summarizer(requests),
    contextWindow: async () => ({ tokens: 258_400, compactAtTokens: 244_800 }) });
  current.conversation = completed("durable context ".repeat(2_000), "Original answer.");
  const before = structuredClone(current.conversation.history);
  assert.equal(await compactSession(current), "compacted");
  assert.equal(requests.length, 1);
  assert.deepEqual(current.conversation.history, before);
  assert.ok(current.conversation.activeNode?.context);
});

test("manual compaction silently ignores a context too small to summarize", async () => {
  const requests: SendRequest[] = [];
  const current = session(summarizer(requests));
  current.conversation = completed("hello", "hi");

  assert.equal(await compactSession(current), "unchanged");
  assert.equal(requests.length, 0);
  assert.equal(current.conversation.activeNode?.revision, 1);
});

test("manual compaction cannot rewrite a historical branch point", async () => {
  const current = session(summarizer([]));
  const first = completed("context ".repeat(2_000), "first");
  current.conversation = first.commit({
    parentId: 1,
    createdAt: "2026-09-02T10:01:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: messages("second", "done"),
    blocks: [],
  }, "completed").select(1);

  await assert.rejects(compactSession(current), /continue this branch/);
  assert.equal(current.conversation.activeNode?.revision, 1);
});

test("manual compaction reports a failed summary without changing history", async () => {
  const provider = summarizer([]);
  provider.send = () => Promise.reject(new Error("offline"));
  const current = session(provider);
  current.conversation = completed("context ".repeat(2_000), "answer");

  await assert.rejects(compactSession(current), /offline/);
  assert.equal(current.conversation.activeNode?.revision, 1);
  assert.equal(current.conversation.activeNode?.context, undefined);
});

test("manual compaction survives resume inside the same logical session", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-manual-compaction-"));
  const workspace = path.join(root, "workspace");
  const sessions = path.join(root, "data", "sessions");
  await mkdir(workspace);
  try {
    const store = await DurableSessionStore.open(workspace, sessions);
    const source = completed("durable context ".repeat(2_000), "Original answer.");
    const published = await store.publish(source);
    const resumed = await SessionPersistence.resume(store, published.meta.id);
    const current = session(summarizer([]));
    current.conversation = resumed.conversation;
    current.persistence = resumed.persistence;

    assert.equal(await compactSession(current), "compacted");
    await resumed.persistence.close();

    const reopened = await SessionPersistence.resume(store, published.meta.id);
    assert.equal(reopened.persistence.sessionId, published.meta.id);
    assert.equal(reopened.conversation.activeNode?.revision, 2);
    assert.equal(reopened.conversation.activeNode?.context?.summary, "Manual summary.");
    assert.match(JSON.stringify(reopened.conversation.history), /durable context/);
    assert.doesNotMatch(JSON.stringify(reopened.conversation.contextHistory), /durable context/);
    await reopened.persistence.close();
    assert.equal((await store.list()).length, 1);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

function completed(user: string, answer: string): ConversationTree {
  return ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-02T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: messages(user, answer),
    blocks: [{ kind: "user", text: user }, { kind: "answer", text: answer }],
  }, "completed");
}

function messages(user: string, answer: string): Message[] {
  return [
    { role: "user", content: [{ kind: "text", text: user }] },
    { role: "assistant", content: [{ kind: "text", text: answer }] },
  ];
}

function summarizer(requests: SendRequest[]): Provider {
  return {
    id: "fake",
    defaultModel: "fake-1",
    auth: { kind: "api-key", keyVar: "FAKE_API_KEY" },
    blocked: () => undefined,
    models: () => Promise.resolve(["fake-1"]),
    contextWindow: () => Promise.resolve({ tokens: 32_000 }),
    send: (request) => {
      requests.push(request);
      return Promise.resolve({
        role: "assistant",
        content: [{ kind: "text", text: "Manual summary." }],
        usage: {
          inputTokens: 2_000,
          outputTokens: 20,
          cachedInputTokens: 0,
          cacheWriteInputTokens: 0,
          reasoningTokens: 0,
        },
      });
    },
  };
}

function session(provider: Provider): Session {
  return {
    config: {
      providerId: "fake",
      model: "fake-1",
      reducedMotion: true,
      effort: "high",
      maxTokens: 4_096,
      compactionPercent: 85,
      root: process.cwd(),

      ephemeral: false,
    },
    provider,
    model: "fake-1",
    palette: STEEL,
    tools: [],
    system: "",
    conversation: ConversationTree.empty(),
    usage: emptyUsage(),
  };
}
