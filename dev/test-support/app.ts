import { ConversationTree } from "../../src/conversation.ts";
import type { Session } from "../../src/session.ts";
import type { Message, Provider, SendRequest } from "../../src/types.ts";
import { STEEL } from "../../src/ui/theme.ts";
import { emptyUsage } from "../../src/usage.ts";

export function provider(reply = "Hello from fake."): Provider {
  return {
    id: "fake",
    defaultModel: "fake-1",
    auth: { kind: "api-key", keyVar: "FAKE_API_KEY" },
    blocked: () => undefined,
    models: () => Promise.resolve(["fake-1"]),
    async send(request: SendRequest): Promise<Message> {
      request.onStream?.({ kind: "text", text: reply });
      return {
        role: "assistant",
        content: [{ kind: "text", text: reply }],
        usage: {
          inputTokens: 7,
          outputTokens: 4,
          cachedInputTokens: 0,
          cacheWriteInputTokens: 0,
          reasoningTokens: 0,
        },
      };
    },
  };
}

export function session(from = provider()): Session {
  return {
    config: {
      providerId: from.id,
      model: from.defaultModel,
      reducedMotion: true,
      effort: "high",
      maxTokens: 4096,
      compactionPercent: 85,
      root: process.cwd(),
      autoApprove: false,
      ephemeral: false,
    },
    provider: from,
    model: from.defaultModel,
    palette: STEEL,
    tools: [],
    system: "be useful",
    conversation: ConversationTree.empty(),
    usage: emptyUsage(),
  };
}

export async function* input(...lines: string[]): AsyncIterable<string> {
  for (const line of lines) yield line;
}

export function messageText(message: Message | undefined): string {
  return message?.content.find((block) => block.kind === "text")?.text ?? "";
}
