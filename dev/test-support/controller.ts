import type { Message, Provider, SendRequest } from "../../src/types.ts";
import type { ControllerEvents, ControllerOptions } from "../../src/controller.ts";
import { policyForContextWindow } from "../../src/context/policy.ts";
import type { Tool } from "../../src/tools/index.ts";

export type FakeProvider = Provider & { seen: SendRequest[] };

export function scripted(replies: Message[]): FakeProvider {
  const seen: SendRequest[] = [];
  let next = 0;
  return {
    id: "fake",
    defaultModel: "fake-1",
    auth: { kind: "api-key", keyVar: "FAKE_API_KEY" },
    seen,
    blocked: () => undefined,
    models: () => Promise.resolve(["fake-1"]),
    async send(request: SendRequest): Promise<Message> {
      seen.push(request);
      const reply = replies[next];
      next += 1;
      if (reply === undefined) throw new Error("the script ran out of replies");
      // Stand in for a provider streaming its text before it resolves.
      for (const block of reply.content) {
        if (block.kind === "text") request.onStream?.({ kind: "text", text: block.text });
      }
      return reply;
    },
  };
}

export const echo: Tool = {
  name: "echo",
  description: "echoes",
  dangerous: false,
  concurrency: "shared",
  input: { type: "object", properties: { text: { type: "string" } }, required: ["text"] },
  async run(args) {
    if (args.text === "boom") throw new Error("exploded");
    return { output: String(args.text) };
  },
};

export const destroy: Tool = {
  name: "destroy",
  description: "needs approval",
  dangerous: true,
  concurrency: "exclusive",
  input: { type: "object", properties: {}, required: [] },
  async run() {
    return { output: "destroyed" };
  },
};

export function options(provider: Provider, overrides: Partial<ControllerOptions> = {}): ControllerOptions {
  return {
    provider,
    tools: [echo, destroy],
    model: "fake-1",
    system: "be useful",
    maxTokens: 100,
    contextPolicy: () => Promise.resolve(policyForContextWindow(undefined, 85)),
    effort: "high",
    toolContext: { root: process.cwd() },
    ...overrides,
  };
}

export function events(approve = true): ControllerEvents & { texts: string[] } {
  const texts: string[] = [];
  return {
    texts,
    onStream(event) {
      if (event.kind !== "tool") texts.push(event.text);
    },
    onToolCall() {},
    onToolResult() {},
    async approve() {
      return approve;
    },
  };
}

export function assistantText(text: string): Message {
  return { role: "assistant", content: [{ kind: "text", text }] };
}

export function texts(messages: readonly Message[]): string[] {
  return messages.flatMap((message) => message.content)
    .filter((block) => block.kind === "text")
    .map((block) => block.text);
}

export function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export function aborted(signal: AbortSignal | undefined): Promise<never> {
  return new Promise((_resolve, reject) => {
    const stop = () => reject(
      signal?.reason instanceof Error ? signal.reason : new Error("interrupted"),
    );
    if (signal?.aborted === true) stop();
    else signal?.addEventListener("abort", stop, { once: true });
  });
}
