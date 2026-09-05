// Strict codecs for session files. Disk is an untrusted boundary even when
// the directory is owner-only: every value is bounded and re-owned before it
// can become conversation or provider input.

import type { TurnFailure, TurnNode, TurnSettlement } from "../conversation.ts";
import type { ContextAnchor } from "../context/projection.ts";
import { CONTEXT_LIMITS } from "../context/projection.ts";
import { messageFromRecord, messageRecord } from "./codec-messages.ts";
import { blockFromRecord, blockRecord } from "./codec-transcript.ts";
import {
  bounded,
  boundedLine,
  digest,
  identifier,
  integer,
  invalid,
  keys,
  record,
  SESSION_FILE_LIMITS,
  timestamp,
} from "./codec-values.ts";

export { SESSION_FILE_LIMITS } from "./codec-values.ts";

export const SESSION_SCHEMA = 4;
export type SessionSchema = 1 | 2 | 3 | 4;
export type SessionMeta = Readonly<{
  version: SessionSchema;
  id: string;
  workspaceRoot: string;
  workspaceDigest: string;
  createdAt: string;
}>;

export type SessionHead = Readonly<{
  version: SessionSchema;
  sequence: number;
  nodeId: number;
  parentId: number;
  revision: number;
  updatedAt: string;
}>;

export type StoredNode = Readonly<{
  sequence: number;
  updatedAt: string;
  node: TurnNode;
}>;

export function encodeMeta(meta: SessionMeta): string {
  decodeMeta(meta);
  return boundedLine(meta, SESSION_FILE_LIMITS.metadataBytes);
}

export function decodeMeta(value: unknown): SessionMeta {
  if (!record(value) || !keys(value, "createdAt,id,version,workspaceDigest,workspaceRoot")) {
    throw invalid();
  }
  if (
    !schema(value["version"]) ||
    !identifier(value["id"]) ||
    !bounded(value["workspaceRoot"], 32_768) ||
    !digest(value["workspaceDigest"]) ||
    !timestamp(value["createdAt"])
  ) throw invalid();
  return Object.freeze({
    version: value["version"],
    id: value["id"],
    workspaceRoot: value["workspaceRoot"],
    workspaceDigest: value["workspaceDigest"],
    createdAt: value["createdAt"],
  });
}

export function encodeHead(head: SessionHead): string {
  decodeHead(head);
  return boundedLine(head, SESSION_FILE_LIMITS.metadataBytes);
}

export function decodeHead(value: unknown): SessionHead {
  if (!record(value) || !keys(value, "nodeId,parentId,revision,sequence,updatedAt,version")) {
    throw invalid();
  }
  if (
    !schema(value["version"]) ||
    !integer(value["sequence"], 0) ||
    !integer(value["nodeId"], 1) ||
    !integer(value["parentId"], 0) || value["parentId"] >= value["nodeId"] ||
    !integer(value["revision"], 1) ||
    !timestamp(value["updatedAt"])
  ) throw invalid();
  return Object.freeze({
    version: value["version"],
    sequence: value["sequence"],
    nodeId: value["nodeId"],
    parentId: value["parentId"],
    revision: value["revision"],
    updatedAt: value["updatedAt"],
  });
}

export function encodeNode(node: TurnNode, sequence: number, updatedAt: string): string {
  const envelope = nodeEnvelope(node, sequence, updatedAt);
  decodeNode(envelope);
  return boundedLine(envelope, SESSION_FILE_LIMITS.nodeBytes);
}

/** Enforce the exact current-disk boundary before a turn enters the tree. */
export function assertPersistableNode(node: TurnNode): void {
  encodeNode(node, 1, node.createdAt);
}

function nodeEnvelope(node: TurnNode, sequence: number, updatedAt: string): unknown {
  return {
    version: SESSION_SCHEMA,
    sequence,
    updatedAt,
    node: {
      id: node.id,
      parentId: node.parentId,
      revision: node.revision,
      createdAt: node.createdAt,
      settlement: node.settlement,
      identity: node.identity,
      messages: node.messages.map(messageRecord),
      blocks: node.blocks.flatMap(blockRecord),
      context: node.context ?? null,
      failure: node.failure ?? null,
    },
  };
}

export function decodeNode(value: unknown): StoredNode {
  if (!record(value) || !keys(value, "node,sequence,updatedAt,version")) throw invalid();
  const version = value["version"];
  if (
    !schema(version) || !integer(value["sequence"], 1) ||
    !timestamp(value["updatedAt"])
  ) throw invalid();
  const raw = value["node"];
  const nodeKeys = version === 1
    ? "blocks,createdAt,id,identity,messages,parentId,revision,settlement"
    : version === 2
    ? "blocks,context,createdAt,id,identity,messages,parentId,revision,settlement"
    : "blocks,context,createdAt,failure,id,identity,messages,parentId,revision,settlement";
  if (!record(raw) || !keys(raw, nodeKeys)) {
    throw invalid();
  }
  const identity = raw["identity"];
  const messages = raw["messages"];
  const blocks = raw["blocks"];
  if (
    !integer(raw["id"], 1) ||
    !integer(raw["parentId"], 0) ||
    !integer(raw["revision"], 1) ||
    !timestamp(raw["createdAt"]) ||
    !settlement(raw["settlement"], version) ||
    !record(identity) || !keys(identity, "effort,model,providerId") ||
    !bounded(identity["providerId"], 128) || !bounded(identity["model"], 512) ||
    !bounded(identity["effort"], 32) ||
    !Array.isArray(messages) || messages.length > SESSION_FILE_LIMITS.blocks ||
    !Array.isArray(blocks) || blocks.length > SESSION_FILE_LIMITS.blocks
  ) throw invalid();

  const context = version === 1
    ? undefined
    : contextFromRecord(raw["context"], raw["id"], messages.length);
  const failure = version < 3
    ? undefined
    : failureFromRecord(raw["failure"], raw["settlement"]);

  const node: TurnNode = Object.freeze({
    id: raw["id"],
    parentId: raw["parentId"],
    revision: raw["revision"],
    createdAt: raw["createdAt"],
    settlement: raw["settlement"],
    identity: Object.freeze({
      providerId: identity["providerId"],
      model: identity["model"],
      effort: identity["effort"],
    }),
    messages: Object.freeze(messages.map(messageFromRecord)),
    blocks: Object.freeze(blocks.map((block) => blockFromRecord(block, version))),
    ...(context === undefined ? {} : { context: Object.freeze(context) }),
    ...(failure === undefined ? {} : { failure: Object.freeze(failure) }),
  });
  return Object.freeze({ sequence: value["sequence"], updatedAt: value["updatedAt"], node });
}

function failureFromRecord(
  value: unknown,
  settlement: TurnSettlement,
): TurnFailure | undefined {
  const failed = settlement === "failed" || settlement === "interrupted";
  if (value === null) {
    if (failed) throw invalid();
    return undefined;
  }
  if (!failed || !record(value) || !keys(value, "text,tone")) throw invalid();
  if (
    !bounded(value["text"]) ||
    (settlement === "failed" && value["tone"] !== "error") ||
    (settlement === "interrupted" && value["tone"] !== "warn")
  ) throw invalid();
  return { text: value["text"], tone: value["tone"] as "warn" | "error" };
}

function contextFromRecord(
  value: unknown,
  ownerId: number,
  ownerMessages: number,
): ContextAnchor | undefined {
  if (value === null) return undefined;
  if (!record(value) || !keys(value, "createdAt,messageCount,summary,throughNodeId")) {
    throw invalid();
  }
  if (
    !integer(value["throughNodeId"], 1) || value["throughNodeId"] > ownerId ||
    !integer(value["messageCount"], 0) ||
    (value["throughNodeId"] === ownerId && value["messageCount"] > ownerMessages) ||
    !timestamp(value["createdAt"]) ||
    !bounded(value["summary"], CONTEXT_LIMITS.summaryCodeUnits)
  ) throw invalid();
  return {
    throughNodeId: value["throughNodeId"],
    messageCount: value["messageCount"],
    createdAt: value["createdAt"],
    summary: value["summary"],
  };
}

function schema(value: unknown): value is SessionSchema {
  return value === 1 || value === 2 || value === 3 || value === SESSION_SCHEMA;
}

function settlement(value: unknown, version: SessionSchema): value is TurnSettlement {
  return value === "checkpointed" || value === "completed" ||
    (version >= 3 && (value === "failed" || value === "interrupted"));
}
