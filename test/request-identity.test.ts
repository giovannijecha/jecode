import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { ConversationTree } from "../src/conversation.ts";
import { requestIdentityForSession, resetRequestIdentity } from "../src/request-identity.ts";
import type { Session } from "../src/session.ts";
import { SessionPersistence } from "../src/sessions/runtime.ts";
import { DurableSessionStore } from "../src/sessions/store.ts";
import type { Message, Provider } from "../src/types.ts";
import { STEEL } from "../src/ui/theme.ts";
import { emptyUsage } from "../src/usage.ts";

test("provider request identity is stable across publication and resume", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-request-identity-"));
  const workspace = path.join(root, "workspace");
  const sessions = path.join(root, "sessions");
  await mkdir(workspace);
  try {
    const store = await DurableSessionStore.open(workspace, sessions);
    const persistence = SessionPersistence.fresh(store);
    const current = fixtureSession(persistence);
    const before = requestIdentityForSession(current);

    await persistence.checkpoint(completed());
    assert.equal(persistence.sessionId, persistence.conversationId);
    assert.deepEqual(requestIdentityForSession(current), before);
    const id = persistence.sessionId as string;
    await persistence.close();

    const resumed = await SessionPersistence.resume(store, id);
    current.persistence = resumed.persistence;
    assert.deepEqual(requestIdentityForSession(current), before);

    await resumed.persistence.reset();
    assert.notDeepEqual(requestIdentityForSession(current), before);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("ephemeral identity is stable until the conversation is reset", () => {
  const current = fixtureSession();
  const before = requestIdentityForSession(current);

  assert.deepEqual(requestIdentityForSession(current), before);
  resetRequestIdentity(current);
  assert.notDeepEqual(requestIdentityForSession(current), before);
});

test("provider routes have separate stable request identities", () => {
  const current = fixtureSession();
  const originalProvider = current.provider;
  const original = requestIdentityForSession(current);

  current.provider = { ...originalProvider, id: "other" };
  const other = requestIdentityForSession(current);
  assert.notDeepEqual(other, original);

  current.provider = originalProvider;
  assert.deepEqual(requestIdentityForSession(current), original);
});

function completed(): ConversationTree {
  const messages: Message[] = [
    { role: "user", content: [{ kind: "text", text: "hello" }] },
    { role: "assistant", content: [{ kind: "text", text: "done" }] },
  ];
  return ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-04T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages,
    blocks: [],
  }, "completed");
}

function fixtureSession(persistence?: SessionPersistence): Session {
  const provider: Provider = {
    id: "fake",
    defaultModel: "fake-1",
    auth: { kind: "api-key", keyVar: "FAKE_API_KEY" },
    blocked: () => undefined,
    models: () => Promise.resolve(["fake-1"]),
    send: () => Promise.resolve({ role: "assistant", content: [] }),
  };
  return {
    config: {
      providerId: "fake",
      model: "fake-1",
      reducedMotion: true,
      effort: "high",
      maxTokens: 4_096,
      compactionPercent: 85,
      root: process.cwd(),
      autoApprove: false,
      ephemeral: persistence === undefined,
    },
    provider,
    model: "fake-1",
    palette: STEEL,
    tools: [],
    system: "",
    conversation: ConversationTree.empty(),
    usage: emptyUsage(),
    ...(persistence === undefined ? {} : { persistence }),
  };
}
