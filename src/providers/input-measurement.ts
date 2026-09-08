// Measure the content each adapter actually sends, without exposing wire
// formats to context policy or counting the normalized and raw copies twice.

import type { Message, RequestInput } from "../types.ts";
import { estimateSerializedTokensResponsive } from "../context/estimate.ts";
import { toWireItems, toWireTool as responsesTool } from "./openai-wire.ts";
import { toWireMessage, toWireTool as anthropicTool } from "./anthropic-wire.ts";
import { toWireMessages, toWireTool as ollamaTool } from "./ollama-wire.ts";
import { countO200k } from "../context/tokenizer/o200k.ts";

type WireMessage = Readonly<{ items: readonly unknown[]; outputTokens?: number; unmeasuredOpaque?: boolean }>;

export function measureResponsesInput(
  request: RequestInput,
  providerId: string,
  signal?: AbortSignal,
): Promise<number> {
  return measure(
    { instructions: request.system, tools: request.tools.map(responsesTool) },
    request.messages.map((message) => {
      const items = toWireItems(message, providerId);
      const outputTokens = opaqueReserve(message, providerId);
      if (outputTokens === undefined) return { items,
        unmeasuredOpaque: items.some((item) => hasOpaqueField(item, "reasoning", "encrypted_content")) };
      return {
        items: items.map((item) => withoutOpaqueField(item, "reasoning", "encrypted_content")),
        outputTokens,
      };
    }),
    request.tools.length,
    signal,
    responsesTokenCounter(request.model, providerId),
  );
}

export function measureAnthropicInput(request: RequestInput, signal?: AbortSignal): Promise<number> {
  return measure(
    { system: request.system, tools: request.tools.map(anthropicTool) },
    request.messages.map((message) => {
      const wire = toWireMessage(message);
      const outputTokens = opaqueReserve(message, "anthropic");
      if (outputTokens === undefined || !Array.isArray(wire.content)) return { items: [wire] };
      return {
        items: [{
          ...wire,
          content: wire.content.map((item: unknown) => withoutOpaqueField(
            withoutOpaqueField(item, "thinking", "signature"),
            "redacted_thinking",
            "data",
          )),
        }],
        outputTokens,
      };
    }),
    request.tools.length,
    signal,
  );
}

export function measureOllamaInput(request: RequestInput, signal?: AbortSignal): Promise<number> {
  return measure(
    { messages: toWireMessages(request.system, []), tools: request.tools.map(ollamaTool) },
    request.messages.map((message) => ({ items: toWireMessages("", [message]) })),
    request.tools.length,
    signal,
  );
}

async function measure(
  envelope: unknown,
  messages: readonly WireMessage[],
  toolCount: number,
  signal?: AbortSignal,
  count = estimateSerializedTokensResponsive,
): Promise<number> {
  let tokens = 64 + toolCount * 16 + await count(envelope, signal);
  for (const message of messages) {
    if (message.items.length === 0) continue;
    const visible = await (message.unmeasuredOpaque ? estimateSerializedTokensResponsive : count)(message.items, signal);
    // Reported output already includes hidden reasoning. Reserve it once for
    // this assistant message, never once per opaque item or in addition to it.
    tokens += Math.max(visible, message.outputTokens ?? 0) + message.items.length * 8;
  }
  return tokens;
}

/** Account aliases may precede tiktoken's model map: use a reference encoding,
 * never label this a provider-exact count. Legacy/unknown API routes stay conservative. */
function responsesTokenCounter(model: string, providerId: string): typeof estimateSerializedTokensResponsive {
  if (responsesTokenization(model, providerId) === "heuristic") return estimateSerializedTokensResponsive;
  return async (value, signal) => Math.max(1, Math.ceil(await countO200k(JSON.stringify(value), signal) * 1.1));
}

export function responsesTokenization(model: string, providerId: string): "o200k-reference" | "heuristic" {
  const modern = /^(?:gpt-6-astra(?:-|$)|gpt-5|gpt-4\.[15](?:-|$)|gpt-4o(?:-|$)|chatgpt-4o-|o[13](?:-|$)|o4-mini(?:-|$)|ft:gpt-4o)/u.test(model);
  return providerId === "openai-codex" || modern ? "o200k-reference" : "heuristic";
}

function opaqueReserve(message: Message, providerId: string): number | undefined {
  const tokens = message.usage?.outputTokens;
  return message.role === "assistant" && message.rawFrom === providerId &&
      Array.isArray(message.raw) && typeof tokens === "number" &&
      Number.isSafeInteger(tokens) && tokens > 0 && message.raw.some((item: unknown) => (
        providerId === "anthropic"
          ? hasOpaqueField(item, "thinking", "signature") ||
            hasOpaqueField(item, "redacted_thinking", "data")
          : hasOpaqueField(item, "reasoning", "encrypted_content")
      ))
    ? tokens
    : undefined;
}

function withoutOpaqueField(value: unknown, type: string, field: string): unknown {
  if (!hasOpaqueField(value, type, field)) return value;
  const visible = { ...(value as Record<string, unknown>) };
  delete visible[field];
  return visible;
}

function hasOpaqueField(value: unknown, type: string, field: string): boolean {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const item = value as Record<string, unknown>;
  return item["type"] === type && typeof item[field] === "string";
}
