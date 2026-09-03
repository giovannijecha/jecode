// One streamed provider request with a single safe context-overflow recovery.

import type { ControllerEvents, ControllerOptions } from "./controller.ts";
import {
  budgetRequestFromInputTokens,
  estimateRequestInputTokensResponsive,
} from "./context/budget.ts";
import type { ContextPolicy } from "./context/policy.ts";
import { isContextOverflow } from "./context/policy.ts";
import type { Message, ToolSpec } from "./types.ts";

export type ControllerResponse = Readonly<{
  message: Message;
  context: Message[];
  inputTokens: number;
}>;

type PreparedContext = Readonly<{
  projected: readonly Message[] | undefined;
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
  let context = prepared.projected === undefined ? [...current] : clone(prepared.projected);
  let inputTokens = prepared.inputTokens;
  let recovered = false;

  for (;;) {
    const budget = budgetRequestFromInputTokens(inputTokens, options.maxTokens, policy);
    try {
      const message = await options.provider.send({
        model: options.model,
        system: options.system,
        messages: context,
        tools: specs,
        maxTokens: budget.maxOutputTokens,
        effort: options.effort,
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
        inputTokens,
      );
      if (next.projected === undefined) throw error;
      context = clone(next.projected);
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
  knownInputTokens?: number,
): Promise<PreparedContext> {
  const inputTokens = knownInputTokens ?? await estimateRequestInputTokensResponsive(
    {
      system: options.system,
      messages: context,
      tools: specs,
    },
    signal,
  );
  const projected = await events.onContext?.(history, context, {
    reason,
    policy,
    inputTokens,
    ...(error === undefined ? {} : { error }),
  });
  return {
    projected,
    inputTokens: projected === undefined
      ? inputTokens
      : await estimateRequestInputTokensResponsive({
        system: options.system,
        messages: projected,
        tools: specs,
      }, signal),
  };
}

function clone(messages: readonly Message[]): Message[] {
  return structuredClone([...messages]);
}
