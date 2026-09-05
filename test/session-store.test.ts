import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdir, open, rm, writeFile } from "node:fs/promises";
import * as path from "node:path";
import { ConversationTree } from "../src/conversation.ts";
import { DurableSessionStore } from "../src/sessions/store.ts";
import {
  checkpointWithLease,
  sessionFixture,
  stripRaw,
  turn,
} from "../dev/test-support/session-store.ts";

const boundaryClusters = [
  ["emoji", String.fromCodePoint(0x1f600)],
  ["combining mark", `e${String.fromCodePoint(0x0301)}`],
  [
    "ZWJ sequence",
    `${String.fromCodePoint(0x1f469)}${String.fromCodePoint(0x200d)}` +
      String.fromCodePoint(0x1f4bb),
  ],
  ["flag", `${String.fromCodePoint(0x1f1ee)}${String.fromCodePoint(0x1f1f9)}`],
] as const;

test("publishes, updates, lists, and reloads a workspace-scoped session", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    let conversation = turn(ConversationTree.empty(), 0, "first\nquestion", "one");
    const published = await store.publish(conversation);

    conversation = turn(conversation, 1, "second", "two");
    const updated = await checkpointWithLease(store, published, conversation);
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

test("reloads conversation nodes in order across bounded read batches", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    let conversation = ConversationTree.empty();
    for (let index = 0; index < 10; index++) {
      conversation = turn(
        conversation,
        conversation.activeNodeId,
        `question ${index}`,
        `answer ${index}`,
      );
    }

    const published = await store.publish(conversation);
    const loaded = await store.load(published.meta.id);

    assert.deepEqual(
      loaded.conversation.nodes.map((node) => node.id),
      [1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
    );
    assert.deepEqual(loaded.conversation.history, conversation.history.map(stripRaw));
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("rejects aggregate sparse node storage before parsing it", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const published = await store.publish(turn(ConversationTree.empty(), 0, "first", "one"));
    const nodes = path.join(
      fixture.sessions,
      store.workspaceDigest,
      published.meta.id,
      "nodes",
    );
    for (let id = 1; id <= 10; id++) {
      const file = path.join(nodes, `${String(id).padStart(6, "0")}.json`);
      const handle = await open(file, "w");
      try {
        await handle.truncate(20 * 1_024 * 1_024);
      } finally {
        await handle.close();
      }
    }

    await assert.rejects(store.load(published.meta.id), /aggregate storage limit/);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("catalogue previews end before a grapheme that crosses their boundary", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    for (const [name, cluster] of boundaryClusters) {
      const prefix = "a".repeat(160 - cluster.length + 1);
      const published = await store.publish(
        turn(ConversationTree.empty(), 0, `${prefix}${cluster}tail`, "done"),
      );
      const preview = (await store.list()).find((entry) => entry.id === published.meta.id)?.preview;

      assert.equal(preview, prefix, `${name} was split`);
      assert.equal(preview?.isWellFormed(), true, `${name} produced malformed UTF-16`);
    }
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

test("catalogue keeps the most recently updated session beyond 128 entries", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const original = turn(ConversationTree.empty(), 0, "oldest session", "one");
    const first = await store.publish(original);
    await new Promise<void>((resolve) => setTimeout(resolve, 5));
    for (let index = 1; index < 130; index++) {
      await store.publish(turn(ConversationTree.empty(), 0, `session ${index}`, "one"));
    }
    await new Promise<void>((resolve) => setTimeout(resolve, 5));
    await checkpointWithLease(store, first, turn(original, 1, "updated last", "two"));

    const catalog = await store.list(64);
    assert.equal(catalog.length, 64);
    assert.equal(catalog[0]?.id, first.meta.id);
    assert.equal(catalog[0]?.turns, 2);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});
