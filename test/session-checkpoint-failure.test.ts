import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { syncBuiltinESMExports } from "node:module";
import * as path from "node:path";
import { ConversationTree } from "../src/conversation.ts";
import { decodeHead, decodeNode, encodeNode } from "../src/sessions/codec.ts";
import { decodeSessionCatalog } from "../src/sessions/catalog.ts";
import { SessionPersistence } from "../src/sessions/runtime.ts";
import { DurableSessionStore } from "../src/sessions/store.ts";
import { revise, sessionFixture, stripRaw, turn } from "../dev/test-support/session-store.ts";

test("resume rebuilds an old index hiding a completed node without a checkpoint marker", async () => {
  const fixture = await sessionFixture();
  try {
    const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
    const first = turn(ConversationTree.empty(), 0, "first", "one");
    const unfinished = ConversationTree.restore([
      { ...first.activeNode!, settlement: "checkpointed" },
    ], 1);
    const published = await store.publish(unfinished);
    const directory = path.join(fixture.sessions, store.workspaceDigest, published.meta.id);
    const catalogFile = path.join(directory, "catalog.json");
    const completed = revise(unfinished, "recovered answer");
    const nodeFile = path.join(directory, "nodes", "000001.json");
    const node = encodeNode(completed.activeNode!, 2, "2026-09-01T12:00:00.000Z");
    await fs.writeFile(catalogFile, JSON.stringify({ ...published.catalog, version: 1 }), "utf8");
    await fs.writeFile(nodeFile, node, "utf8");
    const metaBefore = await fs.readFile(path.join(directory, "meta.json"), "utf8");

    const candidates = await SessionPersistence.candidates(store);

    assert.deepEqual(candidates.map((entry) => entry.id), [published.meta.id]);
    const rebuilt = decodeSessionCatalog(JSON.parse(await fs.readFile(catalogFile, "utf8")));
    assert.equal(rebuilt.version, 2);
    assert.equal(rebuilt.head.sequence, 2);
    assert.equal(rebuilt.resumeNodeId, 1);
    assert.equal(await fs.readFile(nodeFile, "utf8"), node);
    assert.equal(await fs.readFile(path.join(directory, "meta.json"), "utf8"), metaBefore);
    assert.deepEqual((await store.load(published.meta.id)).conversation.history, completed.history.map(stripRaw));
  } finally {
    await fs.rm(fixture.root, { recursive: true, force: true });
  }
});

for (const boundary of ["catalogue", "node", "head"] as const) {
  test(`resume listing recovers after a failed ${boundary} replacement`, async (context) => {
    const fixture = await sessionFixture();
    const rename = fs.rename;
    let rejectedTarget: string | undefined;
    context.mock.method(fs, "rename", async (source: string, target: string) => {
      if (target === rejectedTarget) {
        throw Object.assign(new Error("injected replacement failure"), { code: "EACCES" });
      }
      return rename(source, target);
    });
    syncBuiltinESMExports();
    try {
      const store = await DurableSessionStore.open(fixture.workspace, fixture.sessions);
      const first = turn(ConversationTree.empty(), 0, "first", "one");
      const unfinished = ConversationTree.restore([
        { ...first.activeNode!, settlement: "checkpointed" },
      ], 1);
      const persistence = SessionPersistence.fresh(store);
      try {
        await persistence.checkpoint(unfinished);
        const id = persistence.sessionId!;
        // The store canonicalizes temp-directory aliases and Windows short names.
        const directory = await fs.realpath(path.join(fixture.sessions, store.workspaceDigest, id));
        const nodeFile = path.join(directory, "nodes", "000001.json");
        const headFile = path.join(directory, "head.json");
        const beforeHead = await fs.readFile(headFile, "utf8");
        const completed = revise(unfinished, "completed answer");
        rejectedTarget = boundary === "catalogue"
          ? path.join(directory, "catalog.json") : boundary === "node" ? nodeFile : headFile;

        await assert.rejects(persistence.checkpoint(completed), /injected replacement failure/);
        assert.equal(await fs.readFile(headFile, "utf8"), beforeHead);
        const node = decodeNode(JSON.parse(await fs.readFile(nodeFile, "utf8")));
        assert.equal(node.sequence, boundary === "head" ? 2 : 1);
        rejectedTarget = undefined;
        assert.deepEqual(await SessionPersistence.candidates(store), []);
        await persistence.close();

        // Use the actual resume entry point while this process is still alive.
        const candidates = await SessionPersistence.candidates(store);
        assert.deepEqual(candidates.map((entry) => entry.id), boundary === "head" ? [id] : []);
        const catalog = decodeSessionCatalog(JSON.parse(
          await fs.readFile(path.join(directory, "catalog.json"), "utf8"),
        ));
        const head = decodeHead(JSON.parse(await fs.readFile(headFile, "utf8")));
        assert.deepEqual(catalog.head, head);
        assert.equal(catalog.resumeNodeId, boundary === "head" ? 1 : 0);
        if (boundary === "head") {
          const resumed = await SessionPersistence.resume(store, id);
          try {
            assert.deepEqual(resumed.conversation.history, completed.history.map(stripRaw));
          } finally {
            await resumed.persistence.close();
          }
        }
      } finally {
        rejectedTarget = undefined;
        await persistence.close();
      }
    } finally {
      context.mock.restoreAll();
      syncBuiltinESMExports();
      await fs.rm(fixture.root, { recursive: true, force: true });
    }
  });
}
