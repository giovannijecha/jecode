// One streamed provider request with a single safe context-overflow recovery.

import type { ControllerEvents, ControllerOptions } from "./controller.ts";
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
  const prepared = await events.onContext?.(history, current, "budget");
  let context = prepared === undefined ? [...current] : clone(prepared);
  let recovered = false;

  for (;;) {
    try {
      const message = await options.provider.send({
        model: options.model,
        system: options.system,
        messages: context,
        tools: specs,
        maxTokens: options.maxTokens,
        effort: options.effort,
        signal,
        onStream: (event) => events.onStream(event),
        onStatus: (status) => events.onStatus?.(status),
      });
      return { message, context };
    } catch (error) {
      if (recovered) throw error;
      const projected = await events.onContext?.(
        history,
        context,
        "overflow",
        error as Error,
      );
      if (projected === undefined) throw error;
      context = clone(projected);
      recovered = true;
    }
  }
}

function clone(messages: readonly Message[]): Message[] {
  return structuredClone([...messages]);
}
