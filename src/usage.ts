// Provider-neutral token accounting for one in-memory session.

import type { Usage } from "./types.ts";
import type { Message } from "./types.ts";

export type UsageTotals = Usage & {
  requests: number;
  /** Input tokens on the most recent provider request: the best context signal available. */
  lastInputTokens: number;
};

export function emptyUsage(): UsageTotals {
  return {
    requests: 0,
    lastInputTokens: 0,
    inputTokens: 0,
    outputTokens: 0,
    cachedInputTokens: 0,
    cacheWriteInputTokens: 0,
    reasoningTokens: 0,
  };
}

export function recordUsage(total: UsageTotals, next: Usage): void {
  total.requests += 1;
  addUsage(total, next);
}

/** Replace context pressure without inventing provider-reported usage. */
export function recordRequestInput(total: UsageTotals, inputTokens: number): void {
  total.lastInputTokens = inputTokens;
}

/** Account for an internal request without replacing the main context signal. */
export function recordAuxiliaryUsage(total: UsageTotals, next: Usage): void {
  total.requests += 1;
  addUsage(total, next);
}

function addUsage(total: UsageTotals, next: Usage): void {
  total.inputTokens += next.inputTokens;
  total.outputTokens += next.outputTokens;
  total.cachedInputTokens += next.cachedInputTokens;
  total.cacheWriteInputTokens += next.cacheWriteInputTokens;
  total.reasoningTokens += next.reasoningTokens;
}

export function usageFromHistory(messages: readonly Message[]): UsageTotals {
  const total = emptyUsage();
  for (const message of messages) {
    if (message.role === "assistant" && message.usage !== undefined) {
      recordUsage(total, message.usage);
      recordRequestInput(total, message.usage.inputTokens);
    }
  }
  return total;
}

export function formatTokens(value: number): string {
  if (value < 1_000) return String(value);
  if (value < 10_000) return `${(value / 1_000).toFixed(1)}k`;
  if (value < 1_000_000) return `${Math.round(value / 1_000)}k`;
  return `${(value / 1_000_000).toFixed(1)}m`;
}
