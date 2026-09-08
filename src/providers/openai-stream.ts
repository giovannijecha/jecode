// Reassembling an OpenAI Responses reply from its event stream.
//
// Unlike Anthropic, a standard Responses stream ends with the whole finished
// response in `response.completed`. The ChatGPT Codex backend can instead send
// an empty final `output` after complete `response.output_item.done` events, so
// those streamed items remain the fallback when the final envelope is empty.

import type { ResponseStage, StreamEvent } from "../types.ts";
import { providerWireError } from "./failure.ts";
import type { OpenAIResponse } from "./openai-wire.ts";
import { OpenAISummary } from "./openai-summary.ts";

const OUTPUT_EVENTS = new Set([
  "response.output_item.added", "response.output_item.done",
  "response.content_part.added", "response.content_part.done",
  "response.output_text.delta", "response.output_text.done",
  "response.refusal.delta", "response.refusal.done",
  "response.reasoning_summary_part.added", "response.reasoning_summary_part.done",
  "response.reasoning_summary_text.delta", "response.reasoning_summary_text.done",
  "response.function_call_arguments.delta", "response.function_call_arguments.done",
]);

/** Diagnostic phase only: output can be opaque reasoning, not visible text or a saved result. */
export function openAIResponseStage(value: unknown, previous: ResponseStage): ResponseStage {
  if (previous === "terminal" || typeof value !== "object" || value === null || !("type" in value)) return previous;
  if (openAITerminalEvent(value) || value.type === "error") return "terminal";
  if (typeof value.type === "string" && OUTPUT_EVENTS.has(value.type)) return "output";
  if (previous === "awaiting" && (value.type === "response.created" || value.type === "response.in_progress")) return "accepted";
  return previous;
}

export function openAITerminalEvent(value: unknown): boolean {
  if (typeof value !== "object" || value === null || !("type" in value)) return false;
  return value.type === "response.completed" || value.type === "response.done" ||
    value.type === "response.incomplete" || value.type === "response.failed";
}

export async function assembleOpenAI(
  events: AsyncIterable<unknown>,
  onStream?: (event: StreamEvent) => void,
  onStatus?: (status: string) => void,
): Promise<OpenAIResponse> {
  const items: unknown[] = [];
  const announcedTools = { identities: new Set<string>(), anonymous: false };
  const summary = new OpenAISummary();
  const display = (event: StreamEvent): void => {
    if (event.kind !== "thinking") summary.reset();
    onStream?.(event);
  };
  let refusal = false;
  let activity: string | undefined;
  const status = (next: string): void => {
    if (activity === next) return;
    activity = next;
    onStatus?.(next);
  };

  for await (const raw of events) {
    const event = raw as {
      type?: string;
      delta?: string;
      item?: unknown;
      item_id?: unknown;
      output_index?: unknown;
      summary_index?: unknown;
      name?: unknown;
      response?: unknown;
      error?: { code?: string; message?: string; type?: string };
      message?: string;
    };

    switch (event.type) {
      case "response.created":
      case "response.in_progress":
        if (activity === undefined) status("Working");
        break;

      case "response.output_text.delta":
        if (typeof event.delta === "string") {
          status("Responding");
          display({ kind: "text", text: event.delta });
        }
        break;

      case "response.refusal.delta":
        if (typeof event.delta === "string") {
          status("Responding");
          display({ kind: "text", text: `${refusal ? "" : "[refused] "}${event.delta}` });
          refusal = true;
        }
        break;

      case "response.reasoning_summary_text.delta": {
        const text = summary.delta(event);
        if (text !== undefined) {
          status("Thinking");
          display({ kind: "thinking", text });
        }
        break;
      }

      case "response.reasoning_summary_part.added":
        summary.end();
        status("Thinking");
        break;

      case "response.reasoning_summary_text.done":
      case "response.reasoning_summary_part.done":
        summary.end();
        status("Working");
        break;

      case "response.output_item.added":
        if (isFunctionCall(event.item)) {
          announceTool(event, event.item, announcedTools, display, status);
        } else if (itemType(event.item) === "reasoning") {
          status("Thinking");
        } else if (itemType(event.item) === "message") {
          status("Responding");
        }
        break;

      case "response.function_call_arguments.delta":
      case "response.function_call_arguments.done":
        announceTool(event, undefined, announcedTools, display, status);
        break;

      case "response.output_item.done":
        if (event.item !== undefined) {
          if (isFunctionCall(event.item)) {
            announceTool(event, event.item, announcedTools, display, status);
          } else if (itemType(event.item) === "reasoning") {
            summary.end();
            status("Working");
          }
          items.push(event.item);
        }
        break;

      case "response.done":
      case "response.completed":
        return withDefaultStatus(
          reconcileOutput(event.response as OpenAIResponse | undefined, items),
          "completed",
        );

      case "response.incomplete":
        return {
          ...reconcileOutput(event.response as OpenAIResponse | undefined, items),
          status: "incomplete",
        };

      case "response.failed": {
        const response = event.response as OpenAIResponse | undefined;
        throw providerWireError("openai stream error", response?.error?.message, {
          code: response?.error?.code,
          type: response?.error?.type,
        });
      }
      case "error":
        throw providerWireError(
          "openai stream error",
          event.error?.message ?? event.message,
          { code: event.error?.code, type: event.error?.type },
        );

      default:
        break;
    }
  }

  throw new Error("openai stream ended before a terminal response event");
}

/** State-only keepalives prove transport liveness, not forward model progress. */
export function openAIStreamProgress(raw: unknown): boolean {
  if (typeof raw !== "object" || raw === null) return false;
  const type = (raw as Record<string, unknown>)["type"];
  if (typeof type !== "string") return false;
  if (type === "response.created") return true;
  if (
    type === "response.done" ||
    type === "response.completed" ||
    type === "response.incomplete" ||
    type === "response.failed" ||
    type === "error"
  ) return true;
  return /\.(?:added|delta|done)$/u.test(type);
}

type OpenAIStreamEvent = {
  item_id?: unknown;
  output_index?: unknown;
  name?: unknown;
};

type FunctionCallItem = {
  type: "function_call";
  id?: unknown;
  call_id?: unknown;
  name?: unknown;
};

type ToolAnnouncements = {
  identities: Set<string>;
  anonymous: boolean;
};

function isFunctionCall(item: unknown): item is FunctionCallItem {
  return typeof item === "object" && item !== null &&
    (item as Record<string, unknown>)["type"] === "function_call";
}

function itemType(item: unknown): unknown {
  return typeof item === "object" && item !== null
    ? (item as Record<string, unknown>)["type"]
    : undefined;
}

function announceTool(
  event: OpenAIStreamEvent,
  item: FunctionCallItem | undefined,
  announced: ToolAnnouncements,
  onStream?: (event: StreamEvent) => void,
  onStatus?: (status: string) => void,
): void {
  const identities = toolIdentities(event, item);
  if (identities.length === 0) {
    if (announced.anonymous) return;
    announced.anonymous = true;
  } else {
    const duplicate = identities.some((identity) => announced.identities.has(identity));
    for (const identity of identities) announced.identities.add(identity);
    if (duplicate) return;
  }

  const rawName = item?.name ?? event.name;
  const name = typeof rawName === "string" && rawName !== "" ? rawName : undefined;
  onStatus?.(`Preparing ${name ?? "tool"}`);
  onStream?.({ kind: "tool", ...(name === undefined ? {} : { name }) });
}

function toolIdentities(
  event: OpenAIStreamEvent,
  item: FunctionCallItem | undefined,
): string[] {
  const identities: string[] = [];
  const itemId = event.item_id ?? item?.id;
  if (typeof itemId === "string" && itemId !== "") identities.push(`item:${itemId}`);
  if (typeof event.output_index === "number") identities.push(`output:${event.output_index}`);
  if (typeof item?.call_id === "string" && item.call_id !== "") {
    identities.push(`call:${item.call_id}`);
  }
  return identities;
}

function reconcileOutput(completed: OpenAIResponse | undefined, items: unknown[]): OpenAIResponse {
  if (completed === undefined) return { output: items };
  if (completed.error != null || (completed.status !== undefined &&
      completed.status !== "completed" && completed.status !== "incomplete")) {
    throw providerWireError("openai stream error", completed.error?.message ?? "Response did not complete", {
      code: completed.error?.code, type: completed.error?.type,
    });
  }
  const finalCount = Array.isArray(completed.output) ? completed.output.length : 0;
  return finalCount === 0 && items.length > 0 ? { ...completed, output: items } : completed;
}

function withDefaultStatus(response: OpenAIResponse, status: string): OpenAIResponse {
  return response.status === undefined ? { ...response, status } : response;
}
