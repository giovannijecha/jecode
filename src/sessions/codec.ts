// Strict codecs for session files. Disk is an untrusted boundary even when
// the directory is owner-only: every value is bounded and re-owned before it
// can become conversation or provider input.

import type { TurnNode } from "../conversation.ts";
import type { Detail, TranscriptBlock } from "../transcript-types.ts";
import type { Block, Message, Usage } from "../types.ts";

export const SESSION_SCHEMA = 1;
export const SESSION_FILE_LIMITS = Object.freeze({
  text: 1_048_576,
  jsonDepth: 24,
  jsonNodes: 32_768,
  blocks: 8_192,
  details: 8_192,
});

export type SessionMeta = Readonly<{
  version: 1;
  id: string;
  workspaceRoot: string;
  workspaceDigest: string;
  createdAt: string;
}>;

export type SessionHead = Readonly<{
  version: 1;
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
  return line(meta);
}

export function decodeMeta(value: unknown): SessionMeta {
  if (!record(value) || !keys(value, "createdAt,id,version,workspaceDigest,workspaceRoot")) {
    throw invalid();
  }
  if (
    value["version"] !== SESSION_SCHEMA ||
    !identifier(value["id"]) ||
    !bounded(value["workspaceRoot"], 32_768) ||
    !digest(value["workspaceDigest"]) ||
    !timestamp(value["createdAt"])
  ) throw invalid();
  return Object.freeze({
    version: 1,
    id: value["id"],
    workspaceRoot: value["workspaceRoot"],
    workspaceDigest: value["workspaceDigest"],
    createdAt: value["createdAt"],
  });
}

export function encodeHead(head: SessionHead): string {
  return line(head);
}

export function decodeHead(value: unknown): SessionHead {
  if (!record(value) || !keys(value, "nodeId,parentId,revision,sequence,updatedAt,version")) {
    throw invalid();
  }
  if (
    value["version"] !== SESSION_SCHEMA ||
    !integer(value["sequence"], 0) ||
    !integer(value["nodeId"], 1) ||
    !integer(value["parentId"], 0) || value["parentId"] >= value["nodeId"] ||
    !integer(value["revision"], 1) ||
    !timestamp(value["updatedAt"])
  ) throw invalid();
  return Object.freeze({
    version: 1,
    sequence: value["sequence"],
    nodeId: value["nodeId"],
    parentId: value["parentId"],
    revision: value["revision"],
    updatedAt: value["updatedAt"],
  });
}

export function encodeNode(node: TurnNode, sequence: number, updatedAt: string): string {
  return line({
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
    },
  });
}

export function decodeNode(value: unknown): StoredNode {
  if (!record(value) || !keys(value, "node,sequence,updatedAt,version")) throw invalid();
  if (
    value["version"] !== SESSION_SCHEMA || !integer(value["sequence"], 1) ||
    !timestamp(value["updatedAt"])
  ) throw invalid();
  const raw = value["node"];
  if (!record(raw) || !keys(raw, "blocks,createdAt,id,identity,messages,parentId,revision,settlement")) {
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
    (raw["settlement"] !== "checkpointed" && raw["settlement"] !== "completed") ||
    !record(identity) || !keys(identity, "effort,model,providerId") ||
    !bounded(identity["providerId"], 128) || !bounded(identity["model"], 512) ||
    !bounded(identity["effort"], 32) ||
    !Array.isArray(messages) || messages.length > SESSION_FILE_LIMITS.blocks ||
    !Array.isArray(blocks) || blocks.length > SESSION_FILE_LIMITS.blocks
  ) throw invalid();

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
    blocks: Object.freeze(blocks.map(blockFromRecord)),
  });
  return Object.freeze({ sequence: value["sequence"], updatedAt: value["updatedAt"], node });
}

function messageRecord(message: Message): unknown {
  return {
    role: message.role,
    content: message.content.map(contentRecord),
    usage: message.usage ?? null,
  };
}

function messageFromRecord(value: unknown): Message {
  if (!record(value) || !keys(value, "content,role,usage")) throw invalid();
  if (
    (value["role"] !== "user" && value["role"] !== "assistant") ||
    !Array.isArray(value["content"]) || value["content"].length > SESSION_FILE_LIMITS.blocks
  ) throw invalid();
  const usage = value["usage"] === null ? undefined : usageFromRecord(value["usage"]);
  return {
    role: value["role"],
    content: value["content"].map(contentFromRecord),
    ...(usage === undefined ? {} : { usage }),
  };
}

function contentRecord(block: Block): unknown {
  if (block.kind === "text") return { kind: block.kind, text: block.text };
  if (block.kind === "tool_call") {
    return { kind: block.kind, id: block.id, name: block.name, input: block.input };
  }
  return {
    kind: block.kind,
    id: block.id,
    output: block.output,
    isError: block.isError,
  };
}

function contentFromRecord(value: unknown): Block {
  if (!record(value) || typeof value["kind"] !== "string") throw invalid();
  if (value["kind"] === "text" && keys(value, "kind,text") && boundedText(value["text"])) {
    return { kind: "text", text: value["text"] };
  }
  if (
    value["kind"] === "tool_call" && keys(value, "id,input,kind,name") &&
    bounded(value["id"], 512) && bounded(value["name"], 256)
  ) {
    const input = jsonObject(value["input"]);
    return { kind: "tool_call", id: value["id"], name: value["name"], input };
  }
  if (
    value["kind"] === "tool_result" && keys(value, "id,isError,kind,output") &&
    bounded(value["id"], 512) && boundedText(value["output"]) &&
    typeof value["isError"] === "boolean"
  ) {
    return {
      kind: "tool_result",
      id: value["id"],
      output: value["output"],
      isError: value["isError"],
    };
  }
  throw invalid();
}

function usageFromRecord(value: unknown): Usage {
  if (!record(value) || !keys(
    value,
    "cacheWriteInputTokens,cachedInputTokens,inputTokens,outputTokens,reasoningTokens",
  )) throw invalid();
  const inputTokens = value["inputTokens"];
  const outputTokens = value["outputTokens"];
  const cachedInputTokens = value["cachedInputTokens"];
  const cacheWriteInputTokens = value["cacheWriteInputTokens"];
  const reasoningTokens = value["reasoningTokens"];
  if (
    !integer(inputTokens, 0) || !integer(outputTokens, 0) ||
    !integer(cachedInputTokens, 0) || !integer(cacheWriteInputTokens, 0) ||
    !integer(reasoningTokens, 0)
  ) throw invalid();
  return {
    inputTokens,
    outputTokens,
    cachedInputTokens,
    cacheWriteInputTokens,
    reasoningTokens,
  };
}

function blockRecord(block: TranscriptBlock): unknown[] {
  if (block.kind === "notice") return [];
  if (block.kind === "user" || block.kind === "answer" || block.kind === "reasoning") {
    return [{ kind: block.kind, text: block.text }];
  }
  if (block.tone === "pending") return [];
  return [{
    kind: block.kind,
    name: block.name,
    target: block.target,
    right: block.right,
    tone: block.tone,
    body: block.body?.map(detailRecord) ?? null,
  }];
}

function blockFromRecord(value: unknown): TranscriptBlock {
  if (!record(value) || typeof value["kind"] !== "string") throw invalid();
  if (
    (value["kind"] === "user" || value["kind"] === "answer" || value["kind"] === "reasoning") &&
    keys(value, "kind,text") && boundedText(value["text"])
  ) return { kind: value["kind"], text: value["text"] };
  if (
    value["kind"] === "tool" && keys(value, "body,kind,name,right,target,tone") &&
    bounded(value["name"], 256) && boundedText(value["target"]) &&
    boundedText(value["right"], 1_024) &&
    (value["tone"] === "ok" || value["tone"] === "fail" || value["tone"] === "deny") &&
    (value["body"] === null ||
      (Array.isArray(value["body"]) && value["body"].length <= SESSION_FILE_LIMITS.details))
  ) {
    const body = value["body"] === null
      ? undefined
      : value["body"].map(detailFromRecord);
    return {
      kind: "tool",
      name: value["name"],
      target: value["target"],
      right: value["right"],
      tone: value["tone"],
      ...(body === undefined ? {} : { body }),
    };
  }
  throw invalid();
}

function detailRecord(detail: Detail): unknown {
  if (detail.kind === "out" || detail.kind === "gap") return { kind: detail.kind, text: detail.text };
  return {
    kind: detail.kind,
    text: detail.text,
    oldLine: detail.oldLine ?? null,
    newLine: detail.newLine ?? null,
    emphasis: detail.emphasis ?? null,
  };
}

function detailFromRecord(value: unknown): Detail {
  if (!record(value) || typeof value["kind"] !== "string") throw invalid();
  if (
    (value["kind"] === "out" || value["kind"] === "gap") &&
    keys(value, "kind,text") && boundedText(value["text"])
  ) return { kind: value["kind"], text: value["text"] };
  if (
    (value["kind"] === "keep" || value["kind"] === "add" || value["kind"] === "del") &&
    keys(value, "emphasis,kind,newLine,oldLine,text") && boundedText(value["text"]) &&
    nullableInteger(value["oldLine"], 1) && nullableInteger(value["newLine"], 1)
  ) {
    const emphasis = emphasisFromRecord(value["emphasis"]);
    return {
      kind: value["kind"],
      text: value["text"],
      ...(value["oldLine"] === null ? {} : { oldLine: value["oldLine"] }),
      ...(value["newLine"] === null ? {} : { newLine: value["newLine"] }),
      ...(emphasis === undefined ? {} : { emphasis }),
    };
  }
  throw invalid();
}

function emphasisFromRecord(value: unknown): { start: number; length: number } | undefined {
  if (value === null) return undefined;
  if (!record(value) || !keys(value, "length,start")) throw invalid();
  if (!integer(value["start"], 0) || !integer(value["length"], 1)) throw invalid();
  return { start: value["start"], length: value["length"] };
}

function jsonObject(value: unknown): Record<string, unknown> {
  const budget = { nodes: 0 };
  const safe = jsonValue(value, budget, 0);
  if (!record(safe)) throw invalid();
  return safe;
}

function jsonValue(value: unknown, budget: { nodes: number }, depth: number): unknown {
  budget.nodes++;
  if (budget.nodes > SESSION_FILE_LIMITS.jsonNodes || depth > SESSION_FILE_LIMITS.jsonDepth) {
    throw invalid();
  }
  if (value === null || typeof value === "boolean") return value;
  if (typeof value === "string") {
    if (!boundedText(value)) throw invalid();
    return value;
  }
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw invalid();
    return value;
  }
  if (Array.isArray(value)) {
    if (value.length > SESSION_FILE_LIMITS.blocks) throw invalid();
    return value.map((item) => jsonValue(item, budget, depth + 1));
  }
  if (!record(value) || Object.keys(value).length > SESSION_FILE_LIMITS.blocks) throw invalid();
  const safe: Record<string, unknown> = {};
  for (const [name, child] of Object.entries(value)) {
    if (!boundedText(name, 1_024)) throw invalid();
    safe[name] = jsonValue(child, budget, depth + 1);
  }
  return safe;
}

function line(value: unknown): string {
  return `${JSON.stringify(value, null, 2)}\n`;
}

function keys(value: Record<string, unknown>, expected: string): boolean {
  return Object.keys(value).sort().join(",") === expected;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function bounded(value: unknown, limit: number = SESSION_FILE_LIMITS.text): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= limit;
}

function boundedText(value: unknown, limit: number = SESSION_FILE_LIMITS.text): value is string {
  return typeof value === "string" && value.length <= limit;
}

function integer(value: unknown, minimum: number): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= minimum;
}

function nullableInteger(value: unknown, minimum: number): value is number | null {
  return value === null || integer(value, minimum);
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
  return new Error("session data is invalid or unsupported");
}
