// Verified in-memory checkpoint identity shared by persistence operations.

import type { ConversationTree } from "../conversation.ts";
import { catalogMatches, encodeSessionCatalog } from "./catalog.ts";
import type { StoredSessionCatalog } from "./catalog.ts";
import { encodeHead, encodeMeta } from "./codec.ts";
import type { SessionHead, SessionMeta } from "./codec.ts";
import type { SessionLease } from "./lease.ts";
import { assertSessionId, workspaceKey } from "./bucket.ts";

export type SessionSnapshot = Readonly<{
  meta: SessionMeta;
  head: SessionHead;
  conversation: ConversationTree;
  catalog: StoredSessionCatalog;
}>;

export type ClaimedSessionSnapshot = SessionSnapshot & Readonly<{ lease: SessionLease }>;

export function assertSharedNodes(
  previous: ConversationTree,
  next: ConversationTree,
  replacedId: number | undefined,
): void {
  for (const node of previous.nodes) {
    if (node.id === replacedId) continue;
    const candidate = next.node(node.id);
    if (candidate !== node) {
      throw new Error("session checkpoint rewrites prior conversation history");
    }
  }
}

export function assertSnapshot(
  snapshot: SessionSnapshot,
  workspaceRoot: string,
  workspaceDigest: string,
): void {
  encodeMeta(snapshot.meta);
  encodeHead(snapshot.head);
  encodeSessionCatalog(snapshot.catalog);
  assertSessionId(snapshot.meta.id);
  if (
    snapshot.meta.workspaceDigest !== workspaceDigest ||
    workspaceKey(snapshot.meta.workspaceRoot) !== workspaceKey(workspaceRoot)
  ) throw new Error("session snapshot belongs to a different workspace");
  const active = snapshot.conversation.activeNode;
  if (
    active === undefined ||
    active.id !== snapshot.head.nodeId ||
    active.parentId !== snapshot.head.parentId ||
    active.revision !== snapshot.head.revision ||
    snapshot.head.sequence < snapshot.conversation.nodes.length
  ) throw new Error("session snapshot does not match its verified head");
  if (!catalogMatches(snapshot.catalog, snapshot.meta, snapshot.head)) {
    throw new Error("session snapshot does not match its verified catalogue");
  }
}
