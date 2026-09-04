import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { once } from "node:events";
import {
  mkdtemp,
  mkdir,
  open,
  readFile,
  readdir,
  rename,
  rm,
  symlink,
  unlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { ConversationTree } from "../src/conversation.ts";
import { encodeHead, encodeNode } from "../src/sessions/codec.ts";
import {
  createLeaseDirectory,
  leaseFromGeneration,
} from "../src/sessions/lease.ts";
import type { SessionLease } from "../src/sessions/lease.ts";
import { DurableSessionStore } from "../src/sessions/store.ts";
import type { Message } from "../src/types.ts";

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

test("a live checkpoint prevents claiming an otherwise idle session", async () => {
  const fixture = await sessionFixture();
  let checkpoint: ReturnType<typeof leaseFromGeneration> | undefined;
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const first = turn(ConversationTree.empty(), 0, "first", "one");
    const published = await store.publish(first);
    const second = turn(first, 1, "second", "two");
    const directory = path.join(fixture.sessions, store.workspaceDigest, published.meta.id);
    await writeFile(
      path.join(directory, "nodes", "000002.json"),
      encodeNode(
        second.activeNode as NonNullable<typeof second.activeNode>,
        2,
        "2026-09-01T12:00:00.000Z",
      ),
      "utf8",
    );
    const checkpointDirectory = path.join(directory, ".checkpoint");
    const generation = await createLeaseDirectory(
      checkpointDirectory,
      `${process.pid}:${randomUUID()}`,
    );
    checkpoint = leaseFromGeneration(checkpointDirectory, generation);

    await assert.rejects(store.claim(published.meta.id), /live checkpoint/);
    await assert.rejects(store.load(published.meta.id), /exclusive ownership/);
    assert.match(await readFile(path.join(directory, "head.json"), "utf8"), /"sequence": 1/);
  } finally {
    await checkpoint?.release();
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("claim removes only an observed dead checkpoint generation", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const published = await store.publish(turn(ConversationTree.empty(), 0, "first", "one"));
    const directory = path.join(fixture.sessions, store.workspaceDigest, published.meta.id);
    const child = spawn(process.execPath, ["-e", ""], { stdio: "ignore", windowsHide: true });
    const pid = child.pid as number;
    await once(child, "exit");
    await createLeaseDirectory(
      path.join(directory, ".checkpoint"),
      `${pid}:${randomUUID()}`,
    );

    const lease = await store.claim(published.meta.id);
    await lease.assertOwned();
    await lease.close();
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

test("a stale legacy active marker fails closed with repair guidance", async () => {
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
    await writeFile(active, "dead:legacy-fixture", "utf8");

    await assert.rejects(store.claim(published.meta.id), /stale legacy active marker/);
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

function revise(conversation: ConversationTree, answer: string): ConversationTree {
  const active = conversation.activeNode as NonNullable<typeof conversation.activeNode>;
  return conversation.commit({
    nodeId: active.id,
    parentId: active.parentId,
    createdAt: active.createdAt,
    identity: active.identity,
    messages: completed("first", answer),
    blocks: [{ kind: "user", text: "first" }, { kind: "answer", text: answer }],
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

async function loadWithLease(
  store: DurableSessionStore,
  id: string,
): Promise<Awaited<ReturnType<DurableSessionStore["load"]>>> {
  const lease = await store.claim(id);
  try {
    return await store.load(id, lease);
  } finally {
    await lease.close();
  }
}

async function checkpointWithLease(
  store: DurableSessionStore,
  snapshot: Parameters<DurableSessionStore["checkpoint"]>[0],
  conversation: ConversationTree,
): Promise<Awaited<ReturnType<DurableSessionStore["checkpoint"]>>> {
  const lease = await store.claim(snapshot.meta.id);
  try {
    return await store.checkpoint(snapshot, conversation, lease);
  } finally {
    await lease.close();
  }
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
