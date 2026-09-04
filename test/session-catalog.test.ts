import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { ConversationTree } from "../src/conversation.ts";
import {
  advanceSessionCatalog,
  decodeSessionCatalog,
  encodeSessionCatalog,
  sessionCatalog,
  SESSION_CATALOG_FILE,
  SESSION_CHECKPOINT_FILE,
} from "../src/sessions/catalog.ts";
import { encodeNode, SESSION_SCHEMA } from "../src/sessions/codec.ts";
import type { SessionHead, SessionMeta } from "../src/sessions/codec.ts";
import { DurableSessionStore } from "../src/sessions/store.ts";
import type { Message } from "../src/types.ts";

test("session catalogue codec is strict, bounded, and head-tied", () => {
  const tree = turn(ConversationTree.empty(), 0, "first question", "done");
  const createdAt = "2026-09-01T10:00:00.000Z";
  const meta = metadata(createdAt);
  const head = sessionHead(1, 1, 0, "2026-09-01T10:01:00.000Z");
  const catalog = sessionCatalog(meta, head, tree);

  assert.deepEqual(decodeSessionCatalog(JSON.parse(encodeSessionCatalog(catalog))), catalog);
  assert.equal(catalog.turns, 1);
  assert.equal(catalog.preview, "first question");
  assert.throws(
    () => decodeSessionCatalog({ ...catalog, future: true }),
    /invalid or unsupported/,
  );
  assert.throws(
    () => decodeSessionCatalog({ ...catalog, preview: "a".repeat(161) }),
    /invalid or unsupported/,
  );
  assert.throws(
    () => decodeSessionCatalog({ ...catalog, turns: 0 }),
    /invalid or unsupported/,
  );
});

test("incremental catalogue falls back when an unfinished head joins the visible path", () => {
  const createdAt = "2026-09-01T10:00:00.000Z";
  const meta = metadata(createdAt);
  const first = turn(ConversationTree.empty(), 0, "first", "done");
  const firstCatalog = sessionCatalog(
    meta,
    sessionHead(1, 1, 0, "2026-09-01T10:01:00.000Z"),
    first,
  );
  const unfinished = first.commit({
    parentId: 1,
    createdAt,
    identity: { providerId: "ollama", model: "m", effort: "high" },
    messages: completed("unfinished", "partial"),
    blocks: [],
  }, "checkpointed");
  const unfinishedHead = sessionHead(2, 2, 1, "2026-09-01T10:02:00.000Z");
  const unfinishedCatalog = advanceSessionCatalog(
    firstCatalog,
    meta,
    unfinishedHead,
    unfinished,
  );
  const third = turn(unfinished, 2, "third", "done");
  const thirdHead = sessionHead(3, 3, 2, "2026-09-01T10:03:00.000Z");

  assert.equal(unfinishedCatalog.turns, 1);
  assert.equal(advanceSessionCatalog(unfinishedCatalog, meta, thirdHead, third).turns, 3);
});

test("catalogue backfills and repairs bounded summaries for legacy sessions", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const published = await store.publish(turn(ConversationTree.empty(), 0, "legacy", "one"));
    const directory = sessionDirectory(fixture.sessions, store, published.meta.id);
    await rm(path.join(directory, SESSION_CATALOG_FILE));

    assert.deepEqual(await store.list(), [{
      id: published.meta.id,
      createdAt: published.meta.createdAt,
      updatedAt: published.head.updatedAt,
      turns: 1,
      preview: "legacy",
      active: false,
    }]);
    const rebuilt = decodeSessionCatalog(JSON.parse(
      await readFile(path.join(directory, SESSION_CATALOG_FILE), "utf8"),
    ));
    assert.equal(rebuilt.head.sequence, published.head.sequence);
    assert.equal(rebuilt.resumeNodeId, 1);

    await writeFile(path.join(directory, SESSION_CATALOG_FILE), "{", "utf8");
    assert.equal((await store.list())[0]?.id, published.meta.id);
    const repaired = await readFile(path.join(directory, SESSION_CATALOG_FILE), "utf8");
    assert.doesNotThrow(() => decodeSessionCatalog(JSON.parse(repaired)));
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("catalogue repairs a summary that no longer matches the durable head", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const first = turn(ConversationTree.empty(), 0, "first", "one");
    const published = await store.publish(first);
    const lease = await store.claim(published.meta.id);
    let updated;
    try {
      updated = await store.checkpoint(
        published,
        turn(first, 1, "second", "two"),
        lease,
      );
    } finally {
      await lease.close();
    }
    const directory = sessionDirectory(fixture.sessions, store, published.meta.id);
    await writeFile(
      path.join(directory, SESSION_CATALOG_FILE),
      encodeSessionCatalog(published.catalog),
      "utf8",
    );

    const listed = (await store.list())[0];
    assert.equal(listed?.turns, 2);
    assert.equal(listed?.updatedAt, updated.head.updatedAt);
    const repaired = decodeSessionCatalog(JSON.parse(
      await readFile(path.join(directory, SESSION_CATALOG_FILE), "utf8"),
    ));
    assert.equal(repaired.head.sequence, updated.head.sequence);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("catalogue recovers a checkpoint left ahead of its head marker", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const first = turn(ConversationTree.empty(), 0, "first", "one");
    const published = await store.publish(first);
    const second = turn(first, 1, "second", "two");
    const candidate = second.activeNode as NonNullable<typeof second.activeNode>;
    const now = "2026-09-01T12:00:00.000Z";
    const directory = sessionDirectory(fixture.sessions, store, published.meta.id);
    await writeFile(path.join(directory, SESSION_CHECKPOINT_FILE), "dead:fixture", "utf8");
    await writeFile(
      path.join(directory, "nodes", "000002.json"),
      encodeNode(candidate, 2, now),
      "utf8",
    );

    const listed = (await store.list())[0];
    assert.equal(listed?.turns, 2);
    assert.equal(listed?.updatedAt, now);
    const rebuilt = decodeSessionCatalog(JSON.parse(
      await readFile(path.join(directory, SESSION_CATALOG_FILE), "utf8"),
    ));
    assert.equal(rebuilt.head.sequence, 2);
    await assert.rejects(
      readFile(path.join(directory, SESSION_CHECKPOINT_FILE), "utf8"),
      (error: NodeJS.ErrnoException) => error.code === "ENOENT",
    );
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("catalogue summaries do not weaken strict loading of a selected session", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const published = await store.publish(turn(ConversationTree.empty(), 0, "first", "one"));
    const directory = sessionDirectory(fixture.sessions, store, published.meta.id);
    await writeFile(path.join(directory, "nodes", "000001.json"), "{}", "utf8");

    assert.equal((await store.list())[0]?.id, published.meta.id);
    await assert.rejects(store.load(published.meta.id), /invalid or unsupported/);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("catalogue keeps failed and interrupted turns resumable", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    for (const [settlement, tone] of [
      ["failed", "error"],
      ["interrupted", "warn"],
    ] as const) {
      const conversation = ConversationTree.empty().commit({
        parentId: 0,
        createdAt: "2026-09-01T10:00:00.000Z",
        identity: { providerId: "ollama", model: "m", effort: "high" },
        messages: completed(settlement, "partial answer"),
        blocks: [{ kind: "user", text: settlement }],
        failure: { text: `${settlement} safely`, tone },
      }, settlement);
      await store.publish(conversation);
    }

    const catalog = await store.list();
    assert.equal(catalog.length, 2);
    assert.deepEqual(new Set(catalog.map((entry) => entry.preview)), new Set([
      "failed",
      "interrupted",
    ]));
    assert.equal(catalog.every((entry) => entry.turns === 1), true);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("a live checkpoint marker cannot appear as an idle resume candidate", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const published = await store.publish(turn(ConversationTree.empty(), 0, "first", "one"));
    const directory = sessionDirectory(fixture.sessions, store, published.meta.id);
    await writeFile(
      path.join(directory, SESSION_CHECKPOINT_FILE),
      `${process.pid}:in-flight-fixture`,
      "utf8",
    );

    assert.equal((await store.list())[0]?.active, true);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

function metadata(createdAt: string): SessionMeta {
  return Object.freeze({
    version: SESSION_SCHEMA,
    id: "session-1",
    workspaceRoot: "C:\\work",
    workspaceDigest: "a".repeat(64),
    createdAt,
  });
}

function sessionHead(
  sequence: number,
  nodeId: number,
  parentId: number,
  updatedAt: string,
): SessionHead {
  return Object.freeze({
    version: SESSION_SCHEMA,
    sequence,
    nodeId,
    parentId,
    revision: 1,
    updatedAt,
  });
}

function turn(
  conversation: ConversationTree,
  parentId: number,
  user: string,
  answer: string,
): ConversationTree {
  return conversation.commit({
    parentId,
    createdAt: "2026-09-01T10:00:00.000Z",
    identity: { providerId: "ollama", model: "m", effort: "high" },
    messages: completed(user, answer),
    blocks: [],
  }, "completed");
}

function completed(user: string, answer: string): Message[] {
  return [
    { role: "user", content: [{ kind: "text", text: user }] },
    { role: "assistant", content: [{ kind: "text", text: answer }] },
  ];
}

async function sessionFixture(): Promise<{
  root: string;
  workspace: string;
  sessions: string;
}> {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-session-catalog-"));
  const workspace = path.join(root, "workspace");
  const sessions = path.join(root, "data", "sessions");
  await mkdir(workspace);
  return { root, workspace, sessions };
}

function sessionDirectory(
  sessions: string,
  store: DurableSessionStore,
  id: string,
): string {
  return path.join(sessions, store.workspaceDigest, id);
}
