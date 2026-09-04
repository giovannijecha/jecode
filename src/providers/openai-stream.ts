// Reassembling an OpenAI Responses reply from its event stream.
//
// Unlike Anthropic, a standard Responses stream ends with the whole finished
// response in `response.completed`. The ChatGPT Codex backend can instead send
// an empty final `output` after complete `response.output_item.done` events, so
// those streamed items remain the fallback when the final envelope is empty.

import type { StreamEvent } from "../types.ts";
import { providerWireError } from "./failure.ts";
import type { OpenAIResponse } from "./openai-wire.ts";

export async function assembleOpenAI(
  events: AsyncIterable<unknown>,
  onStream?: (event: StreamEvent) => void,
  onStatus?: (status: string) => void,
): Promise<OpenAIResponse> {
  const items: unknown[] = [];
  const announcedTools = { identities: new Set<string>(), anonymous: false };
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
          onStream?.({ kind: "text", text: event.delta });
        }
        break;

      case "response.refusal.delta":
        if (typeof event.delta === "string") {
          status("Responding");
          onStream?.({ kind: "text", text: `${refusal ? "" : "[refused] "}${event.delta}` });
          refusal = true;
        }
        break;

      case "response.reasoning_summary_text.delta":
        if (typeof event.delta === "string") {
          status("Thinking");
          onStream?.({ kind: "thinking", text: event.delta });
        }
        break;

      case "response.reasoning_summary_part.added":
        status("Thinking");
        break;

      case "response.reasoning_summary_text.done":
      case "response.reasoning_summary_part.done":
        status("Working");
        break;

      case "response.output_item.added":
        if (isFunctionCall(event.item)) {
          announceTool(event, event.item, announcedTools, onStream, status);
        } else if (itemType(event.item) === "reasoning") {
          status("Thinking");
        } else if (itemType(event.item) === "message") {
          status("Responding");
        }
        break;

      case "response.function_call_arguments.delta":
      case "response.function_call_arguments.done":
        announceTool(event, undefined, announcedTools, onStream, status);
        break;

      case "response.output_item.done":
        if (event.item !== undefined) {
          if (isFunctionCall(event.item)) {
            announceTool(event, event.item, announcedTools, onStream, status);
          } else if (itemType(event.item) === "reasoning") {
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
  const finalCount = Array.isArray(completed.output) ? completed.output.length : 0;
  return items.length > finalCount ? { ...completed, output: items } : completed;
}

function withDefaultStatus(response: OpenAIResponse, status: string): OpenAIResponse {
  return response.status === undefined ? { ...response, status } : response;
}
