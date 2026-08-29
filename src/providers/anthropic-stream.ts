// Reassembling an Anthropic response from its event stream.
//
// The stream never delivers a finished message — `message_stop` carries
// nothing — so the blocks have to be rebuilt here, index by index, into
// exactly the array a non-streamed response would have returned. That matters
// beyond display: thinking blocks carry a signature and must be echoed back
// byte-for-byte on the next request.

import type { StreamEvent } from "../types.ts";
import type { AnthropicResponse, WireBlock } from "./anthropic-wire.ts";
import { addBounded, MAX_TOOL_ARGUMENT_CHARS } from "./stream-limits.ts";

export async function assembleAnthropic(
  events: AsyncIterable<unknown>,
  onStream?: (event: StreamEvent) => void,
): Promise<AnthropicResponse> {
  const blocks = new Map<number, WireBlock>();
  const partialJson = new Map<number, string>();
  const sizes = { toolArguments: 0 };
  let stopReason: string | undefined;
  let stopDetails: AnthropicResponse["stop_details"];
  let usage: AnthropicResponse["usage"];

  for await (const raw of events) {
    const event = raw as WireEvent;

    switch (event.type) {
      case "message_start":
        usage = mergeUsage(usage, event.message?.usage);
        break;

      case "content_block_start": {
        if (typeof event.index === "number" && event.content_block !== undefined) {
          blocks.set(event.index, { ...event.content_block });
        }
        break;
      }

      case "content_block_delta": {
        if (typeof event.index !== "number") break;
        applyDelta(blocks, partialJson, sizes, event.index, event.delta, onStream);
        break;
      }

      case "content_block_stop": {
        if (typeof event.index !== "number") break;
        const pending = partialJson.get(event.index);
        const block = blocks.get(event.index);
        if (pending !== undefined && block !== undefined) {
          block.input = parseJsonObject(pending);
          partialJson.delete(event.index);
        }
        break;
      }

      case "message_delta": {
        stopReason = event.delta?.stop_reason ?? stopReason;
        stopDetails = event.delta?.stop_details ?? stopDetails;
        usage = mergeUsage(usage, event.usage);
        break;
      }

      case "error": {
        throw new Error(
          `anthropic stream error: ${event.error?.message ?? "unspecified"}`,
        );
      }

      default:
        break; // message_start, ping, message_stop: nothing to accumulate
    }
  }

  const content = [...blocks.entries()]
    .sort(([a], [b]) => a - b)
    .map(([, block]) => block);

  return { content, stop_reason: stopReason, stop_details: stopDetails, usage };
}

type WireEvent = {
  type?: string;
  index?: number;
  content_block?: WireBlock;
  delta?: {
    type?: string;
    text?: string;
    thinking?: string;
    signature?: string;
    partial_json?: string;
    stop_reason?: string;
    stop_details?: AnthropicResponse["stop_details"];
  };
  error?: { message?: string };
  message?: { usage?: AnthropicResponse["usage"] };
  usage?: AnthropicResponse["usage"];
};

function mergeUsage(
  before: AnthropicResponse["usage"],
  after: AnthropicResponse["usage"],
): AnthropicResponse["usage"] {
  return after === undefined ? before : { ...before, ...after };
}

function applyDelta(
  blocks: Map<number, WireBlock>,
  partialJson: Map<number, string>,
  sizes: { toolArguments: number },
  index: number,
  delta: WireEvent["delta"],
  onStream?: (event: StreamEvent) => void,
): void {
  const block = blocks.get(index);
  if (delta === undefined || block === undefined) return;

  switch (delta.type) {
    case "text_delta":
      if (typeof delta.text !== "string") return;
      block.text = `${(block.text as string | undefined) ?? ""}${delta.text}`;
      onStream?.({ kind: "text", text: delta.text });
      return;

    case "thinking_delta":
      if (typeof delta.thinking !== "string") return;
      block.thinking = `${(block.thinking as string | undefined) ?? ""}${delta.thinking}`;
      onStream?.({ kind: "thinking", text: delta.thinking });
      return;

    case "signature_delta":
      if (typeof delta.signature === "string") block.signature = delta.signature;
      return;

    case "input_json_delta":
      if (typeof delta.partial_json !== "string") return;
      sizes.toolArguments = addBounded(
        sizes.toolArguments,
        delta.partial_json.length,
        MAX_TOOL_ARGUMENT_CHARS,
        "streamed tool arguments",
      );
      partialJson.set(index, `${partialJson.get(index) ?? ""}${delta.partial_json}`);
      return;

    default:
      return;
  }
}

// Tool arguments arrive as a stream of JSON fragments. An empty accumulation
// is a call with no arguments, not a malformed one.
function parseJsonObject(text: string): Record<string, unknown> {
  if (text.trim() === "") return {};
  try {
    const parsed = JSON.parse(text) as unknown;
    return typeof parsed === "object" && parsed !== null
      ? (parsed as Record<string, unknown>)
      : {};
  } catch {
    return {};
  }
}
