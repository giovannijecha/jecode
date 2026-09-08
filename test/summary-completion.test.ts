import { test } from "node:test";
import assert from "node:assert/strict";
import { compactContext } from "../src/context/compactor.ts";
import { policyForContextWindow } from "../src/context/policy.ts";
import { fromWireResponse as openai } from "../src/providers/openai-wire.ts";
import { fromWireResponse as anthropic } from "../src/providers/anthropic-wire.ts";
import { fromWireReply as ollama } from "../src/providers/ollama-wire.ts";
import { provider } from "../dev/test-support/app.ts";
import { safeDiagnostic } from "../src/context/diagnostics.ts";
import type { Message } from "../src/types.ts";

const text = "Only the first part of the task.";
const user = (value: string): Message => ({ role: "user", content: [{ kind: "text", text: value }] });
const cases: [string, () => Message][] = [
  ["OpenAI incomplete", () => openai({ status: "incomplete", incomplete_details: { reason: "max_output_tokens" },
    output: [{ type: "message", content: [{ type: "output_text", text }] }] })],
  ["OpenAI refusal", () => openai({ status: "completed",
    output: [{ type: "message", content: [{ type: "refusal", refusal: "Cannot summarize." }] }] })],
  ["Anthropic output limit", () => anthropic({ stop_reason: "max_tokens", content: [{ type: "text", text }] })],
  ["Anthropic pause", () => anthropic({ stop_reason: "pause_turn", content: [{ type: "text", text }] })],
  ["Anthropic refusal", () => anthropic({ stop_reason: "refusal", content: [] })],
  ["Ollama output limit", () => ollama({ content: text, reasoning: "", toolCalls: [], finishReason: "length" })],
  ["Ollama content filter", () => ollama({ content: text, reasoning: "", toolCalls: [], finishReason: "content_filter" })],
  ["unexpected tool call", () => ({ role: "assistant", content: [{ kind: "text", text },
    { kind: "tool_call", id: "unexpected", name: "run_command", input: {} }] })],
];

for (const [name, reply] of cases) {
  test(`${name} cannot replace context with an unusable summary`, async () => {
    const history = [user("Earlier work ".repeat(3_000)), user("Current requirement.")];
    const before = structuredClone(history);
    let requests = 0;
    let accounted = 0;
    const outcomes: string[] = [];
    const result = await compactContext({
      provider: { ...provider(), async send() { requests++; return { ...reply(), usage: {
        inputTokens: 1_000, outputTokens: 20, cachedInputTokens: 0, cacheWriteInputTokens: 0, reasoningTokens: 0,
      } }; } },
      model: "fixture", effort: "high", context: history, turn: [history.at(-1)!],
      nodeId: 1, coveredMessages: 0, lastInputTokens: 0, estimatedInputTokens: 18_000, force: true,
      policy: policyForContextWindow({ tokens: 64_000 }, 85),
      onUsage: () => accounted++, onDiagnostic: value => outcomes.push(safeDiagnostic(value)?.outcome ?? "missing"),
    });
    assert.equal(requests, 1, "the provider supplied the rejected summary");
    assert.equal(accounted, 1);
    assert.deepEqual(outcomes, ["incomplete"]);
    assert.equal(result, undefined);
    assert.deepEqual(history, before);
  });
}

for (const [name, reply] of [
  ["OpenAI", openai({ status: "completed", output: [{ type: "message", content: [{ type: "output_text", text }] }] })],
  ["Anthropic", anthropic({ stop_reason: "end_turn", content: [{ type: "text", text }] })],
  ["Ollama", ollama({ finishReason: "stop", content: text, reasoning: "", toolCalls: [] })],
  ["literal notice text", openai({ status: "completed", output: [{ type: "message", content: [
    { type: "output_text", text: "The UI displayed [truncated: ...]; the user then completed the task." },
  ] }] })],
] as const) {
  test(`${name} complete summaries remain usable without guessing from their wording`, async () => {
    const history = [user("Earlier work ".repeat(3_000)), user("Current requirement.")];
    const result = await compactContext({
      provider: { ...provider(), async send() { return reply; } },
      model: "fixture", effort: "high", context: history, turn: [history.at(-1)!],
      nodeId: 1, coveredMessages: 0, lastInputTokens: 0, estimatedInputTokens: 18_000, force: true,
      policy: policyForContextWindow({ tokens: 64_000 }, 85),
    });
    assert.ok(result?.anchor);
    assert.deepEqual(result.messages.at(-1), history.at(-1));
  });
}
