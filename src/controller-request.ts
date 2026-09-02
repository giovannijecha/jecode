// One streamed provider request with a single safe context-overflow recovery.

import type { ControllerEvents, ControllerOptions } from "./controller.ts";
import { budgetRequest, estimateRequestInputTokens } from "./context/budget.ts";
import type { ContextPolicy } from "./context/policy.ts";
import { isContextOverflow } from "./context/policy.ts";
import type { Message, ToolSpec } from "./types.ts";

export type ControllerResponse = Readonly<{
  message: Message;
  context: Message[];
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
  const prepared = await prepareContext(history, current, specs, options, events, policy, "budget");
  let context = prepared === undefined ? [...current] : clone(prepared);
  let recovered = false;

  for (;;) {
    const budget = budgetRequest({
      system: options.system,
      messages: context,
      tools: specs,
    }, options.maxTokens, policy);
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
      return { message, context };
    } catch (error) {
      if (recovered) throw error;
      if (isContextOverflow(error as Error)) policy = await options.contextPolicy();
      const projected = await prepareContext(
        history,
        context,
        specs,
        options,
        events,
        policy,
        "overflow",
        error as Error,
      );
      if (projected === undefined) throw error;
      context = clone(projected);
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
  error?: Error,
): Promise<readonly Message[] | undefined> {
  const inputTokens = estimateRequestInputTokens({
    system: options.system,
    messages: context,
    tools: specs,
  });
  return events.onContext?.(history, context, {
    reason,
    policy,
    inputTokens,
    ...(error === undefined ? {} : { error }),
  });
}

function clone(messages: readonly Message[]): Message[] {
  return structuredClone([...messages]);
}
