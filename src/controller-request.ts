// One streamed provider request with a single safe context-overflow recovery.

import type { ControllerEvents, ControllerOptions } from "./controller.ts";
import {
  budgetRequestFromInputTokens,
} from "./context/budget.ts";
import type { ContextPolicy } from "./context/policy.ts";
import { isContextOverflow } from "./context/policy.ts";
import type { InputMeasurement, InputMeter } from "./context/measurement.ts";
import { fitRequestInput } from "./context/request.ts";
import type { Message, ToolSpec } from "./types.ts";

export type ControllerResponse = Readonly<{
  message: Message;
  context: Message[];
  inputTokens: number;
  measurement: InputMeasurement;
}>;

type PreparedContext = Readonly<{
  context: Message[];
  requestMessages: Message[];
  inputTokens: number;
  measurement: InputMeasurement;
}>;

export async function requestAssistant(
  history: readonly Message[],
  current: readonly Message[],
  specs: ToolSpec[],
  options: ControllerOptions,
  events: ControllerEvents,
  meter: InputMeter,
  signal?: AbortSignal,
): Promise<ControllerResponse> {
  let policy = await options.contextPolicy();
  const prepared = await prepareContext(
    history,
    current,
    specs,
    options,
    events,
    meter,
    policy,
    "budget",
    signal,
  );
  let context = prepared.context;
  let requestMessages = prepared.requestMessages;
  let inputTokens = prepared.inputTokens;
  let measurement = prepared.measurement;
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
      return { message, context, inputTokens: budget.inputTokens, measurement };
    } catch (error) {
      if (recovered) throw error;
      if (isContextOverflow(error as Error)) policy = await options.contextPolicy();
      const next = await prepareContext(
        history,
        context,
        specs,
        options,
        events,
        meter,
        policy,
        "overflow",
        signal,
        error as Error,
      );
      if (sameContext(next.context, context)) throw error;
      context = next.context;
      requestMessages = next.requestMessages;
      inputTokens = next.inputTokens;
      measurement = next.measurement;
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
  meter: InputMeter,
  policy: ContextPolicy,
  reason: "budget" | "overflow",
  signal?: AbortSignal,
  error?: Error,
): Promise<PreparedContext> {
  const input = {
    model: options.model,
    effort: options.effort,
    system: options.system,
    messages: [...context],
    tools: specs,
  };
  const initial = await meter.measure(input, signal);
  const projected = await events.onContext?.(history, context, {
    reason,
    policy,
    inputTokens: initial.inputTokens,
    ...(error === undefined ? {} : { error }),
  });
  const semantic = projected === undefined ? clone(context) : clone(projected);
  if (projected !== undefined) meter.reset();
  const semanticInput = { ...input, messages: semantic };
  const measurement = projected === undefined ? initial : await meter.measure(semanticInput, signal);
  const fitted = await fitRequestInput(semanticInput, meter, policy, measurement, signal);
  return {
    context: semantic,
    requestMessages: fitted.messages,
    inputTokens: fitted.measurement.inputTokens,
    measurement: fitted.measurement,
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
