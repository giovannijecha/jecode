// Small, head-tied projections for the resume catalogue.
//
// Conversation nodes remain authoritative and are fully decoded when a
// session is selected. This auxiliary record lets listing stay independent of
// every unselected tree's depth; a missing or suspect record falls back to the
// strict loader and can be rebuilt without changing the session schema.

import { Buffer } from "node:buffer";
import type { ConversationTree, TurnNode } from "../conversation.ts";
import { CONVERSATION_LIMITS } from "../conversation.ts";
import { leadingText } from "../text-boundary.ts";
import type { SessionHead, SessionMeta } from "./codec.ts";
import { decodeHead, encodeHead } from "./codec.ts";

export const SESSION_CATALOG_FILE = "catalog.json";
export const SESSION_CHECKPOINT_FILE = ".checkpoint";
export const SESSION_CATALOG_BYTES = 4 * 1_024;

// Older indexes can hide a node committed before a failed head write. Rebuild
// them through strict loading instead of trusting their apparently current head.
const CATALOG_SCHEMA = 2;
const PREVIEW_CODE_UNITS = 160;

export type StoredSessionCatalog = Readonly<{
  version: 2;
  id: string;
  workspaceDigest: string;
  createdAt: string;
  head: SessionHead;
  resumeNodeId: number;
  turns: number;
  preview: string;
}>;

export function sessionCatalog(
  meta: SessionMeta,
  head: SessionHead,
  conversation: ConversationTree,
): StoredSessionCatalog {
  assertActiveHead(head, conversation);
  const resumable = conversation.latestResumable();
  return own({
    version: CATALOG_SCHEMA,
    id: meta.id,
    workspaceDigest: meta.workspaceDigest,
    createdAt: meta.createdAt,
    head,
    resumeNodeId: resumable?.activeNodeId ?? 0,
    turns: resumable === undefined ? 0 : selectedTurnCount(resumable),
    preview: resumable === undefined ? "" : firstUserText(resumable),
  });
}

/** Advance the common linear checkpoint path without walking prior turns. */
export function advanceSessionCatalog(
  previous: StoredSessionCatalog,
  meta: SessionMeta,
  head: SessionHead,
  conversation: ConversationTree,
): StoredSessionCatalog {
  assertActiveHead(head, conversation);
  if (
    previous.id !== meta.id ||
    previous.workspaceDigest !== meta.workspaceDigest ||
    previous.createdAt !== meta.createdAt
  ) throw new Error("session catalogue does not match its metadata");

  const active = conversation.activeNode as TurnNode;
  const revisesHead = active.id === previous.head.nodeId &&
    active.parentId === previous.head.parentId &&
    active.revision === previous.head.revision + 1;
  const extendsHead = active.parentId === previous.head.nodeId &&
    active.id > previous.head.nodeId && active.revision === 1;
  if (!revisesHead && !extendsHead) return sessionCatalog(meta, head, conversation);

  // A resumable child of an unfinished head makes that intermediate node part
  // of the selected path. Rebuild in that uncommon case so the turn count is
  // exact instead of assuming one visible turn was appended.
  if (
    extendsHead && active.settlement !== "checkpointed" &&
    previous.resumeNodeId !== previous.head.nodeId
  ) return sessionCatalog(meta, head, conversation);

  if (active.settlement === "checkpointed" && previous.resumeNodeId === active.id) {
    return sessionCatalog(meta, head, conversation);
  }

  const resumable = active.settlement !== "checkpointed";
  const addsTurn = resumable && previous.resumeNodeId !== active.id;
  const turns = previous.turns + (addsTurn ? 1 : 0);
  const preview = turns === 0
    ? ""
    : previous.turns === 0
    ? firstUserTextInNode(active)
    : previous.preview;

  return own({
    version: CATALOG_SCHEMA,
    id: meta.id,
    workspaceDigest: meta.workspaceDigest,
    createdAt: meta.createdAt,
    head,
    resumeNodeId: resumable ? active.id : previous.resumeNodeId,
    turns,
    preview,
  });
}

export function encodeSessionCatalog(catalog: StoredSessionCatalog): string {
  const encoded = `${JSON.stringify(catalog, null, 2)}\n`;
  decodeSessionCatalog(JSON.parse(encoded) as unknown);
  if (Buffer.byteLength(encoded, "utf8") > SESSION_CATALOG_BYTES) {
    throw new Error("session catalogue data is invalid or unsupported");
  }
  return encoded;
}

export function decodeSessionCatalog(value: unknown): StoredSessionCatalog {
  if (!record(value) || !keys(
    value,
    "createdAt,head,id,preview,resumeNodeId,turns,version,workspaceDigest",
  )) throw invalid();
  const head = decodeHead(value["head"]);
  const resumeNodeId = value["resumeNodeId"];
  const turns = value["turns"];
  const preview = value["preview"];
  if (
    value["version"] !== CATALOG_SCHEMA ||
    !identifier(value["id"]) ||
    !digest(value["workspaceDigest"]) ||
    !timestamp(value["createdAt"]) ||
    !integer(resumeNodeId, 0) || resumeNodeId > head.nodeId ||
    !integer(turns, 0) || turns > CONVERSATION_LIMITS.nodes || turns > resumeNodeId ||
    typeof preview !== "string" || preview.length > PREVIEW_CODE_UNITS ||
    (resumeNodeId === 0) !== (turns === 0) ||
    (turns === 0 ? preview !== "" : preview === "")
  ) throw invalid();
  return Object.freeze({
    version: CATALOG_SCHEMA,
    id: value["id"],
    workspaceDigest: value["workspaceDigest"],
    createdAt: value["createdAt"],
    head,
    resumeNodeId,
    turns,
    preview,
  });
}

export function catalogMatches(
  catalog: StoredSessionCatalog,
  meta: SessionMeta,
  head: SessionHead,
): boolean {
  return catalog.id === meta.id &&
    catalog.workspaceDigest === meta.workspaceDigest &&
    catalog.createdAt === meta.createdAt &&
    sameSessionHead(catalog.head, head);
}

export function sameSessionHead(left: SessionHead, right: SessionHead): boolean {
  return left.version === right.version &&
    left.sequence === right.sequence &&
    left.nodeId === right.nodeId &&
    left.parentId === right.parentId &&
    left.revision === right.revision &&
    left.updatedAt === right.updatedAt;
}

function own(value: StoredSessionCatalog): StoredSessionCatalog {
  encodeHead(value.head);
  return decodeSessionCatalog(structuredClone(value));
}

function assertActiveHead(head: SessionHead, conversation: ConversationTree): void {
  const active = conversation.activeNode;
  if (
    active === undefined || active.id !== head.nodeId ||
    active.parentId !== head.parentId || active.revision !== head.revision
  ) throw new Error("session catalogue does not match its conversation head");
}

function selectedTurnCount(conversation: ConversationTree): number {
  let count = 0;
  let id = conversation.activeNodeId;
  while (id !== 0) {
    count++;
    id = conversation.node(id)?.parentId ?? 0;
  }
  return count;
}

function firstUserText(conversation: ConversationTree): string {
  const path: TurnNode[] = [];
  let id = conversation.activeNodeId;
  while (id !== 0) {
    const node = conversation.node(id);
    if (node === undefined) throw new Error("session catalogue path is incomplete");
    path.push(node);
    id = node.parentId;
  }
  for (let index = path.length - 1; index >= 0; index--) {
    const preview = userTextInNode(path[index] as TurnNode);
    if (preview !== undefined) return preview;
  }
  return "Untitled session";
}

function firstUserTextInNode(node: TurnNode): string {
  return userTextInNode(node) ?? "Untitled session";
}

function userTextInNode(node: TurnNode): string | undefined {
  for (const message of node.messages) {
    if (message.role !== "user") continue;
    const preview = textPreview(message.content.find((block) => block.kind === "text")?.text);
    if (preview !== undefined) return preview;
  }
  return undefined;
}

function textPreview(text: string | undefined): string | undefined {
  const normalized = text?.replace(/\s+/gu, " ").trim();
  return normalized === undefined || normalized === ""
    ? undefined
    : leadingText(normalized, PREVIEW_CODE_UNITS);
}

function keys(value: Record<string, unknown>, expected: string): boolean {
  return Object.keys(value).sort().join(",") === expected;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function integer(value: unknown, minimum: number): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= minimum;
}

function identifier(value: unknown): value is string {
  return typeof value === "string" && /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}$/.test(value);
}

function digest(value: unknown): value is string {
  return typeof value === "string" && /^[a-f0-9]{64}$/.test(value);
}

function timestamp(value: unknown): value is string {
  return typeof value === "string" && value.length >= 20 && value.length <= 64 &&
    Number.isFinite(Date.parse(value));
}

function invalid(): Error {
  return new Error("session catalogue data is invalid or unsupported");
}
