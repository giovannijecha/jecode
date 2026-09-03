import { test } from "node:test";
import assert from "node:assert/strict";
import {
  budgetRequest,
  budgetRequestFromInputTokens,
  estimateRequestInputTokens,
  type RequestEnvelope,
} from "../src/context/budget.ts";
import { policyForContextWindow } from "../src/context/policy.ts";

const envelope: RequestEnvelope = {
  system: "Work inside the selected workspace.",
  messages: [{ role: "user", content: [{ kind: "text", text: "inspect the project" }] }],
  tools: [{
    name: "read_file",
    description: "Read a bounded range from one workspace file.",
    input: {
      type: "object",
      properties: { path: { type: "string" } },
      required: ["path"],
    },
  }],
};

test("clamps configured output across small and million-token windows", () => {
  for (const tokens of [4_096, 32_000, 128_000, 997_500]) {
    const budget = budgetRequest(envelope, 64_000, policyForContextWindow({ tokens }, 85));
    assert.ok(budget.maxOutputTokens <= 64_000);
    assert.ok(budget.inputTokens + budget.maxOutputTokens <= budget.limitTokens);
  }
});

test("honors a provider request ceiling stricter than the usable window", () => {
  const policy = policyForContextWindow({
    tokens: 128_000,
    compactAtTokens: 96_000,
  }, 95);
  const budget = budgetRequest(envelope, 100_000, policy);

  assert.equal(budget.limitTokens, 91_200);
  assert.equal(budget.inputTokens + budget.maxOutputTokens, budget.limitTokens);
});

test("reuses an exact request estimate without changing the budget", () => {
  const policy = policyForContextWindow({ tokens: 128_000 }, 85);
  const inputTokens = estimateRequestInputTokens(envelope);

  assert.deepEqual(
    budgetRequestFromInputTokens(inputTokens, 64_000, policy),
    budgetRequest(envelope, 64_000, policy),
  );
  assert.throws(
    () => budgetRequestFromInputTokens(0, 64_000, policy),
    /request input tokens must be a positive safe integer/,
  );
});

test("counts system text and tool schemas as request input", () => {
  const withoutTools = estimateRequestInputTokens({ ...envelope, system: "", tools: [] });
  const complete = estimateRequestInputTokens(envelope);
  assert.ok(complete > withoutTools);
});

test("raises the estimate for poorly compressible request content", () => {
  let seed = 0x12345678;
  const highEntropy = Array.from({ length: 20_000 }, () => {
    seed = (seed * 1_664_525 + 1_013_904_223) >>> 0;
    return String.fromCharCode(33 + seed % 94);
  }).join("");
  const repeated = "a".repeat(highEntropy.length);
  const withText = (text: string): RequestEnvelope => ({
    ...envelope,
    messages: [{ role: "user", content: [{ kind: "text", text }] }],
  });

  const dense = estimateRequestInputTokens(withText(highEntropy));
  const ordinary = estimateRequestInputTokens(withText(repeated));
  assert.ok(dense > ordinary * 2);
});

test("keeps a byte-fallback floor for repeated uncommon Unicode", () => {
  const unusual = "\u{10ffff}".repeat(2_000);
  const candidate: RequestEnvelope = {
    ...envelope,
    messages: [{ role: "user", content: [{ kind: "text", text: unusual }] }],
  };
  const serialized = Buffer.from(JSON.stringify(candidate), "utf8");
  const nonAsciiBytes = serialized.reduce((total, byte) => total + Number(byte >= 0x80), 0);

  assert.ok(estimateRequestInputTokens(candidate) >= nonAsciiBytes);
});

test("rejects an envelope that leaves no useful response budget", () => {
  const oversized: RequestEnvelope = {
    ...envelope,
    tools: [{
      ...envelope.tools[0]!,
      description: "x".repeat(15_000),
    }],
  };

  assert.throws(
    () => budgetRequest(oversized, 64_000, policyForContextWindow({ tokens: 4_096 }, 85)),
    /request input needs approximately .* at least 256 output tokens are required/,
  );
});
