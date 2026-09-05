import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile, rm, writeFile } from "node:fs/promises";
import * as path from "node:path";
import { ConversationTree } from "../src/conversation.ts";
import { encodeNode } from "../src/sessions/codec.ts";
import { DurableSessionStore } from "../src/sessions/store.ts";
import {
  checkpointWithLease,
  loadWithLease,
  revise,
  sessionFixture,
  stripRaw,
  turn,
} from "../dev/test-support/session-store.ts";

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

    await assert.rejects(
      checkpointWithLease(store, published, second),
      /incomplete node outside its verified snapshot/,
    );

    await assert.rejects(
      store.load(published.meta.id),
      /session recovery requires exclusive ownership/,
    );
    assert.match(await readFile(path.join(directory, "head.json"), "utf8"), /"sequence": 1/);
    const recovered = await loadWithLease(store, published.meta.id);
    assert.equal(recovered.head.sequence, 2);
    assert.equal(recovered.head.nodeId, 2);
    assert.equal(recovered.head.updatedAt, now);
    assert.deepEqual(recovered.conversation.history, second.history.map(stripRaw));
    assert.match(await readFile(path.join(directory, "head.json"), "utf8"), /"sequence": 2/);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("recovery rejects a lease issued by another store instance", async () => {
  const fixture = await sessionFixture();
  try {
    const firstStore = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const secondStore = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const first = turn(ConversationTree.empty(), 0, "first", "one");
    const published = await firstStore.publish(first);
    const second = turn(first, 1, "second", "two");
    const directory = path.join(
      fixture.sessions,
      firstStore.workspaceDigest,
      published.meta.id,
    );
    await writeFile(
      path.join(directory, "nodes", "000002.json"),
      encodeNode(
        second.activeNode as NonNullable<typeof second.activeNode>,
        2,
        "2026-09-01T12:00:00.000Z",
      ),
      "utf8",
    );
    const lease = await firstStore.claim(published.meta.id);
    try {
      await assert.rejects(
        secondStore.load(published.meta.id, lease),
        /recovery requires exclusive ownership/,
      );
    } finally {
      await lease.close();
    }
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("recovers an active-leaf revision written before its head", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const first = turn(ConversationTree.empty(), 0, "first", "one");
    const published = await store.publish(first);
    const revised = revise(first, "recovered revision");
    const candidate = revised.activeNode as NonNullable<typeof revised.activeNode>;
    const now = "2026-09-01T12:00:00.000Z";
    const directory = path.join(fixture.sessions, store.workspaceDigest, published.meta.id);
    await writeFile(
      path.join(directory, "nodes", "000001.json"),
      encodeNode(candidate, 2, now),
      "utf8",
    );

    const recovered = await loadWithLease(store, published.meta.id);
    assert.equal(recovered.head.sequence, 2);
    assert.equal(recovered.head.revision, 2);
    assert.match(JSON.stringify(recovered.conversation.history), /recovered revision/);
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

    const recovered = await loadWithLease(store, published.meta.id);
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

for (const mutation of ["revision", "new node"] as const) {
  test(`invalid recovered ${mutation} leaves durable files unchanged`, async () => {
    const fixture = await sessionFixture();
    try {
      const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
      const first = turn(ConversationTree.empty(), 0, "first", "one");
      const published = await store.publish(first);
      const next = mutation === "revision"
        ? revise(first, "revised") : turn(first, 1, "second", "two");
      const active = next.activeNode as NonNullable<typeof next.activeNode>;
      const directory = path.join(fixture.sessions, store.workspaceDigest, published.meta.id);
      const nodeFile = path.join(directory, "nodes", `${String(active.id).padStart(6, "0")}.json`);
      const invalid = encodeNode({ ...active, messages: [] }, 2, "2026-09-01T12:00:00.000Z");
      await writeFile(nodeFile, invalid, "utf8");
      const headFile = path.join(directory, "head.json");
      const catalogFile = path.join(directory, "catalog.json");
      const headBefore = await readFile(headFile, "utf8");
      const catalogBefore = await readFile(catalogFile, "utf8");

      await assert.rejects(loadWithLease(store, published.meta.id), /turn checkpoint is invalid/);

      assert.equal(await readFile(headFile, "utf8"), headBefore);
      assert.equal(await readFile(catalogFile, "utf8"), catalogBefore);
      assert.equal(await readFile(nodeFile, "utf8"), invalid);
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  });
}
