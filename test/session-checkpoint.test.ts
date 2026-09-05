import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdir, readdir, rename, rm, symlink, unlink, writeFile } from "node:fs/promises";
import * as path from "node:path";
import { ConversationTree } from "../src/conversation.ts";
import { encodeHead } from "../src/sessions/codec.ts";
import type { SessionLease } from "../src/sessions/lease.ts";
import { DurableSessionStore } from "../src/sessions/store.ts";
import {
  checkpointWithLease,
  revise,
  sessionFixture,
  turn,
} from "../dev/test-support/session-store.ts";

test("replaces only the active leaf from a verified snapshot", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const first = turn(ConversationTree.empty(), 0, "first", "one");
    const published = await store.publish(first);
    const revised = revise(first, "revised answer");

    const updated = await checkpointWithLease(store, published, revised);
    assert.equal(updated.head.sequence, 2);
    assert.equal(updated.head.nodeId, 1);
    assert.equal(updated.head.revision, 2);
    const loaded = await store.load(published.meta.id);
    assert.equal(loaded.conversation.activeNode?.revision, 2);
    assert.match(JSON.stringify(loaded.conversation.history), /revised answer/);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("checkpointing requires the exact store-issued session lease", async () => {
  const fixture = await sessionFixture();
  try {
    const owner = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const outsider = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const first = turn(ConversationTree.empty(), 0, "first", "one");
    const published = await owner.publish(first);
    const second = turn(first, 1, "second", "two");
    const lease = await owner.claim(published.meta.id);
    const structural = {
      id: published.meta.id,
      assertOwned: async () => undefined,
      close: async () => undefined,
    } as SessionLease;
    try {
      await assert.rejects(
        outsider.checkpoint(published, second, lease),
        /checkpoint requires exclusive ownership/,
      );
      await assert.rejects(
        owner.checkpoint(published, second, structural),
        /checkpoint requires exclusive ownership/,
      );
      const unchanged = await owner.load(published.meta.id);
      assert.equal(unchanged.head.sequence, 1);

      const updated = await owner.checkpoint(published, second, lease);
      assert.equal(updated.head.sequence, 2);
    } finally {
      await lease.close();
    }
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("rejects a checkpoint after the verified head changes", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const first = turn(ConversationTree.empty(), 0, "first", "one");
    const published = await store.publish(first);
    const directory = path.join(fixture.sessions, store.workspaceDigest, published.meta.id);
    await writeFile(path.join(directory, "head.json"), encodeHead({
      ...published.head,
      sequence: published.head.sequence + 1,
      updatedAt: "2026-09-01T12:00:00.000Z",
    }), "utf8");

    await assert.rejects(
      checkpointWithLease(store, published, turn(first, 1, "second", "two")),
      /head changed after its verified snapshot/,
    );
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("a failed checkpoint releases its generation before the next attempt", async () => {
  const fixture = await sessionFixture();
  let failOnce = true;
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions, {
      afterCheckpointLease() {
        if (!failOnce) return;
        failOnce = false;
        throw new Error("injected checkpoint failure");
      },
    });
    const first = turn(ConversationTree.empty(), 0, "first", "one");
    const published = await store.publish(first);
    const second = turn(first, 1, "second", "two");

    const lease = await store.claim(published.meta.id);
    let recovered;
    try {
      await assert.rejects(
        store.checkpoint(published, second, lease),
        /injected checkpoint failure/,
      );
      recovered = await store.checkpoint(published, second, lease);
    } finally {
      await lease.close();
    }

    assert.equal(recovered.head.sequence, 2);
    assert.equal((await store.load(published.meta.id)).head.sequence, 2);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("a delayed checkpoint cannot commit after another writer advances the head", async () => {
  const fixture = await sessionFixture();
  let reachedBarrier!: () => void;
  let leaveBarrier!: () => void;
  const barrier = new Promise<void>((resolve) => {
    reachedBarrier = resolve;
  });
  const resume = new Promise<void>((resolve) => {
    leaveBarrier = resolve;
  });
  let delayFirst = true;
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions, {
      async beforeCheckpointLease() {
        if (!delayFirst) return;
        delayFirst = false;
        reachedBarrier();
        await resume;
      },
    });
    const first = turn(ConversationTree.empty(), 0, "first", "one");
    const published = await store.publish(first);
    const lease = await store.claim(published.meta.id);
    let winner;
    try {
      const delayed = store.checkpoint(
        published,
        turn(first, 1, "loser", "stale"),
        lease,
      );
      await barrier;
      winner = await store.checkpoint(
        published,
        turn(first, 1, "winner", "current"),
        lease,
      );
      leaveBarrier();
      await assert.rejects(delayed, /head changed after its verified snapshot/);
    } finally {
      leaveBarrier();
      await lease.close();
    }
    assert.equal(winner.head.sequence, 2);
    const loaded = await store.load(published.meta.id);
    assert.match(JSON.stringify(loaded.conversation.history), /winner/);
    assert.doesNotMatch(JSON.stringify(loaded.conversation.history), /loser/);
  } finally {
    leaveBarrier?.();
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("checkpoint refuses a replaced node directory", async (context) => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const first = turn(ConversationTree.empty(), 0, "first", "one");
    const published = await store.publish(first);
    const directory = path.join(fixture.sessions, store.workspaceDigest, published.meta.id);
    const nodes = path.join(directory, "nodes");
    const parked = path.join(directory, "parked-nodes");
    const outside = path.join(fixture.root, "outside");
    await mkdir(outside);
    await rename(nodes, parked);
    try {
      await symlink(outside, nodes, process.platform === "win32" ? "junction" : "dir");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "EPERM") {
        context.skip("creating directory links is unavailable for this account");
        return;
      }
      throw error;
    }

    await assert.rejects(
      checkpointWithLease(store, published, turn(first, 1, "second", "two")),
      /session node directory is not a direct directory/,
    );
    assert.deepEqual(await readdir(outside), []);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("checkpoint refuses a replaced session directory before creating its lock", async (context) => {
  const fixture = await sessionFixture();
  const outside = path.join(fixture.root, "outside-session");
  const probe = path.join(fixture.root, "link-probe");
  await mkdir(outside);
  try {
    await symlink(outside, probe, process.platform === "win32" ? "junction" : "dir");
    await unlink(probe);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "EPERM") {
      context.skip("creating directory links is unavailable for this account");
      await rm(fixture.root, { recursive: true, force: true });
      return;
    }
    throw error;
  }

  let replace = false;
  let directory = "";
  let parked = "";
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions, {
      async beforeCheckpointLease() {
        if (!replace) return;
        await rename(directory, parked);
        await symlink(
          outside,
          directory,
          process.platform === "win32" ? "junction" : "dir",
        );
      },
    });
    const first = turn(ConversationTree.empty(), 0, "first", "one");
    const published = await store.publish(first);
    directory = path.join(fixture.sessions, store.workspaceDigest, published.meta.id);
    parked = `${directory}.parked`;
    const lease = await store.claim(published.meta.id);
    replace = true;
    try {
      await assert.rejects(
        store.checkpoint(published, turn(first, 1, "second", "two"), lease),
        /lease is no longer owned|directory changed|not a direct directory/,
      );
      assert.deepEqual(await readdir(outside), []);
    } finally {
      await unlink(directory).catch(() => undefined);
      await rename(parked, directory).catch(() => undefined);
      await lease.close().catch(() => undefined);
    }
  } finally {
    await unlink(probe).catch(() => undefined);
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("rejects rematerialized shared history instead of trusting equal mutable data", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const first = turn(ConversationTree.empty(), 0, "first", "one");
    const published = await store.publish(first);
    const rematerialized = ConversationTree.restore(first.nodes, first.activeNodeId);

    await assert.rejects(
      checkpointWithLease(store, published, turn(rematerialized, 1, "second", "two")),
      /rewrites prior conversation history/,
    );
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("rejects unpublished nodes hidden behind an unchanged active head", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const first = turn(ConversationTree.empty(), 0, "first", "one");
    const published = await store.publish(first);
    const hidden = turn(first, 1, "hidden", "two").select(1);

    await assert.rejects(
      checkpointWithLease(store, published, hidden),
      /does not extend its durable tree/,
    );
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});
