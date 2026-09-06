// Inert long-turn probes through the production controller and context manager.

import assert from "node:assert/strict";
import { runTurn } from "../../src/controller.ts";
import { contextManager } from "../../src/context/manager.ts";
import type { ContextDiagnostic } from "../../src/context/manager.ts";
import { automaticCompactionGate } from "../../src/context/automatic.ts";
import { estimateRequestInputTokens } from "../../src/context/budget.ts";
import { policyForContextWindow } from "../../src/context/policy.ts";
import type { Message, Provider } from "../../src/types.ts";
import type { Tool } from "../../src/tools/index.ts";
import { toolSpecs } from "../../src/tools/index.ts";

export async function contextWorkflowProbe(reads: number) {
  const evidence = "export function checked(value) { return value !== undefined ? value : null; }\n"
    .repeat(230).slice(0, 16_384);
  const history: Message[] = [{ role: "user", content: [{ kind: "text", text: "Inspect the source files." }] }];
  const identity = { conversationId: "context-probe", cacheKey: "context-probe" };
  const policy = policyForContextWindow({ tokens: 258_400, compactAtTokens: 244_800 }, 95);
  const diagnostics: ContextDiagnostic[] = [];
  let toolRuns = 0;
  let summaries = 0;
  let requests = 0;
  let peakInputTokens = 0;
  const provider: Provider = {
    id: "fixture", defaultModel: "fixture", auth: { kind: "api-key", keyVar: "UNUSED" },
    blocked: () => undefined, models: async () => ["fixture"],
    async send(request) {
      if (request.identity?.purpose === "compaction") {
        summaries++;
        return { role: "assistant", content: [{ kind: "text", text: "Source inspected; continue the requested review." }] };
      }
      requests++;
      const inputTokens = estimateRequestInputTokens(request);
      peakInputTokens = Math.max(peakInputTokens, inputTokens);
      assert.ok(inputTokens + request.maxTokens <= policy.requestLimitTokens);
      assert.ok(request.messages.flatMap((m) => m.content)
        .filter((b) => b.kind === "tool_result").every((b) => b.output === evidence));
      return {
        role: "assistant",
        content: toolRuns === reads ? [{ kind: "text", text: "Review complete." }]
          : [{ kind: "tool_call", id: `read-${toolRuns}`, name: "fixture", input: {} }],
        usage: { inputTokens, outputTokens: 20, reasoningTokens: 0, cachedInputTokens: 0, cacheWriteInputTokens: 0 },
      };
    },
  };
  const tool: Tool = {
    name: "fixture", description: "Return inert source", dangerous: false, concurrency: "shared",
    input: { type: "object", properties: {} },
    async run() { toolRuns++; return { output: evidence }; },
  };
  const manager = contextManager({
    provider, model: "fixture", effort: "high", system: "Inspect the source.", tools: toolSpecs([tool]),
    maxOutputTokens: 64_000, historyStart: 0, nodeId: () => 1, gate: automaticCompactionGate(),
    identity, onStatus() {}, onUsage() {}, onDiagnostic: (event) => diagnostics.push(event),
  });
  await runTurn(history, {
    provider, model: "fixture", effort: "high", system: "Inspect the source.", tools: [tool],
    maxTokens: 64_000, contextPolicy: async () => policy, requestIdentity: identity,
    toolContext: { root: process.cwd() },
  }, {
    onStream() {}, onToolCall() {}, onToolResult() {}, approve: async () => true,
    onContext: manager.compact,
  });
  assert.equal(toolRuns, reads);
  assert.equal(history.flatMap((m) => m.content).filter((b) => b.kind === "tool_result").length, reads);
  assert.ok(diagnostics.every((d) => d.beforeTokens >= policy.triggerTokens && d.outcome === "accepted"));
  return { reads, outputCharactersPerRead: evidence.length, requests, summaries, peakInputTokens, diagnostics };
}
