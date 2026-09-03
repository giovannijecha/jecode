// Provider-neutral validation for tool arguments crossing a wire boundary.

import type { ToolCallBlock } from "../types.ts";

export type ParsedToolInput = Pick<ToolCallBlock, "input" | "inputError">;

export function toolInputFromJson(text: string | undefined): ParsedToolInput {
  if (text === undefined || text.trim() === "") return { input: {} };
  try {
    return toolInputFromValue(JSON.parse(text) as unknown);
  } catch {
    return { input: {}, inputError: "tool arguments were not valid JSON" };
  }
}

export function toolInputFromValue(value: unknown): ParsedToolInput {
  if (typeof value === "object" && value !== null && !Array.isArray(value)) {
    return { input: value as Record<string, unknown> };
  }
  return { input: {}, inputError: "tool arguments must be a JSON object" };
}
