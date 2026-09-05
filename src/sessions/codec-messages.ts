// Normalized provider-message records; opaque provider data never persists.

import type { Block, Message, Usage } from "../types.ts";
import {
  bounded,
  boundedText,
  integer,
  invalid,
  keys,
  record,
  SESSION_FILE_LIMITS,
} from "./codec-values.ts";

export function messageRecord(message: Message): unknown {
  return {
    role: message.role,
    content: message.content.map(contentRecord),
    usage: message.usage ?? null,
  };
}

export function messageFromRecord(value: unknown): Message {
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
    Object.defineProperty(safe, name, {
      value: jsonValue(child, budget, depth + 1),
      enumerable: true,
      configurable: true,
      writable: true,
    });
  }
  return safe;
}
