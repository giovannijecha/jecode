// Translation between the normalized vocabulary and the OpenAI-compatible
// Chat Completions shape Ollama serves at /v1/chat/completions.
//
// This is a third wire format rather than a reuse of the OpenAI provider, and
// the reason is worth stating: Ollama does not implement the Responses API.
// Chat Completions differs on two points that matter here — a tool result is a
// message of its own with role "tool", not a block inside a user turn, and tool
// arguments travel as a JSON string rather than an object.

import type { Block, Message, ToolSpec, Usage } from "../types.ts";

export type ChatToolCall = { id: string; name: string; args: string };

export type ChatReply = {
  content: string;
  toolCalls: ChatToolCall[];
  finishReason?: string;
  usage?: { prompt_tokens?: number; completion_tokens?: number };
};

export function toWireTool(tool: ToolSpec) {
  return {
    type: "function",
    function: { name: tool.name, description: tool.description, parameters: tool.input },
  };
}

// Flattens the history: one normalized message can become several wire
// messages, because every tool result is its own turn.
export function toWireMessages(system: string, messages: Message[]): unknown[] {
  const wire: unknown[] = [];
  if (system !== "") wire.push({ role: "system", content: system });

  for (const message of messages) {
    const texts: string[] = [];
    const toolCalls: unknown[] = [];

    for (const block of message.content) {
      if (block.kind === "text") {
        texts.push(block.text);
      } else if (block.kind === "tool_call") {
        toolCalls.push({
          id: block.id,
          type: "function",
          function: { name: block.name, arguments: JSON.stringify(block.input) },
        });
      } else {
        wire.push({ role: "tool", tool_call_id: block.id, content: block.output });
      }
    }

    if (message.role === "assistant") {
      if (texts.length === 0 && toolCalls.length === 0) continue;
      const turn: Record<string, unknown> = { role: "assistant", content: texts.join("\n") };
      // An empty tool_calls array is not the same as no tool_calls to every
      // server, so the key is omitted rather than sent empty.
      if (toolCalls.length > 0) turn["tool_calls"] = toolCalls;
      wire.push(turn);
    } else if (texts.length > 0) {
      wire.push({ role: "user", content: texts.join("\n") });
    }
  }

  return wire;
}

export function fromWireReply(reply: ChatReply): Message {
  const content: Block[] = [];
  if (reply.content !== "") content.push({ kind: "text", text: reply.content });

  for (const call of reply.toolCalls) {
    content.push({ kind: "tool_call", id: call.id, name: call.name, input: parseArgs(call.args) });
  }

  // No `raw`: unlike the other two providers, nothing in this shape has to be
  // echoed back verbatim, so the normalized blocks are the whole message.
  return { role: "assistant", content, usage: normalizeUsage(reply) };
}

function normalizeUsage(reply: ChatReply): Usage | undefined {
  if (reply.usage === undefined) return undefined;
  return {
    inputTokens: reply.usage.prompt_tokens ?? 0,
    outputTokens: reply.usage.completion_tokens ?? 0,
    cachedInputTokens: 0,
    cacheWriteInputTokens: 0,
    reasoningTokens: 0,
  };
}

export function stopNotice(reply: ChatReply): string | undefined {
  return reply.finishReason === "length"
    ? "[truncated: hit the output limit — raise --max-tokens]"
    : undefined;
}

// A model that emits malformed JSON gets the empty object, which fails
// validation in tools/args.ts with a message written for it to read. That is a
// recoverable turn; throwing here would end the whole thing instead.
function parseArgs(args: string): Record<string, unknown> {
  if (args.trim() === "") return {};
  try {
    const parsed = JSON.parse(args) as unknown;
    return typeof parsed === "object" && parsed !== null ? (parsed as Record<string, unknown>) : {};
  } catch {
    return {};
  }
}
