// Provider-neutral context pressure and safe compaction boundaries.

import type { Message, ModelContextWindow } from "../types.ts";

export const DEFAULT_COMPACTION_PERCENT = 85;
export const MIN_COMPACTION_PERCENT = 50;
export const MAX_COMPACTION_PERCENT = 95;
export const CONTEXT_CAPACITY_PROBE_TOKENS = 16_000;
export const FALLBACK_CONTEXT_WINDOW_TOKENS = 200_000;

export type ContextPolicy = Readonly<{
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

export function planCompaction(
  context: readonly Message[],
  turn: readonly Message[],
  coveredMessages: number,
  lastInputTokens: number,
  force: boolean,
  policy: ContextPolicy,
): CompactionPlan | undefined {
  if (!validPolicy(policy) || coveredMessages < 0 || coveredMessages > turn.length) return undefined;
  const estimated = estimateTokens(context);
  if (
    !force &&
    (estimated < policy.targetTokens || Math.max(estimated, lastInputTokens) < policy.triggerTokens)
  ) return undefined;

  const currentSuffix = turn.length - coveredMessages;
  const contextPrefix = context.length - currentSuffix;
  if (contextPrefix < 0) return undefined;
  if (!sameMessages(context.slice(contextPrefix), turn.slice(coveredMessages))) return undefined;

  const targetTokens = force
    ? Math.min(policy.targetTokens, Math.max(512, Math.floor(estimated / 4)))
    : policy.targetTokens;
  const recentTokens = Math.min(policy.recentTokens, Math.max(256, Math.floor(targetTokens / 2)));
  let boundary = recentBoundary(turn, coveredMessages, recentTokens);
  let tail = turn.slice(boundary);
  if (turn.length > 1 && estimateTokens(tail) > targetTokens) {
    boundary = turn.length;
    tail = [];
  }

  const prefixEnd = contextPrefix + boundary - coveredMessages;
  if (prefixEnd <= 0 || prefixEnd > context.length) return undefined;
  const prefix = context.slice(0, prefixEnd);
  if (!force && estimateTokens(prefix) < policy.minimumPrefixTokens) return undefined;

  return {
    prefix: clone(prefix),
    tail: clone(tail),
    messageCount: boundary,
  };
}

export function shouldResolveContextPolicy(
  context: readonly Message[],
  lastInputTokens: number,
  force = false,
): boolean {
  return force || Math.max(estimateTokens(context), lastInputTokens) >= CONTEXT_CAPACITY_PROBE_TOKENS;
}

export function policyForContextWindow(
  context: ModelContextWindow | undefined,
  compactionPercent: number,
): ContextPolicy {
  const windowTokens = validWindow(context?.tokens)
    ? context.tokens
    : FALLBACK_CONTEXT_WINDOW_TOKENS;
  const percent = validPercent(compactionPercent)
    ? compactionPercent
    : DEFAULT_COMPACTION_PERCENT;
  const percentageLimit = Math.floor(windowTokens * percent / 100);
  const providerLimit = validWindow(context?.compactAtTokens)
    ? context.compactAtTokens
    : percentageLimit;
  const triggerTokens = Math.min(percentageLimit, providerLimit, windowTokens);
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
    triggerTokens,
    targetTokens,
    recentTokens,
    minimumPrefixTokens,
    summaryMaxTokens: Math.max(1_024, Math.min(4_096, Math.floor(windowTokens / 50))),
  });
}

export function estimateTokens(messages: readonly Message[]): number {
  const bytes = Buffer.byteLength(JSON.stringify(messages), "utf8");
  return Math.ceil(bytes / 3) + messages.length * 8;
}

export function isContextOverflow(error: Error): boolean {
  const candidate = error as Error & { status?: number; body?: string };
  if (candidate.status !== 400 && candidate.status !== 413) return false;
  const detail = `${candidate.message}\n${candidate.body ?? ""}`;
  return /context_length_exceeded|maximum context length|context window|prompt is too long|input (?:is )?too (?:long|large)|(?:input|prompt|context).{0,80}(?:exceed|maximum|max tokens)/i.test(detail);
}

function recentBoundary(
  turn: readonly Message[],
  coveredMessages: number,
  recentTokens: number,
): number {
  let boundary = minimumRecentBoundary(turn);
  for (let candidate = boundary - 1; candidate >= coveredMessages; candidate--) {
    if (!safeBoundary(turn, candidate)) continue;
    if (estimateTokens(turn.slice(candidate)) > recentTokens) break;
    boundary = candidate;
  }
  return Math.max(coveredMessages, boundary);
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
    policy.targetTokens < policy.triggerTokens && policy.recentTokens < policy.triggerTokens;
}

function validWindow(value: number | undefined): value is number {
  return value !== undefined && Number.isSafeInteger(value) && value >= 4_096 && value <= 10_000_000;
}

function validPercent(value: number): boolean {
  return Number.isSafeInteger(value) &&
    value >= MIN_COMPACTION_PERCENT && value <= MAX_COMPACTION_PERCENT;
}

function sameMessages(left: readonly Message[], right: readonly Message[]): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

function clone<T>(value: T): T {
  return structuredClone(value);
}
