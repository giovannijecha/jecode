import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { ConversationTree } from "../src/conversation.ts";
import { SessionPersistence } from "../src/sessions/runtime.ts";
import { DurableSessionStore } from "../src/sessions/store.ts";

test("repeated resumes update one stable logical session", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const source = complete(ConversationTree.empty(), 0, "first", "one");
    const published = await store.publish(source);
    const firstResume = await SessionPersistence.resume(store, published.meta.id);
    const second = complete(firstResume.conversation, 1, "second", "two");

    await firstResume.persistence.checkpoint(second);
    assert.equal(firstResume.persistence.sessionId, published.meta.id);
    await firstResume.persistence.close();
    assert.equal((await store.list()).length, 1);

    const secondResume = await SessionPersistence.resume(store, published.meta.id);
    const third = complete(secondResume.conversation, 2, "third", "three");
    await secondResume.persistence.checkpoint(third);
    assert.equal(secondResume.persistence.sessionId, published.meta.id);
    await secondResume.persistence.close();

    const catalog = await store.list();
    assert.equal(catalog.length, 1);
    assert.equal(catalog[0]?.id, published.meta.id);
    assert.equal(catalog[0]?.turns, 3);
    const loaded = await store.load(published.meta.id);
    assert.equal(loaded.conversation.nodes.length, 3);
    assert.deepEqual(loaded.conversation.history, third.history);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("reset closes the current lease and starts an unpublished session", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const persistence = SessionPersistence.fresh(store);
    await persistence.checkpoint(complete(ConversationTree.empty(), 0, "first", "one"));
    const id = persistence.sessionId as string;
    assert.equal((await store.list())[0]?.active, true);
    await persistence.reset();
    assert.equal(persistence.sessionId, null);
    assert.equal((await store.list())[0]?.active, false);
    const lease = await store.claim(id);
    await lease.close();
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("checkpointing stops when the process no longer owns the session lease", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const persistence = SessionPersistence.fresh(store);
    const first = complete(ConversationTree.empty(), 0, "first", "one");
    await persistence.checkpoint(first);
    const id = persistence.sessionId as string;
    const active = path.join(fixture.sessions, store.workspaceDigest, id, "active");
    await writeFile(active, `${process.pid}:replacement-fixture-token`, "utf8");

    await assert.rejects(
      persistence.checkpoint(complete(first, 1, "second", "two")),
      /lease is no longer owned/,
    );
    assert.equal((await store.load(id)).conversation.nodes.length, 1);
    await persistence.close();
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("resume rewinds an unfinished tool turn to its latest completed ancestor", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const first = complete(ConversationTree.empty(), 0, "first", "one");
    const pending = first.commit({
      parentId: 1,
      createdAt: "2026-09-01T10:01:00.000Z",
      identity: { providerId: "anthropic", model: "claude", effort: "high" },
      messages: [
        { role: "user", content: [{ kind: "text", text: "inspect" }] },
        {
          role: "assistant",
          content: [{ kind: "tool_call", id: "call-1", name: "list_dir", input: { path: "." } }],
          raw: { signedThinking: "never persist or resume" },
          rawFrom: "anthropic",
        },
        {
          role: "user",
          content: [{ kind: "tool_result", id: "call-1", output: "a.ts", isError: false }],
        },
      ],
      blocks: [{ kind: "user", text: "inspect" }],
    }, "checkpointed");
    const published = await store.publish(pending);

    const resumed = await SessionPersistence.resume(store, published.meta.id);
    assert.equal(resumed.conversation.activeNodeId, 1);
    assert.deepEqual(resumed.conversation.history, first.history);
    assert.equal(resumed.conversation.history.some((message) => message.raw !== undefined), false);
    const branch = complete(resumed.conversation, 1, "continue safely", "done");
    await resumed.persistence.checkpoint(branch);
    assert.equal(resumed.persistence.sessionId, published.meta.id);
    await resumed.persistence.close();

    const loaded = await store.load(published.meta.id);
    assert.equal(loaded.conversation.activeNodeId, 3);
    assert.equal(loaded.conversation.node(2)?.settlement, "checkpointed");
    assert.equal(loaded.conversation.node(3)?.parentId, 1);
    assert.equal((await store.list()).length, 1);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("timeline selection stays temporary until a real turn persists one branch", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const first = complete(ConversationTree.empty(), 0, "first", "one");
    const second = complete(first, 1, "second", "two");
    const published = await store.publish(second);

    const inspected = await SessionPersistence.resume(store, published.meta.id);
    const temporarySelection = inspected.conversation.select(1);
    assert.equal(temporarySelection.activeNodeId, 1);
    await inspected.persistence.close();

    const unchanged = await SessionPersistence.resume(store, published.meta.id);
    assert.equal(unchanged.conversation.activeNodeId, 2);
    assert.deepEqual(unchanged.conversation.history, second.history);

    const selected = unchanged.conversation.select(1);
    const branch = complete(selected, 1, "alternate", "three");
    await unchanged.persistence.checkpoint(branch);
    await unchanged.persistence.close();

    const resumed = await SessionPersistence.resume(store, published.meta.id);
    assert.equal(resumed.conversation.activeNodeId, 3);
    assert.equal(resumed.conversation.nodes.length, 3);
    assert.equal(resumed.conversation.node(2)?.parentId, 1);
    assert.equal(resumed.conversation.node(3)?.parentId, 1);
    assert.deepEqual(
      resumed.conversation.history.flatMap((message) => message.content)
        .filter((block) => block.kind === "text")
        .map((block) => block.text),
      ["first", "one", "alternate", "three"],
    );
    await resumed.persistence.close();
    const catalog = await store.list();
    assert.equal(catalog.length, 1);
    assert.equal(catalog[0]?.turns, 2);
    assert.equal(catalog[0]?.preview, "first");
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

function complete(
  conversation: ConversationTree,
  parentId: number,
  user: string,
  answer: string,
): ConversationTree {
  return conversation.commit({
    parentId,
    createdAt: "2026-09-01T10:00:00.000Z",
    identity: { providerId: "ollama", model: "deepseek-v4-flash:0731", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: user }] },
      { role: "assistant", content: [{ kind: "text", text: answer }] },
    ],
    blocks: [{ kind: "user", text: user }, { kind: "answer", text: answer }],
  }, "completed");
}

async function sessionFixture(): Promise<{
  root: string;
  workspace: string;
  sessions: string;
}> {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-session-runtime-"));
  const workspace = path.join(root, "workspace");
  const sessions = path.join(root, "data", "sessions");
  await mkdir(workspace);
  return { root, workspace, sessions };
}
