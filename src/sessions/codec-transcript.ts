// Settled transcript evidence across supported session schema versions.

import type { Detail, TranscriptBlock } from "../transcript-types.ts";
import type { SessionSchema } from "./codec.ts";
import {
  bounded,
  boundedText,
  integer,
  invalid,
  keys,
  nullableInteger,
  record,
  SESSION_FILE_LIMITS,
} from "./codec-values.ts";

export function blockRecord(block: TranscriptBlock): unknown[] {
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
    durationMs: block.durationMs ?? null,
  }];
}

export function blockFromRecord(value: unknown, version: SessionSchema): TranscriptBlock {
  if (!record(value) || typeof value["kind"] !== "string") throw invalid();
  if (
    (value["kind"] === "user" || value["kind"] === "answer" || value["kind"] === "reasoning") &&
    keys(value, "kind,text") && boundedText(value["text"])
  ) return { kind: value["kind"], text: value["text"] };
  const toolKeys = version >= 4
    ? "body,durationMs,kind,name,right,target,tone"
    : "body,kind,name,right,target,tone";
  if (
    value["kind"] === "tool" && keys(value, toolKeys) &&
    bounded(value["name"], 256) && boundedText(value["target"]) &&
    boundedText(value["right"], 1_024) &&
    (value["tone"] === "ok" || value["tone"] === "fail" || value["tone"] === "deny") &&
    (version < 4 || nullableInteger(value["durationMs"], 0)) &&
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
      ...(version < 4 || value["durationMs"] === null
        ? {}
        : { durationMs: value["durationMs"] as number }),
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
