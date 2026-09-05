// Manual responsiveness probe for request estimation and compaction planning.

import type { Message } from "../../src/types.ts";
import { estimateRequestInputTokensResponsive } from "../../src/context/budget.ts";
import { planCompaction, policyForContextWindow } from "../../src/context/policy.ts";
import { reportBenchmark, round } from "./report.ts";
import { contextWorkflowProbe } from "./context-workflow.ts";

const MAX_TOTAL_MS = 2_000;
const MAX_EVENT_LOOP_STALL_MS = 75;
const ITERATIONS = 5;
const largeContext: Message[] = Array.from({ length: 128 }, (_, index) => ({
  role: index % 2 === 0 ? "user" : "assistant",
  content: [{
    kind: "text",
    text: String.fromCharCode(65 + index % 20).repeat(32_768),
  }],
}));
const largeRequest: Message[] = [{
  role: "user",
  content: [{ kind: "text", text: "x".repeat(8 * 1_024 * 1_024 - 1_024) }],
}];

let requestInputTokens = 0;
const request = await sample(async () => {
  requestInputTokens = await estimateRequestInputTokensResponsive({
    system: "",
    messages: largeRequest,
    tools: [],
  });
});
const contextInputTokens = await estimateRequestInputTokensResponsive({
  system: "",
  messages: largeContext,
  tools: [],
});
const planning = await sample(async () => {
  const plan = await planCompaction(
    largeContext,
    largeContext,
    0,
    contextInputTokens,
    true,
    policyForContextWindow({ tokens: 10_000_000 }, 85),
    contextInputTokens,
  );
  if (plan === undefined) throw new Error("forced benchmark plan was not produced");
});

const shortWorkflow = await contextWorkflowProbe(12);
const longWorkflow = await contextWorkflowProbe(40);
const workflowPassed = shortWorkflow.summaries === 0 && longWorkflow.summaries > 0 &&
  longWorkflow.summaries <= 2;

reportBenchmark("context-responsiveness", {
  iterations: ITERATIONS,
  request: {
    inputCharacters: 8 * 1_024 * 1_024 - 1_024,
    inputTokens: requestInputTokens,
    medianMilliseconds: round(request.total),
    medianMaximumStallMilliseconds: round(request.maxStall),
  },
  planning: {
    messages: largeContext.length,
    inputCharacters: 128 * 32_768,
    medianMilliseconds: round(planning.total),
    medianMaximumStallMilliseconds: round(planning.maxStall),
  },
  thresholds: {
    medianMilliseconds: MAX_TOTAL_MS,
    medianMaximumStallMilliseconds: MAX_EVENT_LOOP_STALL_MS,
  },
  workflows: [shortWorkflow, longWorkflow],
  passed: workflowPassed && [request, planning].every((result) =>
    result.total <= MAX_TOTAL_MS && result.maxStall <= MAX_EVENT_LOOP_STALL_MS
  ),
});

if (!workflowPassed) throw new Error("context workflow compacted too often or failed to compact");

for (const [label, result] of [["request estimate", request], ["forced plan", planning]] as const) {
  if (result.total > MAX_TOTAL_MS) throw new Error(`${label} exceeded ${MAX_TOTAL_MS} ms`);
  if (result.maxStall > MAX_EVENT_LOOP_STALL_MS) {
    throw new Error(`${label} blocked the event loop for ${result.maxStall.toFixed(2)} ms`);
  }
}

async function measure(run: () => Promise<void>): Promise<Readonly<{
  total: number;
  maxStall: number;
}>> {
  let previous = performance.now();
  let maxStall = 0;
  const timer = setInterval(() => {
    const now = performance.now();
    maxStall = Math.max(maxStall, now - previous);
    previous = now;
  }, 1);
  const started = performance.now();
  try {
    await run();
  } finally {
    clearInterval(timer);
  }
  const total = performance.now() - started;
  maxStall = Math.max(maxStall, performance.now() - previous);
  return { total, maxStall };
}

async function sample(run: () => Promise<void>): Promise<Readonly<{
  total: number;
  maxStall: number;
}>> {
  await measure(run);
  const measurements = [];
  for (let iteration = 0; iteration < ITERATIONS; iteration++) {
    measurements.push(await measure(run));
  }
  const total = median(measurements.map((measurement) => measurement.total));
  const maxStall = median(measurements.map((measurement) => measurement.maxStall));
  return { total, maxStall };
}

function median(values: number[]): number {
  values.sort((left, right) => left - right);
  return values[Math.floor(values.length / 2)] as number;
}
