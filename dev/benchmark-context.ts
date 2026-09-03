// Manual responsiveness probe for request estimation and compaction planning.

import type { Message } from "../src/types.ts";
import { estimateRequestInputTokensResponsive } from "../src/context/budget.ts";
import { planCompaction, policyForContextWindow } from "../src/context/policy.ts";

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

let inputTokens = 0;
const request = await sample(async () => {
  inputTokens = await estimateRequestInputTokensResponsive({
    system: "",
    messages: largeRequest,
    tools: [],
  });
});
const planning = await sample(async () => {
  const plan = await planCompaction(
    largeContext,
    largeContext,
    0,
    inputTokens,
    true,
    policyForContextWindow({ tokens: 10_000_000 }, 85),
    inputTokens,
  );
  if (plan === undefined) throw new Error("forced benchmark plan was not produced");
});

for (const [label, result] of [["request estimate", request], ["forced plan", planning]] as const) {
  if (result.total > MAX_TOTAL_MS) throw new Error(`${label} exceeded ${MAX_TOTAL_MS} ms`);
  if (result.maxStall > MAX_EVENT_LOOP_STALL_MS) {
    throw new Error(`${label} blocked the event loop for ${result.maxStall.toFixed(2)} ms`);
  }
}

process.stdout.write([
  `request input: ${(8).toFixed(0)} MiB`,
  `iterations: ${ITERATIONS}`,
  `request estimate: ${request.total.toFixed(2)} ms total · ${request.maxStall.toFixed(2)} ms max stall`,
  "forced plan: 128 messages / 4 MiB",
  `planning: ${planning.total.toFixed(2)} ms total · ${planning.maxStall.toFixed(2)} ms max stall`,
  `thresholds: ${MAX_TOTAL_MS} ms total · ${MAX_EVENT_LOOP_STALL_MS} ms max stall`,
].join("\n") + "\n");

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
