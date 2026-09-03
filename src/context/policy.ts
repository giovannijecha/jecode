// Provider-neutral context pressure and safe compaction boundaries.

import type { Message, ModelContextWindow } from "../types.ts";
import {
  estimateSerializedTokens,
  estimateSerializedTokensResponsive,
} from "./estimate.ts";

export const DEFAULT_COMPACTION_PERCENT = 85;
export const MIN_COMPACTION_PERCENT = 50;
export const MAX_COMPACTION_PERCENT = 95;
export const FALLBACK_CONTEXT_WINDOW_TOKENS = 200_000;
export const REQUEST_ESTIMATE_HEADROOM_PERCENT = 5;
export const MIN_REQUEST_OUTPUT_TOKENS = 256;

export type ContextPolicy = Readonly<{
  windowTokens: number;
  requestLimitTokens: number;
  triggerTokens: number;
  targetTokens: number;
  recentTokens: number;
  minimumPrefixTokens: number;
  summaryMaxTokens: number;
}>;

export type CompactionPlan = Readonly<{
  prefix: Message[];
  tail: Message[];
  messageCount: number;
}>;

export async function planCompaction(
  context: readonly Message[],
  turn: readonly Message[],
  coveredMessages: number,
  lastInputTokens: number,
  force: boolean,
  policy: ContextPolicy,
  estimatedInputTokens?: number,
  signal?: AbortSignal,
): Promise<CompactionPlan | undefined> {
  if (!validPolicy(policy) || coveredMessages < 0 || coveredMessages > turn.length) return undefined;
  if (
    estimatedInputTokens !== undefined &&
    (!Number.isSafeInteger(estimatedInputTokens) || estimatedInputTokens <= 0)
  ) return undefined;
  const estimated = estimatedInputTokens ?? await estimateTokensResponsive(context, signal);
  if (
    !force &&
    (estimated < policy.targetTokens || Math.max(estimated, lastInputTokens) < policy.triggerTokens)
  ) return undefined;

  const currentSuffix = turn.length - coveredMessages;
  const contextPrefix = context.length - currentSuffix;
  if (contextPrefix < 0) return undefined;
  if (!await sameMessages(
    context.slice(contextPrefix),
    turn.slice(coveredMessages),
    signal,
  )) return undefined;

  const targetTokens = force
    ? Math.min(policy.targetTokens, Math.max(512, Math.floor(estimated / 4)))
    : policy.targetTokens;
  const recentTokens = Math.min(policy.recentTokens, Math.max(256, Math.floor(targetTokens / 2)));
  const recent = await recentBoundary(turn, coveredMessages, recentTokens, signal);
  let boundary = recent.boundary;
  let tail = turn.slice(boundary);
  if (turn.length > 1 && recent.tokens > targetTokens) {
    boundary = turn.length;
    tail = [];
  }

  const prefixEnd = contextPrefix + boundary - coveredMessages;
  if (prefixEnd <= 0 || prefixEnd > context.length) return undefined;
  const prefix = context.slice(0, prefixEnd);
  if (!force && await estimateTokensResponsive(prefix, signal) < policy.minimumPrefixTokens) {
    return undefined;
  }

  return {
    prefix: clone(prefix),
    tail: clone(tail),
    messageCount: boundary,
  };
}

export function policyForContextWindow(
  context: ModelContextWindow | undefined,
  compactionPercent: number,
): ContextPolicy {
  const windowTokens = context === undefined
    ? FALLBACK_CONTEXT_WINDOW_TOKENS
    : requireWindow(context.tokens);
  const percent = validPercent(compactionPercent)
    ? compactionPercent
    : DEFAULT_COMPACTION_PERCENT;
  const percentageLimit = Math.floor(windowTokens * percent / 100);
  const providerLimit = context?.compactAtTokens === undefined
    ? windowTokens
    : requireWindow(context.compactAtTokens);
  const requestLimitTokens = Math.floor(
    Math.min(providerLimit, windowTokens) * (100 - REQUEST_ESTIMATE_HEADROOM_PERCENT) / 100,
  );
  const triggerTokens = Math.min(
    percentageLimit,
    requestLimitTokens - MIN_REQUEST_OUTPUT_TOKENS,
  );
  const targetTokens = Math.max(512, Math.min(
    Math.floor(windowTokens / 4),
    Math.floor(triggerTokens / 2),
  ));
  const recentTokens = Math.max(256, Math.min(
    Math.floor(windowTokens / 8),
    Math.floor(targetTokens / 2),
  ));
  const minimumPrefixTokens = Math.max(256, Math.min(
    Math.floor(windowTokens / 20),
    Math.floor(targetTokens / 2),
  ));

  return Object.freeze({
    windowTokens,
    requestLimitTokens,
    triggerTokens,
    targetTokens,
    recentTokens,
    minimumPrefixTokens,
    summaryMaxTokens: Math.max(1_024, Math.min(4_096, Math.floor(windowTokens / 50))),
  });
}

export function estimateTokens(messages: readonly Message[]): number {
  return estimateSerializedTokens(messages) + messages.length * 8;
}

export async function estimateTokensResponsive(
  messages: readonly Message[],
  signal?: AbortSignal,
): Promise<number> {
  return await estimateSerializedTokensResponsive(messages, signal) + messages.length * 8;
}

export function isContextOverflow(error: Error): boolean {
  const candidate = error as Error & { status?: number; body?: string };
  if (candidate.status !== 400 && candidate.status !== 413) return false;
  const detail = `${candidate.message}\n${candidate.body ?? ""}`;
  return /context_length_exceeded|maximum context length|context window|prompt is too long|input (?:is )?too (?:long|large)|(?:input|prompt|context).{0,80}(?:exceed|maximum|max tokens)/i.test(detail);
}

async function recentBoundary(
  turn: readonly Message[],
  coveredMessages: number,
  recentTokens: number,
  signal: AbortSignal | undefined,
): Promise<Readonly<{ boundary: number; tokens: number }>> {
  const minimum = Math.max(coveredMessages, minimumRecentBoundary(turn));
  const candidates: number[] = [];
  for (let candidate = coveredMessages; candidate <= minimum; candidate++) {
    if (candidate === minimum || safeBoundary(turn, candidate)) candidates.push(candidate);
  }
  const cache = new Map<number, Promise<number>>();
  const estimateAt = (candidate: number): Promise<number> => {
    let estimate = cache.get(candidate);
    if (estimate === undefined) {
      estimate = estimateTokensResponsive(turn.slice(candidate), signal);
      cache.set(candidate, estimate);
    }
    return estimate;
  };

  const last = candidates.length - 1;
  const minimumTokens = await estimateAt(candidates[last] as number);
  if (minimumTokens > recentTokens) {
    return { boundary: candidates[last] as number, tokens: minimumTokens };
  }

  // Adding older messages raises the byte and literal floors. A lower-bound
  // search therefore replaces the former serialization of every suffix.
  let low = 0;
  let high = last;
  let selected = last;
  while (low <= high) {
    const middle = Math.floor((low + high) / 2);
    const tokens = await estimateAt(candidates[middle] as number);
    if (tokens <= recentTokens) {
      selected = middle;
      high = middle - 1;
    } else {
      low = middle + 1;
    }
  }
  const boundary = candidates[selected] as number;
  return { boundary, tokens: await estimateAt(boundary) };
}

function minimumRecentBoundary(turn: readonly Message[]): number {
  const last = turn.at(-1);
  if (last === undefined) return 0;
  if (last.role === "user" && last.content.some((block) => block.kind === "tool_result")) {
    return Math.max(0, turn.length - 2);
  }
  return Math.max(0, turn.length - 1);
}

function safeBoundary(turn: readonly Message[], index: number): boolean {
  const before = turn[index - 1];
  const after = turn[index];
  if (before?.role !== "assistant" || after?.role !== "user") return true;
  const calls = new Set(before.content
    .filter((block) => block.kind === "tool_call")
    .map((block) => block.id));
  return !after.content.some((block) => block.kind === "tool_result" && calls.has(block.id));
}

function validPolicy(policy: ContextPolicy): boolean {
  return Object.values(policy).every((value) => Number.isSafeInteger(value) && value > 0) &&
    policy.requestLimitTokens <= policy.windowTokens &&
    policy.triggerTokens <= policy.requestLimitTokens &&
    policy.targetTokens < policy.triggerTokens && policy.recentTokens < policy.triggerTokens;
}

function validWindow(value: number | undefined): value is number {
  // Adapters validate raw capacities at 4K before applying their own usable
  // headroom. The resulting safe capacity can therefore be slightly smaller.
  return value !== undefined && Number.isSafeInteger(value) && value >= 1_024 && value <= 10_000_000;
}

function requireWindow(value: number): number {
  if (validWindow(value)) return value;
  throw new Error(`provider reported an invalid usable context window: ${value}`);
}

function validPercent(value: number): boolean {
  return Number.isSafeInteger(value) &&
    value >= MIN_COMPACTION_PERCENT && value <= MAX_COMPACTION_PERCENT;
}

async function sameMessages(
  left: readonly Message[],
  right: readonly Message[],
  signal: AbortSignal | undefined,
): Promise<boolean> {
  if (left.length !== right.length) return false;
  for (let index = 0; index < left.length; index++) {
    throwIfAborted(signal);
    if (JSON.stringify(left[index]) !== JSON.stringify(right[index])) return false;
    await new Promise<void>((resolve) => setImmediate(resolve));
  }
  return true;
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted !== true) return;
  throw signal.reason instanceof Error ? signal.reason : new Error("aborted");
}

function clone<T>(value: T): T {
  return structuredClone(value);
}
