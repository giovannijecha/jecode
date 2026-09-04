// One streamed provider request with a single safe context-overflow recovery.

import type { ControllerEvents, ControllerOptions } from "./controller.ts";
import {
  budgetRequestFromInputTokens,
  estimateRequestInputTokensResponsive,
} from "./context/budget.ts";
import type { ContextPolicy } from "./context/policy.ts";
import { isContextOverflow } from "./context/policy.ts";
import {
  projectToolResults,
  projectToolResultsNewest,
  toolResultProjectionBudget,
} from "./context/request-projection.ts";
import type { Message, ToolSpec } from "./types.ts";

export type ControllerResponse = Readonly<{
  message: Message;
  context: Message[];
  inputTokens: number;
}>;

type PreparedContext = Readonly<{
  context: Message[];
  requestMessages: Message[];
  inputTokens: number;
}>;

export async function requestAssistant(
  history: readonly Message[],
  current: readonly Message[],
  specs: ToolSpec[],
  options: ControllerOptions,
  events: ControllerEvents,
  signal?: AbortSignal,
): Promise<ControllerResponse> {
  let policy = await options.contextPolicy();
  const prepared = await prepareContext(
    history,
    current,
    specs,
    options,
    events,
    policy,
    "budget",
    signal,
  );
  let context = prepared.context;
  let requestMessages = prepared.requestMessages;
  let inputTokens = prepared.inputTokens;
  let recovered = false;

  for (;;) {
    const budget = budgetRequestFromInputTokens(inputTokens, options.maxTokens, policy);
    try {
      const message = await options.provider.send({
        model: options.model,
        system: options.system,
        messages: requestMessages,
        tools: specs,
        maxTokens: budget.maxOutputTokens,
        effort: options.effort,
        ...(options.requestIdentity === undefined
          ? {}
          : { identity: { ...options.requestIdentity, purpose: "turn" as const } }),
        signal,
        onStream: (event) => events.onStream(event),
        onStatus: (status) => events.onStatus?.(status),
      });
      return { message, context, inputTokens: budget.inputTokens };
    } catch (error) {
      if (recovered) throw error;
      if (isContextOverflow(error as Error)) policy = await options.contextPolicy();
      const next = await prepareContext(
        history,
        context,
        specs,
        options,
        events,
        policy,
        "overflow",
        signal,
        error as Error,
      );
      if (sameContext(next.context, context)) throw error;
      context = next.context;
      requestMessages = next.requestMessages;
      inputTokens = next.inputTokens;
      recovered = true;
    }
  }
}

async function prepareContext(
  history: readonly Message[],
  context: readonly Message[],
  specs: ToolSpec[],
  options: ControllerOptions,
  events: ControllerEvents,
  policy: ContextPolicy,
  reason: "budget" | "overflow",
  signal?: AbortSignal,
  error?: Error,
): Promise<PreparedContext> {
  const projectionBudget = toolResultProjectionBudget(policy);
  const initialProjection = projectToolResults(
    context,
    projectionBudget,
  );
  const initialRequest = initialProjection.messages;
  const inputTokens = await estimateRequestInputTokensResponsive(
    {
      system: options.system,
      messages: initialRequest,
      tools: specs,
    },
    signal,
  );
  const projected = await events.onContext?.(history, context, {
    reason,
    policy,
    inputTokens,
    projectionSaturated: initialProjection.saturated,
    ...(error === undefined ? {} : { error }),
  });
  const semantic = projected === undefined ? clone(context) : clone(projected);
  const stableProjection = projected === undefined
    ? initialProjection
    : projectToolResults(semantic, projectionBudget);
  const requestMessages = stableProjection.saturated
    ? projectToolResultsNewest(semantic, projectionBudget).messages
    : stableProjection.messages;
  const canReuseEstimate = projected === undefined && !stableProjection.saturated;
  return {
    context: semantic,
    requestMessages,
    inputTokens: canReuseEstimate
      ? inputTokens
      : await estimateRequestInputTokensResponsive({
          system: options.system,
          messages: requestMessages,
          tools: specs,
        }, signal),
  };
}

function clone(messages: readonly Message[]): Message[] {
  return structuredClone([...messages]);
}

function sameContext(left: readonly Message[], right: readonly Message[]): boolean {
  return left.length === right.length && left.every((message, index) => (
    JSON.stringify(message) === JSON.stringify(right[index])
  ));
}
