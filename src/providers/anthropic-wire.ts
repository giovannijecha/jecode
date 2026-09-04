// Translation between the normalized vocabulary and the Anthropic wire shape.
// Pure functions, no I/O — which is what makes them testable without a key.

import type { Block, Message, ToolSpec, Usage } from "../types.ts";
import { toolInputFromValue } from "./tool-input.ts";
import { wireTokenCount } from "./wire-usage.ts";

export type WireBlock = Record<string, unknown>;

export type AnthropicResponse = {
  content?: unknown[];
  /** Parse failures recorded by the streaming assembler, outside the wire payload. */
  toolInputErrors?: Record<number, string>;
  stop_reason?: string;
  stop_details?: { category?: string | null; explanation?: string };
  usage?: {
    input_tokens?: unknown;
    output_tokens?: unknown;
    cache_read_input_tokens?: unknown;
    cache_creation_input_tokens?: unknown;
  };
};

export function toWireTool(tool: ToolSpec) {
  return { name: tool.name, description: tool.description, input_schema: tool.input };
}

export function toWireMessage(message: Message) {
  if (message.rawFrom === "anthropic" && message.raw !== undefined) {
    return { role: message.role, content: message.raw };
  }
  return { role: message.role, content: message.content.map(toWireBlock) };
}

function toWireBlock(block: Block) {
  if (block.kind === "text") return { type: "text", text: block.text };
  if (block.kind === "tool_call") {
    return { type: "tool_use", id: block.id, name: block.name, input: block.input };
  }
  return {
    type: "tool_result",
    tool_use_id: block.id,
    content: block.output,
    is_error: block.isError,
  };
}

/**
 * Why the model stopped, when that is something the user needs told. Returns
 * undefined for an ordinary ending.
 *
 * `stop_details` is populated for refusals and null for every other stop
 * reason, so it has to be read behind the check, not before it.
 */
export function stopNotice(data: AnthropicResponse): string | undefined {
  if (data.stop_reason === "refusal") {
    const category = data.stop_details?.category ?? "unspecified";
    const why = data.stop_details?.explanation ?? "no explanation given";
    return `[refused: ${category}] ${why}`;
  }
  if (data.stop_reason === "max_tokens") {
    return "[truncated: hit max_tokens — raise --max-tokens]";
  }
  return undefined;
}

export function fromWireResponse(data: AnthropicResponse): Message {
  const raw = Array.isArray(data.content) ? data.content : [];
  const content: Block[] = [];
  let suppressedToolCall = false;

  for (let index = 0; index < raw.length; index++) {
    const item = raw[index];
    const block = item as {
      type?: string;
      text?: string;
      id?: string;
      name?: string;
      input?: unknown;
    };

    if (block.type === "text" && typeof block.text === "string") {
      content.push({ kind: "text", text: block.text });
    } else if (block.type === "tool_use") {
      if (
        data.stop_reason === "tool_use" &&
        typeof block.id === "string" &&
        typeof block.name === "string"
      ) {
        const parsed = toolInputFromValue(block.input ?? {});
        const inputError = data.toolInputErrors?.[index] ?? parsed.inputError;
        content.push({
          kind: "tool_call",
          id: block.id,
          name: block.name,
          input: parsed.input,
          ...(inputError === undefined ? {} : { inputError }),
        });
      } else {
        suppressedToolCall = true;
      }
    }
    // thinking / redacted_thinking and anything future: carried in `raw` only.
  }

  const notice = stopNotice(data);
  if (notice !== undefined) content.push({ kind: "text", text: notice });

  const retainRaw = data.stop_reason !== "max_tokens" && !suppressedToolCall;
  return {
    role: "assistant",
    content,
    ...(retainRaw ? { raw, rawFrom: "anthropic" } : {}),
    usage: normalizeUsage(data),
  };
}

function normalizeUsage(data: AnthropicResponse): Usage | undefined {
  const usage = data.usage;
  if (usage === undefined) return undefined;
  const uncached = wireTokenCount(usage.input_tokens);
  const cached = wireTokenCount(usage.cache_read_input_tokens);
  const cacheWrite = wireTokenCount(usage.cache_creation_input_tokens);
  return {
    // Anthropic reports these as disjoint buckets. Jecode's provider-neutral
    // input count represents the complete request context, as OpenAI's does.
    inputTokens: uncached + cached + cacheWrite,
    outputTokens: wireTokenCount(usage.output_tokens),
    cachedInputTokens: cached,
    cacheWriteInputTokens: cacheWrite,
    reasoningTokens: 0,
  };
}
