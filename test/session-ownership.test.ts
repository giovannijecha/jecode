import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { once } from "node:events";
import { readFile, rm, writeFile } from "node:fs/promises";
import * as path from "node:path";
import { ConversationTree } from "../src/conversation.ts";
import { encodeNode } from "../src/sessions/codec.ts";
import { createLeaseDirectory, leaseFromGeneration } from "../src/sessions/lease.ts";
import { DurableSessionStore } from "../src/sessions/store.ts";
import { sessionFixture, turn } from "../dev/test-support/session-store.ts";

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
