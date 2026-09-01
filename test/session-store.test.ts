import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { ConversationTree } from "../src/conversation.ts";
import { encodeNode } from "../src/sessions/codec.ts";
import { DurableSessionStore } from "../src/sessions/store.ts";
import type { Message } from "../src/types.ts";

test("publishes, updates, lists, and reloads a workspace-scoped session", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    let conversation = turn(ConversationTree.empty(), 0, "first\nquestion", "one");
    const published = await store.publish(conversation);

    conversation = turn(conversation, 1, "second", "two");
    const updated = await store.checkpoint(published.meta.id, conversation);
    assert.equal(updated.head.sequence, 2);
    assert.equal(updated.head.nodeId, 2);

    const loaded = await store.load(published.meta.id);
    assert.deepEqual(loaded.conversation.history, conversation.history.map(stripRaw));
    assert.equal(loaded.conversation.transcript.length, 4);
    assert.deepEqual(await store.list(), [{
      id: published.meta.id,
      createdAt: published.meta.createdAt,
      updatedAt: updated.head.updatedAt,
      turns: 2,
      preview: "first question",
      active: false,
    }]);

    const otherWorkspace = path.join(fixture.root, "other");
    await mkdir(otherWorkspace);
    const other = await DurableSessionStore.open(otherWorkspace, fixture.sessions);
    assert.deepEqual(await other.list(), []);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("recovers the one node mutation written before its head", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const first = turn(ConversationTree.empty(), 0, "first", "one");
    const published = await store.publish(first);
    const second = turn(first, 1, "second", "two");
    const candidate = second.activeNode as NonNullable<typeof second.activeNode>;
    const now = "2026-09-01T12:00:00.000Z";
    const directory = path.join(
      fixture.sessions,
      store.workspaceDigest,
      published.meta.id,
    );
    await writeFile(
      path.join(directory, "nodes", "000002.json"),
      encodeNode(candidate, 2, now),
      "utf8",
    );

    const recovered = await store.load(published.meta.id);
    assert.equal(recovered.head.sequence, 2);
    assert.equal(recovered.head.nodeId, 2);
    assert.equal(recovered.head.updatedAt, now);
    assert.deepEqual(recovered.conversation.history, second.history.map(stripRaw));
    assert.match(await readFile(path.join(directory, "head.json"), "utf8"), /"sequence": 2/);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("recovers a new branch written before the session head", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const first = turn(ConversationTree.empty(), 0, "first", "one");
    const second = turn(first, 1, "second", "two");
    const published = await store.publish(second);
    const branch = turn(second.select(1), 1, "alternate", "branch");
    const candidate = branch.activeNode as NonNullable<typeof branch.activeNode>;
    const now = "2026-09-01T12:00:00.000Z";
    const directory = path.join(fixture.sessions, store.workspaceDigest, published.meta.id);
    await writeFile(
      path.join(directory, "nodes", "000003.json"),
      encodeNode(candidate, 3, now),
      "utf8",
    );

    const recovered = await store.load(published.meta.id);
    assert.equal(recovered.head.nodeId, 3);
    assert.equal(recovered.head.parentId, 1);
    assert.deepEqual(recovered.conversation.history, branch.history.map(stripRaw));
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("rejects a recovered revision that silently changes the head parent", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const first = turn(ConversationTree.empty(), 0, "first", "one");
    const second = turn(first, 1, "second", "two");
    const published = await store.publish(second);
    const current = second.activeNode as NonNullable<typeof second.activeNode>;
    const directory = path.join(fixture.sessions, store.workspaceDigest, published.meta.id);
    await writeFile(
      path.join(directory, "nodes", "000002.json"),
      encodeNode({ ...current, parentId: 0, revision: 2 }, 3, "2026-09-01T12:00:00.000Z"),
      "utf8",
    );

    await assert.rejects(store.load(published.meta.id), /cannot be recovered safely/);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("catalogue hides a session until it has a completed turn", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const pending = ConversationTree.empty().commit({
      parentId: 0,
      createdAt: "2026-09-01T10:00:00.000Z",
      identity: { providerId: "anthropic", model: "claude", effort: "medium" },
      messages: [
        { role: "user", content: [{ kind: "text", text: "inspect" }] },
        {
          role: "assistant",
          content: [{ kind: "tool_call", id: "call-1", name: "list_dir", input: { path: "." } }],
        },
        {
          role: "user",
          content: [{ kind: "tool_result", id: "call-1", output: "a.ts", isError: false }],
        },
      ],
      blocks: [{ kind: "user", text: "inspect" }],
    }, "checkpointed");
    await store.publish(pending);

    assert.deepEqual(await store.list(), []);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("a live lease prevents the same durable session from opening twice", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const published = await store.publish(turn(ConversationTree.empty(), 0, "first", "one"));
    const lease = await store.claim(published.meta.id);
    await assert.rejects(store.claim(published.meta.id), /already open/);
    assert.equal((await store.list())[0]?.active, true);
    await lease.close();
    const next = await store.claim(published.meta.id);
    await next.close();
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("a freshly published session is visible only with its lease already held", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const published = await store.publish(
      turn(ConversationTree.empty(), 0, "first", "one"),
      true,
    );

    assert.equal((await store.list())[0]?.active, true);
    await assert.rejects(store.claim(published.meta.id), /already open/);
    await published.lease.close();
    assert.equal((await store.list())[0]?.active, false);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("an unsafe lease file cannot become a resume candidate or unbounded read", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const published = await store.publish(turn(ConversationTree.empty(), 0, "first", "one"));
    const active = path.join(
      fixture.sessions,
      store.workspaceDigest,
      published.meta.id,
      "active",
    );
    await writeFile(active, "x".repeat(257), "utf8");

    assert.deepEqual(await store.list(), []);
    await assert.rejects(store.claim(published.meta.id), /lease is unsafe or too large/);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("catalogue skips malformed sessions without crossing the workspace boundary", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const bucket = path.join(fixture.sessions, store.workspaceDigest);
    await mkdir(path.join(bucket, "broken", "nodes"), { recursive: true });
    await writeFile(path.join(bucket, "broken", "meta.json"), "{}", "utf8");
    await writeFile(path.join(bucket, "broken", "head.json"), "{}", "utf8");
    assert.deepEqual(await store.list(), []);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

function turn(
  conversation: ConversationTree,
  parentId: number,
  user: string,
  answer: string,
): ConversationTree {
  return conversation.commit({
    parentId,
    createdAt: "2026-09-01T10:00:00.000Z",
    identity: { providerId: "openai-codex", model: "gpt-5.6-terra", effort: "medium" },
    messages: completed(user, answer),
    blocks: [{ kind: "user", text: user }, { kind: "answer", text: answer }],
  }, "completed");
}

function completed(user: string, answer: string): Message[] {
  return [
    { role: "user", content: [{ kind: "text", text: user }] },
    {
      role: "assistant",
      content: [{ kind: "text", text: answer }],
      raw: { encrypted: answer },
      rawFrom: "openai-codex",
    },
  ];
}

function stripRaw(message: Message): Message {
  return {
    role: message.role,
    content: message.content,
    ...(message.usage === undefined ? {} : { usage: message.usage }),
  };
}

async function sessionFixture(): Promise<{
  root: string;
  workspace: string;
  sessions: string;
}> {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-sessions-"));
  const workspace = path.join(root, "workspace");
  const sessions = path.join(root, "data", "sessions");
  await mkdir(workspace);
  return { root, workspace, sessions };
}
