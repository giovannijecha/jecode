import { test } from "node:test";
import assert from "node:assert/strict";
import { inputMeter, measureInput } from "../src/context/measurement.ts";
import { fitRequestInput } from "../src/context/request.ts";
import { policyForContextWindow } from "../src/context/policy.ts";
import { estimateRequestInputTokens } from "../src/context/budget.ts";
import { provider } from "../dev/test-support/app.ts";
import type { Message, RequestInput } from "../src/types.ts";

function input(text = "old input ".repeat(200)): RequestInput {
  return {
    model: "fake-1", effort: "high", system: "instructions", tools: [],
    messages: [{ role: "user", content: [{ kind: "text", text }] }],
  };
}

test("provider usage anchors only the exact sent prefix plus estimated new input", async () => {
  const meter = inputMeter(provider());
  const before = input();
  const first = await meter.measure(before);
  meter.observe(first, 200);
  const next = { ...before, messages: [...before.messages, ...input("next request").messages] };
  const measured = await meter.measure(next);
  assert.equal(measured.source, "provider-prefix");
  assert.equal(measured.inputTokens, 200 + measured.estimatedTokens - first.estimatedTokens);
  assert.ok(measured.inputTokens < measured.estimatedTokens);
  // Missing or malformed usage must not turn an estimate into a real count.
  for (const missing of [undefined, 0, NaN, -1, 1.5]) meter.observe(measured, missing);
  assert.equal((await meter.measure(next)).inputTokens, measured.inputTokens);
  meter.reset();
  assert.equal((await meter.measure(next)).source, "estimate");
});

test("changed history, model, effort, schemas, and instructions invalidate a token anchor", async () => {
  const meter = inputMeter(provider());
  const before = input();
  meter.observe(await meter.measure(before), 100);
  const changes: RequestInput[] = [
    input("replacement summary"),
    { ...before, messages: [] },
    { ...before, model: "other" },
    { ...before, effort: "low" },
    { ...before, system: "new instructions" },
    { ...before, tools: [{ name: "new", description: "a tool", input: {} }] },
    { ...before, messages: before.messages.map((m) => ({ ...m, raw: ["new opaque data"] })) },
  ];
  for (const changed of changes) {
    const measured = await meter.measure(changed);
    assert.equal(measured.source, "estimate");
    assert.equal(measured.inputTokens, measured.estimatedTokens);
  }
});

test("fallback estimation excludes internal raw copies and usage metadata", async () => {
  const before = input();
  const withInternal = {
    ...before,
    messages: before.messages.map((m) => ({ ...m, raw: "duplicate ".repeat(1000), rawFrom: "foreign" })),
  };
  assert.equal(await measureInput(provider(), before), await measureInput(provider(), withInternal));
});

test("provider measurement failures and cancellation prevent unsafe budgeting", async () => {
  for (const tokens of [0, NaN, -5, 1.2]) {
    await assert.rejects(measureInput({ ...provider(), measureInput: async () => tokens }, input()),
      /invalid input token estimate/);
  }
  const control = new AbortController();
  const reason = new Error("cancel estimate");
  const meter = inputMeter({ ...provider(), async measureInput() { control.abort(reason); return 50; } });
  await assert.rejects(meter.measure(input(), control.signal), (error) => error === reason);
});

test("ordinary input keeps full tool evidence and does not spend a separate character budget", async () => {
  const messages: Message[] = [{ role: "user", content: [
    { kind: "tool_result", id: "read", output: "a".repeat(60_000), isError: false },
  ] }];
  const request = { ...input(), messages };
  const meter = inputMeter(provider());
  const measured = await meter.measure(request);
  const fitted = await fitRequestInput(request, meter,
    policyForContextWindow({ tokens: 258_400, compactAtTokens: 244_800 }, 95), measured);
  assert.equal(fitted.clippedResults, 0);
  assert.deepEqual(fitted.messages, messages);
});

test("emergency projection rebudgets Unicode and refuses an irreducible schema", async () => {
  const request = { ...input("small"), messages: [{ role: "user" as const, content: [
    { kind: "tool_result" as const, id: "read", output: "\u{10ffff}".repeat(10_000), isError: false },
  ] }] };
  const meter = inputMeter(provider());
  const policy = policyForContextWindow({ tokens: 4_096 }, 85);
  const fitted = await fitRequestInput(request, meter, policy, await meter.measure(request));
  assert.equal(fitted.clippedResults, 1);
  assert.ok(fitted.measurement.inputTokens + 256 <= policy.requestLimitTokens);
  assert.deepEqual(request.messages[0]?.content[0]?.output, "\u{10ffff}".repeat(10_000));
  const irreducible = { ...request, system: "#".repeat(10_000) };
  const blocked = await fitRequestInput(irreducible, meter, policy, await meter.measure(irreducible));
  assert.ok(blocked.measurement.inputTokens > policy.requestLimitTokens);
  assert.ok(estimateRequestInputTokens(blocked.messages.length ? { ...irreducible, messages: blocked.messages } : irreducible) > policy.requestLimitTokens);
});
