// Conservative provider-neutral budgeting for one complete model request.

import type { Message, ToolSpec } from "../types.ts";
import {
  estimateSerializedTokens,
  estimateSerializedTokensResponsive,
} from "./estimate.ts";
import { MIN_REQUEST_OUTPUT_TOKENS, type ContextPolicy } from "./policy.ts";

const ENVELOPE_OVERHEAD_TOKENS = 64;
const MESSAGE_OVERHEAD_TOKENS = 8;
const TOOL_OVERHEAD_TOKENS = 16;

export type RequestEnvelope = Readonly<{
  system: string;
  messages: readonly Message[];
  tools: readonly ToolSpec[];
}>;

export type RequestBudget = Readonly<{
  inputTokens: number;
  maxOutputTokens: number;
  limitTokens: number;
}>;

/** Include system text and tool schemas instead of measuring conversation alone. */
export function estimateRequestInputTokens(envelope: RequestEnvelope): number {
  const contentTokens = estimateSerializedTokens({
    system: envelope.system,
    messages: envelope.messages,
    tools: envelope.tools,
  });
  return contentTokens +
    ENVELOPE_OVERHEAD_TOKENS +
    envelope.messages.length * MESSAGE_OVERHEAD_TOKENS +
    envelope.tools.length * TOOL_OVERHEAD_TOKENS;
}

/** Preserve the request estimate while yielding during large local inputs. */
export async function estimateRequestInputTokensResponsive(
  envelope: RequestEnvelope,
  signal?: AbortSignal,
): Promise<number> {
  const contentTokens = await estimateSerializedTokensResponsive({
    system: envelope.system,
    messages: envelope.messages,
    tools: envelope.tools,
  }, signal);
  return contentTokens +
    ENVELOPE_OVERHEAD_TOKENS +
    envelope.messages.length * MESSAGE_OVERHEAD_TOKENS +
    envelope.tools.length * TOOL_OVERHEAD_TOKENS;
}

/** Clamp the configured output ceiling so the complete request remains usable. */
export function budgetRequest(
  envelope: RequestEnvelope,
  configuredMaxOutputTokens: number,
  policy: ContextPolicy,
): RequestBudget {
  requirePositiveInteger(configuredMaxOutputTokens, "max output tokens");
  const inputTokens = estimateRequestInputTokens(envelope);
  return finishBudget(inputTokens, configuredMaxOutputTokens, policy);
}

/** Reuse an exact estimate already computed for the same request envelope. */
export function budgetRequestFromInputTokens(
  inputTokens: number,
  configuredMaxOutputTokens: number,
  policy: ContextPolicy,
): RequestBudget {
  requirePositiveInteger(inputTokens, "request input tokens");
  requirePositiveInteger(configuredMaxOutputTokens, "max output tokens");
  return finishBudget(inputTokens, configuredMaxOutputTokens, policy);
}

function finishBudget(
  inputTokens: number,
  configuredMaxOutputTokens: number,
  policy: ContextPolicy,
): RequestBudget {
  const available = policy.requestLimitTokens - inputTokens;
  const minimum = Math.min(configuredMaxOutputTokens, MIN_REQUEST_OUTPUT_TOKENS);
  if (available < minimum) {
    throw new Error(
      `request input needs approximately ${inputTokens} tokens, leaving ` +
      `${Math.max(0, available)} of the ${policy.requestLimitTokens}-token safe request budget; ` +
      `at least ${minimum} output tokens are required`,
    );
  }

  return Object.freeze({
    inputTokens,
    maxOutputTokens: Math.min(configuredMaxOutputTokens, available),
    limitTokens: policy.requestLimitTokens,
  });
}

function requirePositiveInteger(value: number, label: string): void {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new Error(`${label} must be a positive safe integer`);
  }
}
