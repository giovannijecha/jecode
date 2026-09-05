// Strict full-session loading and one adjacent checkpoint recovery.

import * as path from "node:path";
import { atomicWrite } from "../atomic.ts";
import { ConversationTree } from "../conversation.ts";
import { assertDirectoryAnchor, captureDirectDirectory } from "../directory-anchor.ts";
import { assertSessionId } from "./bucket.ts";
import type { SessionBucket } from "./bucket.ts";
import { sameSessionHead, sessionCatalog } from "./catalog.ts";
import { decodeHead, decodeMeta, encodeHead, SESSION_FILE_LIMITS, SESSION_SCHEMA } from "./codec.ts";
import type { StoredNode } from "./codec.ts";
import { FILE_MODE, readJson, readNodes } from "./files.ts";
import { sessionLeaseOwns } from "./lease.ts";
import type { SessionLease } from "./lease.ts";
import type { SessionSnapshot } from "./snapshot.ts";

export async function loadSession(
  bucket: SessionBucket,
  id: string,
  scope: object,
  recoveryLease?: SessionLease,
): Promise<SessionSnapshot> {
  assertSessionId(id);
  await bucket.assert();
  const directory = bucket.directory(id);
  const directoryAnchor = await captureDirectDirectory(directory, "session directory");
  const nodesAnchor = await captureDirectDirectory(
    path.join(directory, "nodes"),
    "session node directory",
  );
  const validateSession = async (): Promise<void> => {
    await Promise.all([
      bucket.assert(),
      assertDirectoryAnchor(directoryAnchor),
      assertDirectoryAnchor(nodesAnchor),
    ]);
  };
  const meta = decodeMeta(await readJson(
    path.join(directory, "meta.json"),
    SESSION_FILE_LIMITS.metadataBytes,
    undefined,
    validateSession,
  ));
  bucket.assertWorkspace(meta, id);
  const persistedHead = decodeHead(await readJson(
    path.join(directory, "head.json"),
    SESSION_FILE_LIMITS.metadataBytes,
    undefined,
    validateSession,
  ));
  let head = persistedHead;
  const stored = await readNodes(nodesAnchor, validateSession);
  const ahead = stored.filter((entry) => entry.sequence > head.sequence);
  if (ahead.some((entry) => entry.sequence !== head.sequence + 1) || ahead.length > 1) {
    throw new Error("session has an ambiguous incomplete checkpoint");
  }

  const byId = new Map(stored.map((entry) => [entry.node.id, entry]));
  const headed = byId.get(head.nodeId);
  if (headed === undefined) throw new Error("session head is missing its conversation node");

  if (ahead.length === 0) {
    if (
      headed.sequence !== head.sequence || headed.node.revision !== head.revision ||
      headed.node.parentId !== head.parentId
    ) {
      throw new Error("session head does not match its conversation node");
    }
  } else {
    const candidate = ahead[0] as StoredNode;
    const replacesHead = candidate.node.id === head.nodeId &&
      candidate.node.parentId === head.parentId &&
      candidate.node.revision === head.revision + 1;
    const candidateParent = byId.get(candidate.node.parentId);
    const extendsTree = candidate.node.id === stored.length &&
      candidate.node.revision === 1 && candidateParent !== undefined &&
      candidateParent.sequence <= head.sequence;
    if (!replacesHead && !extendsTree) {
      throw new Error("session checkpoint cannot be recovered safely");
    }
    if (
      recoveryLease === undefined ||
      !sessionLeaseOwns(recoveryLease, id, scope)
    ) {
      throw new Error("session recovery requires exclusive ownership");
    }
    await recoveryLease.assertOwned();
    head = Object.freeze({
      version: SESSION_SCHEMA,
      sequence: candidate.sequence,
      nodeId: candidate.node.id,
      parentId: candidate.node.parentId,
      revision: candidate.node.revision,
      updatedAt: candidate.updatedAt,
    });
  }

  // Recovery must pass all turn, tree, and catalogue invariants before it
  // changes durable state; the file codec alone cannot validate the tree.
  const nodes = stored.map((entry) => entry.node);
  const conversation = ConversationTree.restore(nodes, head.nodeId);
  const catalog = sessionCatalog(meta, head, conversation);
  if (head !== persistedHead && recoveryLease !== undefined) {
    await atomicWrite(path.join(directory, "head.json"), encodeHead(head), {
      mode: FILE_MODE,
      validate: async (phase) => {
        await validateSession();
        if (phase !== "before-rename") return;
        await recoveryLease.assertOwned();
        const current = decodeHead(await readJson(
          path.join(directory, "head.json"),
          SESSION_FILE_LIMITS.metadataBytes,
          undefined,
          validateSession,
        ));
        if (!sameSessionHead(current, persistedHead)) {
          throw new Error("session head changed while recovering its checkpoint");
        }
      },
    });
  }

  return Object.freeze({ meta, head, conversation, catalog });
}
