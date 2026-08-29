// Translation between the normalized vocabulary and the Anthropic wire shape.
// Pure functions, no I/O — which is what makes them testable without a key.

import type { Block, Message, ToolSpec, Usage } from "../types.ts";

export type WireBlock = Record<string, unknown>;

export type AnthropicResponse = {
  content?: unknown[];
  stop_reason?: string;
  stop_details?: { category?: string | null; explanation?: string };
  usage?: {
    input_tokens?: number;
    output_tokens?: number;
    cache_read_input_tokens?: number;
    cache_creation_input_tokens?: number;
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

  for (const item of raw) {
    const block = item as {
      type?: string;
      text?: string;
      id?: string;
      name?: string;
      input?: unknown;
    };

    if (block.type === "text" && typeof block.text === "string") {
      content.push({ kind: "text", text: block.text });
    } else if (
      block.type === "tool_use" &&
      typeof block.id === "string" &&
      typeof block.name === "string"
    ) {
      content.push({
        kind: "tool_call",
        id: block.id,
        name: block.name,
        input: (block.input ?? {}) as Record<string, unknown>,
      });
    }
    // thinking / redacted_thinking and anything future: carried in `raw` only.
  }

  const notice = stopNotice(data);
  if (notice !== undefined) content.push({ kind: "text", text: notice });

  return { role: "assistant", content, raw, rawFrom: "anthropic", usage: normalizeUsage(data) };
}

function normalizeUsage(data: AnthropicResponse): Usage | undefined {
  const usage = data.usage;
  if (usage === undefined) return undefined;
  return {
    inputTokens: usage.input_tokens ?? 0,
    outputTokens: usage.output_tokens ?? 0,
    cachedInputTokens: usage.cache_read_input_tokens ?? 0,
    cacheWriteInputTokens: usage.cache_creation_input_tokens ?? 0,
    reasoningTokens: 0,
  };
}
