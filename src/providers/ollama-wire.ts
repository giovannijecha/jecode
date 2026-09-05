// Translation between the normalized vocabulary and the OpenAI-compatible
// Chat Completions shape Ollama serves at /v1/chat/completions.
//
// This is a third wire format rather than a reuse of the OpenAI provider, and
// the reason is worth stating: Ollama does not implement the Responses API.
// Chat Completions differs on two points that matter here — a tool result is a
// message of its own with role "tool", not a block inside a user turn, and tool
// arguments travel as a JSON string rather than an object.

import type { Block, Message, ToolSpec, Usage } from "../types.ts";
import { toolInputFromJson } from "./tool-input.ts";
import { wireTokenCount } from "./wire-usage.ts";

export type ChatToolCall = { id: string; name: string; args: string };

export type ChatReply = {
  content: string;
  reasoning: string;
  toolCalls: ChatToolCall[];
  finishReason?: string;
  usage?: { prompt_tokens?: unknown; completion_tokens?: unknown };
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
      const reasoning = ollamaReasoning(message);
      if (reasoning !== undefined) turn["reasoning"] = reasoning;
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

  const acceptsToolCalls = reply.finishReason === "tool_calls";
  if (acceptsToolCalls) {
    for (const call of reply.toolCalls) {
      content.push({
        kind: "tool_call",
        id: call.id,
        name: call.name,
        ...toolInputFromJson(call.args),
      });
    }
  }

  const notice = stopNotice(reply);
  if (notice !== undefined) content.push({ kind: "text", text: notice });

  const suppressedToolCall = reply.toolCalls.length > 0 && !acceptsToolCalls;
  const retainRaw = reply.finishReason !== "length" && !suppressedToolCall;
  const raw = !retainRaw || reply.reasoning === "" ? undefined : { reasoning: reply.reasoning };
  return {
    role: "assistant",
    content,
    ...(raw === undefined ? {} : { raw, rawFrom: "ollama" }),
    usage: normalizeUsage(reply),
  };
}

function ollamaReasoning(message: Message): string | undefined {
  if (
    message.rawFrom !== "ollama" ||
    typeof message.raw !== "object" ||
    message.raw === null ||
    Array.isArray(message.raw)
  ) return undefined;
  const reasoning = (message.raw as Record<string, unknown>)["reasoning"];
  return typeof reasoning === "string" && reasoning !== "" ? reasoning : undefined;
}

function normalizeUsage(reply: ChatReply): Usage | undefined {
  if (reply.usage === undefined) return undefined;
  return {
    inputTokens: wireTokenCount(reply.usage.prompt_tokens),
    outputTokens: wireTokenCount(reply.usage.completion_tokens),
    cachedInputTokens: 0,
    cacheWriteInputTokens: 0,
    reasoningTokens: 0,
  };
}

export function stopNotice(reply: ChatReply): string | undefined {
  return reply.finishReason === "length"
    ? "[truncated: hit the output limit — raise max output tokens in /settings]"
    : undefined;
}
