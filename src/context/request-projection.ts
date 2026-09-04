// Ephemeral provider projection for aggregate tool evidence. Canonical context,
// saved sessions, transcript, and exports always retain complete tool output.

import type { Message, ToolResultBlock } from "../types.ts";
import { leadingText, trailingText } from "../text-boundary.ts";
import type { ContextPolicy } from "./policy.ts";

export const TOOL_RESULT_CLIP_MARKER = "[tool output clipped]";
const FAIR_CONTENT_CODE_UNITS = 256;
const APPEND_STABLE_RESULT_CODE_UNITS = 16_384;
const MAX_TOOL_RESULT_PROJECTION_CODE_UNITS = 256_000;

export type ToolResultProjection = Readonly<{
  messages: Message[];
  clippedResults: number;
  outputCodeUnits: number;
  saturated: boolean;
}>;

export function toolResultProjectionBudget(policy: ContextPolicy): number {
  return Math.min(MAX_TOOL_RESULT_PROJECTION_CODE_UNITS, policy.targetTokens);
}

/**
 * Keep historical result excerpts byte-for-byte stable as new results append.
 * Semantic compaction is allowed to establish a new prefix before the safety
 * fallback below redistributes an exhausted aggregate budget.
 */
export function projectToolResults(
  source: readonly Message[],
  requestedCodeUnits: number,
): ToolResultProjection {
  const results = toolResults(source, requestedCodeUnits);
  if (results.length === 0) {
    return { messages: [...source], clippedResults: 0, outputCodeUnits: 0, saturated: false };
  }
  const allocations: number[] = [];
  let remaining = requestedCodeUnits;
  let saturated = false;
  for (const result of results) {
    const desired = Math.min(result.output.length, APPEND_STABLE_RESULT_CODE_UNITS);
    if (desired <= remaining) {
      allocations.push(desired);
      remaining -= desired;
      continue;
    }
    saturated = true;
    const excerpt = remaining >= TOOL_RESULT_CLIP_MARKER.length ? remaining : 0;
    allocations.push(Math.min(result.output.length, excerpt));
    remaining -= excerpt;
  }
  return projectAllocations(source, allocations, saturated);
}

/** Prefer recent evidence when compaction cannot restore a stable prefix. */
export function projectToolResultsNewest(
  source: readonly Message[],
  requestedCodeUnits: number,
): ToolResultProjection {
  const results = toolResults(source, requestedCodeUnits);
  if (results.length === 0) {
    return { messages: [...source], clippedResults: 0, outputCodeUnits: 0, saturated: false };
  }

  const allocations = results.map(() => 0);
  let remaining = requestedCodeUnits;
  for (let index = results.length - 1; index >= 0 && remaining > 0; index--) {
    const result = results[index] as ToolResultBlock;
    const minimum = Math.min(result.output.length, TOOL_RESULT_CLIP_MARKER.length);
    if (minimum > remaining) continue;
    allocations[index] = minimum;
    remaining -= minimum;
  }

  remaining -= allocateFairExcerpt(results, allocations, remaining);
  for (let index = results.length - 1; index >= 0 && remaining > 0; index--) {
    const result = results[index] as ToolResultBlock;
    const extra = Math.min(remaining, result.output.length - (allocations[index] as number));
    allocations[index] = (allocations[index] as number) + extra;
    remaining -= extra;
  }
  return projectAllocations(source, allocations, true);
}

function toolResults(
  source: readonly Message[],
  requestedCodeUnits: number,
): ToolResultBlock[] {
  if (!Number.isSafeInteger(requestedCodeUnits) || requestedCodeUnits < 0) {
    throw new RangeError("tool-result projection budget is invalid");
  }
  return source.flatMap((message) =>
    message.content.filter((block): block is ToolResultBlock => block.kind === "tool_result")
  );
}

function projectAllocations(
  source: readonly Message[],
  allocations: readonly number[],
  saturated: boolean,
): ToolResultProjection {
  let resultIndex = 0;
  let clippedResults = 0;
  let outputCodeUnits = 0;
  const messages = source.map((message): Message => {
    let changed = false;
    const content = message.content.map((block) => {
      if (block.kind !== "tool_result") return block;
      const allocation = allocations[resultIndex++] as number;
      const output = clippedOutput(block.output, allocation);
      if (output !== block.output) {
        clippedResults++;
        changed = true;
      }
      outputCodeUnits += output.length;
      return { ...block, output };
    });
    return changed ? { ...message, content } : message;
  });
  return { messages, clippedResults, outputCodeUnits, saturated };
}

function allocateFairExcerpt(
  results: readonly ToolResultBlock[],
  allocations: number[],
  available: number,
): number {
  const needs = results.map((result, index) => {
    const allocated = allocations[index] as number;
    return allocated === 0 ? 0 : Math.min(
      FAIR_CONTENT_CODE_UNITS,
      result.output.length - allocated,
    );
  });
  let pool = Math.min(available, needs.reduce((total, value) => total + value, 0));
  const initial = pool;
  let open = needs.map((_need, index) => index).filter((index) => (needs[index] as number) > 0);
  while (pool > 0 && open.length > 0) {
    const share = Math.floor(pool / open.length);
    if (share === 0) {
      for (let cursor = open.length - 1; cursor >= 0 && pool > 0; cursor--) {
        const index = open[cursor] as number;
        allocations[index] = (allocations[index] as number) + 1;
        needs[index] = (needs[index] as number) - 1;
        pool--;
      }
      break;
    }
    for (const index of open) {
      const extra = Math.min(share, needs[index] as number, pool);
      allocations[index] = (allocations[index] as number) + extra;
      needs[index] = (needs[index] as number) - extra;
      pool -= extra;
    }
    open = open.filter((index) => (needs[index] as number) > 0);
  }
  return initial - pool;
}

function clippedOutput(output: string, maxCodeUnits: number): string {
  if (output.length <= maxCodeUnits) return output;
  if (maxCodeUnits <= TOOL_RESULT_CLIP_MARKER.length) {
    return leadingText(TOOL_RESULT_CLIP_MARKER, maxCodeUnits);
  }
  const content = maxCodeUnits - TOOL_RESULT_CLIP_MARKER.length;
  const head = leadingText(output, Math.ceil(content / 2));
  const tail = trailingText(output, Math.floor(content / 2));
  return `${head}${TOOL_RESULT_CLIP_MARKER}${tail}`;
}
