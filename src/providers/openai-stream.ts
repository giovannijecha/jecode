// Reassembling an OpenAI Responses reply from its event stream.
//
// Unlike Anthropic, a standard Responses stream ends with the whole finished
// response in `response.completed`. The ChatGPT Codex backend can instead send
// an empty final `output` after complete `response.output_item.done` events, so
// those streamed items remain the fallback when the final envelope is empty.

import type { StreamEvent } from "../types.ts";
import type { OpenAIResponse } from "./openai-wire.ts";

export async function assembleOpenAI(
  events: AsyncIterable<unknown>,
  onStream?: (event: StreamEvent) => void,
): Promise<OpenAIResponse> {
  const items: unknown[] = [];
  let refusal = false;

  for await (const raw of events) {
    const event = raw as {
      type?: string;
      delta?: string;
      item?: unknown;
      response?: unknown;
      error?: { message?: string };
      message?: string;
    };

    switch (event.type) {
      case "response.output_text.delta":
        if (typeof event.delta === "string") onStream?.({ kind: "text", text: event.delta });
        break;

      case "response.refusal.delta":
        if (typeof event.delta === "string") {
          onStream?.({ kind: "text", text: `${refusal ? "" : "[refused] "}${event.delta}` });
          refusal = true;
        }
        break;

      case "response.reasoning_summary_text.delta":
        if (typeof event.delta === "string") onStream?.({ kind: "thinking", text: event.delta });
        break;

      case "response.output_item.done":
        if (event.item !== undefined) items.push(event.item);
        break;

      case "response.done":
      case "response.completed":
      case "response.incomplete":
        return reconcileOutput(event.response as OpenAIResponse | undefined, items);

      case "response.failed": {
        const response = event.response as OpenAIResponse | undefined;
        throw new Error(`openai stream error: ${response?.error?.message ?? "unspecified"}`);
      }
      case "error":
        throw new Error(`openai stream error: ${event.error?.message ?? event.message ?? "unspecified"}`);

      default:
        break;
    }
  }

  throw new Error("openai stream ended before a terminal response event");
}

function reconcileOutput(completed: OpenAIResponse | undefined, items: unknown[]): OpenAIResponse {
  if (completed === undefined) return { output: items };
  const finalCount = Array.isArray(completed.output) ? completed.output.length : 0;
  return items.length > finalCount ? { ...completed, output: items } : completed;
}
