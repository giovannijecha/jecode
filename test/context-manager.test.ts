import { test } from "node:test";
import assert from "node:assert/strict";
import { contextManager } from "../src/context/manager.ts";
import type { ContextDiagnostic } from "../src/context/manager.ts";
import { automaticCompactionGate } from "../src/context/automatic.ts";
import { compactContext } from "../src/context/compactor.ts";
import type { CompactionOutcome } from "../src/context/compactor.ts";
import { policyForContextWindow } from "../src/context/policy.ts";
import { provider } from "../dev/test-support/app.ts";
import type { Message, Usage } from "../src/types.ts";
import { measureResponsesInput } from "../src/providers/input-measurement.ts";
import { contextWorkflowProbe } from "../dev/benchmarks/context-workflow.ts";

const user = (text: string): Message => ({ role: "user", content: [{ kind: "text", text }] });
const policy = policyForContextWindow({ tokens: 64_000 }, 85);
const history = [user("old context ".repeat(11_000)),
  { role: "assistant" as const, content: [{ kind: "text" as const, text: "Prior answer" }] }, user("continue")];

test("long tool turns compact at token pressure without losing canonical evidence", async () => {
  const result = await contextWorkflowProbe(40);
  assert.equal(result.summaries, 1);
  assert.equal(result.requests, 41);
  assert.equal(result.diagnostics[0]?.outcome, "accepted");
});

test("retention counts opaque reasoning so a first long turn can actually compact", async () => {
  const messages = [user("Inspect the source.")];
  for (let i = 0; i < 4; i++) {
    messages.push({
      role: "assistant", content: [{ kind: "tool_call", id: `read-${i}`, name: "read_file", input: {} }],
      rawFrom: "openai-codex",
      raw: [{ type: "reasoning", encrypted_content: "opaque" },
        { type: "function_call", call_id: `read-${i}`, name: "read_file", arguments: "{}" }],
      usage: { inputTokens: 100, outputTokens: 60_000, reasoningTokens: 59_980,
        cachedInputTokens: 0, cacheWriteInputTokens: 0 },
    }, { role: "user", content: [{ kind: "tool_result", id: `read-${i}`, output: "Evidence", isError: false }] });
  }
  const from = { ...provider("Earlier inspection completed."),
    measureInput: (input: import("../src/types.ts").RequestInput, signal?: AbortSignal) =>
      measureResponsesInput(input, "openai-codex", signal) };
  const measured = await from.measureInput({ model: "fixture", effort: "high", system: "", tools: [], messages });
  assert.ok(measured > 232_304);
  const before = structuredClone(messages);
  const result = await compactContext({
    provider: from, model: "fixture", effort: "high", context: messages, turn: messages,
    nodeId: 1, coveredMessages: 0, lastInputTokens: 0, estimatedInputTokens: measured,
    policy: policyForContextWindow({ tokens: 258_400, compactAtTokens: 244_800 }, 95),
  });
  assert.ok(result);
  assert.ok(result.estimatedInputTokens < 100_000);
  assert.equal(result.messages.at(-2)?.content[0]?.kind, "tool_call");
  assert.equal(result.messages.at(-1)?.content[0]?.kind, "tool_result");
  assert.deepEqual(messages, before);
});

test("failed summaries wait for meaningful token growth rather than another tool round", async () => {
  let attempts = 0;
  const diagnostics: ContextDiagnostic[] = [];
  const manager = contextManager({
    provider: { ...provider(), async send() { attempts++; throw new Error("unavailable"); } },
    model: "fake-1", effort: "high", system: "instructions", tools: [], maxOutputTokens: 4_096,
    historyStart: 2, nodeId: () => 2, gate: automaticCompactionGate(),
    identity: { conversationId: "fixture", cacheKey: "fixture" },
    onStatus() {}, onUsage() {}, onDiagnostic: (event) => diagnostics.push(event),
  });
  assert.equal(await manager.compact(history, history, { reason: "budget", policy, inputTokens: 58_000 }), undefined);
  const next = [...history, user("one small addition")];
  assert.equal(await manager.compact(next, next, { reason: "budget", policy, inputTokens: 58_010 }), undefined);
  assert.equal(attempts, 1);
  assert.deepEqual(diagnostics.map((d) => d.outcome), ["failed"]);
  await manager.compact(next, next, { reason: "budget", policy, inputTokens: 62_000 });
  assert.equal(attempts, 2);
  assert.equal(manager.anchor, undefined);
});

test("accepted summaries preserve canonical history, record usage and settle diagnostics", async () => {
  const diagnostics: ContextDiagnostic[] = [];
  const usage: Usage[] = [];
  const status: boolean[] = [];
  const manager = contextManager({
    provider: provider("Durable working state"), model: "fake-1", effort: "high",
    system: "instructions", tools: [], maxOutputTokens: 4_096, historyStart: 2,
    nodeId: () => 2, gate: automaticCompactionGate(),
    identity: { conversationId: "fixture", cacheKey: "fixture" },
    onStatus: (active) => status.push(active), onUsage: (u) => usage.push(u),
    onDiagnostic: (d) => diagnostics.push(d),
  });
  const before = structuredClone(history);
  const result = await manager.compact(history, history, { reason: "budget", policy, inputTokens: 58_000 });
  assert.ok(result);
  assert.deepEqual(history, before);
  assert.deepEqual(status, [true, false]);
  assert.equal(usage.length, 1);
  assert.equal(manager.anchor?.messageCount, 0);
  assert.equal(diagnostics[0]?.outcome, "accepted");
  assert.ok((diagnostics[0]?.afterTokens ?? Infinity) < policy.triggerTokens);
  assert.equal(await manager.compact(history, history, {
    reason: "overflow", policy, inputTokens: 58_000, error: new Error("unrelated network error"),
  }), undefined);
});

test("rejected summaries still account for consumed usage and require useful savings", async () => {
  const outcomes: CompactionOutcome[] = [];
  let accounted = 0;
  const result = await compactContext({
    provider: provider("#".repeat(31_000)), model: "fake-1", effort: "high",
    context: [user("x".repeat(100_000)), user("current")], turn: [user("current")],
    nodeId: 1, coveredMessages: 0, lastInputTokens: 0, estimatedInputTokens: 33_500,
    force: true, policy, onUsage: () => accounted++, onOutcome: (o) => outcomes.push(o),
  });
  assert.equal(result, undefined);
  assert.equal(accounted, 1);
  assert.deepEqual(outcomes, ["insufficient-savings"]);
});

test("summary requests select low only when supported and time out without changing context", async () => {
  const outcomes: CompactionOutcome[] = [];
  let effort: string | undefined;
  // Keep an event-loop handle alive: production fetch owns one during the request.
  const keepAlive = setInterval(() => {}, 100);
  try {
    const result = await compactContext({
      provider: { ...provider(), efforts: async () => ["low", "high"], async send(req) {
        effort = req.effort;
        return new Promise((_resolve, reject) => {
          req.signal?.addEventListener("abort", () => reject(req.signal?.reason), { once: true });
        });
      } },
      model: "fake-1", effort: "high", context: history, turn: [history.at(-1)!],
      nodeId: 1, coveredMessages: 0, lastInputTokens: 0, estimatedInputTokens: 58_000,
      policy, timeoutMs: 25, onOutcome: (outcome) => outcomes.push(outcome),
    });
    assert.equal(result, undefined);
    assert.equal(effort, "low");
    assert.deepEqual(outcomes, ["timeout"]);
  } finally {
    clearInterval(keepAlive);
  }
});
