// Opt-in development observations. Never publish text, identities, or raw data.

import { channel } from "node:diagnostics_channel";
import type { CompactContextOptions, CompactionResult, CompactionOutcome, SummaryMeasurement } from "./compactor.ts";

export const CONTEXT_DIAGNOSTIC_CHANNEL = "jecode.context";
const observations = channel(CONTEXT_DIAGNOSTIC_CHANNEL);

export type CompactionDiagnostic = Readonly<{
  kind: "compaction";
  reason: "budget" | "overflow" | "manual";
  outcome: CompactionOutcome | "no-prefix" | "cancelled";
  beforeTokens: number;
  afterTokens?: number;
  elapsedMs: number;
  windowTokens?: number;
  triggerTokens?: number;
  requestLimitTokens?: number;
  summaryChars?: number;
  summaryProviderMs?: number;
  firstSummaryTextMs?: number;
}>;

export type RequestDiagnostic = Readonly<{
  kind: "request";
  source: "estimate" | "provider-prefix";
  tokenization?: "o200k-reference" | "heuristic";
  estimatedTokens: number;
  inputTokens: number;
  reportedInputTokens?: number;
  outcome?: "completed" | "failed" | "cancelled";
  windowTokens?: number;
  triggerTokens?: number;
  requestLimitTokens?: number;
  outputBudgetTokens?: number;
  preparationMs?: number;
  providerMs?: number;
  firstEventMs?: number;
  clippedResults?: number;
}>;

export type PreparationDiagnostic = Readonly<{
  kind: "preparation";
  reason: "budget" | "overflow";
  outcome: "failed" | "cancelled";
  elapsedMs: number;
  windowTokens: number;
  triggerTokens: number;
  requestLimitTokens: number;
}>;

export type ContextDiagnostic = CompactionDiagnostic | RequestDiagnostic | PreparationDiagnostic;

/** Whitelist at the channel boundary; subscribers must not serialize callers. */
export function safeDiagnostic(value: unknown): ContextDiagnostic | undefined {
  if (typeof value !== "object" || value === null) return undefined;
  const v = value as Record<string, unknown>;
  if (v["kind"] === "preparation" && (v["reason"] === "budget" || v["reason"] === "overflow") &&
    (v["outcome"] === "failed" || v["outcome"] === "cancelled") && count(v["elapsedMs"]) &&
    count(v["windowTokens"]) && count(v["triggerTokens"]) && count(v["requestLimitTokens"])) {
    return { kind: "preparation", reason: v["reason"], outcome: v["outcome"], elapsedMs: v["elapsedMs"],
      windowTokens: v["windowTokens"], triggerTokens: v["triggerTokens"], requestLimitTokens: v["requestLimitTokens"] };
  }
  if (v["kind"] === "request" &&
    (v["source"] === "estimate" || v["source"] === "provider-prefix") &&
    count(v["estimatedTokens"]) && count(v["inputTokens"]) &&
    (v["reportedInputTokens"] === undefined || count(v["reportedInputTokens"]))) {
    const extras: Partial<RequestDiagnostic> = {};
    for (const key of ["windowTokens", "triggerTokens", "requestLimitTokens", "outputBudgetTokens",
      "preparationMs", "providerMs", "firstEventMs", "clippedResults"] as const) {
      if (v[key] !== undefined) {
        if (!count(v[key])) return undefined;
        Object.assign(extras, { [key]: v[key] });
      }
    }
    const result = v["outcome"];
    if (result !== undefined && result !== "completed" && result !== "failed" && result !== "cancelled") return undefined;
    const tokenization = v["tokenization"];
    if (tokenization !== undefined && tokenization !== "o200k-reference" && tokenization !== "heuristic") return undefined;
    return { ...extras, ...(result === undefined ? {} : { outcome: result }),
      ...(tokenization === undefined ? {} : { tokenization }),
      kind: "request", source: v["source"], estimatedTokens: v["estimatedTokens"],
      inputTokens: v["inputTokens"],
      ...(v["reportedInputTokens"] === undefined ? {} : { reportedInputTokens: v["reportedInputTokens"] }) };
  }
  if (v["kind"] === "compaction" &&
    (v["reason"] === "budget" || v["reason"] === "overflow" || v["reason"] === "manual") &&
    outcome(v["outcome"]) && count(v["beforeTokens"]) && count(v["elapsedMs"]) &&
    (v["afterTokens"] === undefined || count(v["afterTokens"]))) {
    const limits: Record<string, number> = {};
    for (const key of ["windowTokens", "triggerTokens", "requestLimitTokens",
      "summaryChars", "summaryProviderMs", "firstSummaryTextMs"] as const) {
      if (v[key] !== undefined) {
        if (!count(v[key])) return undefined;
        limits[key] = v[key];
      }
    }
    return { ...limits, kind: "compaction", reason: v["reason"], outcome: v["outcome"],
      beforeTokens: v["beforeTokens"], elapsedMs: v["elapsedMs"],
      ...(v["afterTokens"] === undefined ? {} : { afterTokens: v["afterTokens"] }) };
  }
  return undefined;
}

export function publishDiagnostic(event: ContextDiagnostic): void {
  if (!observations.hasSubscribers) return;
  const safe = safeDiagnostic(event);
  if (safe !== undefined) observations.publish(safe);
}

export async function observedCompaction(
  options: CompactContextOptions,
  perform: (options: CompactContextOptions) => Promise<CompactionResult | undefined>,
): Promise<CompactionResult | undefined> {
  const started = performance.now();
  let state: CompactionDiagnostic["outcome"] = "no-prefix";
  let result: CompactionResult | undefined;
  let summary: SummaryMeasurement | undefined;
  try {
    result = await perform({ ...options,
      onBegin() { state = "failed"; options.onBegin?.(); },
      onOutcome(value) { state = value; options.onOutcome?.(value); },
      onSummary(value) { summary = value; options.onSummary?.(value); },
    });
    return result;
  } catch (error) {
    if (options.signal?.aborted) state = "cancelled";
    else if (state === "no-prefix") state = "failed";
    throw error;
  } finally {
    const event: CompactionDiagnostic = {
      kind: "compaction", reason: options.reason ?? (options.force ? "manual" : "budget"),
      outcome: state, beforeTokens: options.estimatedInputTokens ?? 0,
      windowTokens: options.policy.windowTokens, triggerTokens: options.policy.triggerTokens,
      requestLimitTokens: options.policy.requestLimitTokens,
      ...(result === undefined ? {} : { afterTokens: result.estimatedInputTokens }),
      elapsedMs: Math.round(performance.now() - started),
      ...summary,
    };
    publishDiagnostic(event);
    options.onDiagnostic?.(event);
  }
}

function count(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function outcome(value: unknown): value is CompactionDiagnostic["outcome"] {
  return value === "accepted" || value === "empty" || value === "oversized" ||
    value === "insufficient-savings" || value === "failed" || value === "timeout" ||
    value === "cancelled" || value === "no-prefix";
}
