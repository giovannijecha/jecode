// Probe definitions and workload fields used to reject misleading comparisons.

export const probes = [
  { name: "context", benchmark: "context-responsiveness",
    files: ["context.ts", "context-workflow.ts", "context-integrated.ts", "context-fixture.ts", "corpus.ts", "samples.ts",
      "../test-support/app.ts", "../test-support/app-harness.ts", "../test-support/responses-server.ts"],
    workload: ["iterations", "tokenizer.inputCharacters", "request.inputCharacters",
      "planning.messages", "planning.inputCharacters", "workflows.*.reads", "workflows.*.outputCharactersPerRead",
      "?integrated.workload", "?mixedTokenizer.inputCharacters", "?mixedTokenizer.seed"] },
  { name: "redaction", benchmark: "streaming-redaction", files: ["redaction.ts", "samples.ts"],
    workload: ["secrets", "outputCharacters", "iterations", "?cases.*.name", "?cases.*.chunks", "?cases.*.inputCharacters"] },
  { name: "search", benchmark: "workspace-search", files: ["search.ts", "samples.ts"],
    workload: ["files", "fileBytes", "inputBytes", "iterations", "?cases.*.name", "?correctness"] },
  { name: "session", benchmark: "durable-session-store", files: ["session.ts", "samples.ts"],
    workload: ["checkpoint.iterations", "checkpoint.results.*.nodes", "catalog.iterations",
      "catalog.sessions", "catalog.results.*.nodesPerSession", "load.iterations", "load.results.*.nodes",
      "?cardinality.*.sessions", "?cardinality.*.nodesPerSession"] },
  { name: "transcript", benchmark: "incremental-transcript", files: ["transcript.ts"],
    workload: ["blocks", "liveReasoningCharacters", "viewports.*.columns", "cachedResize.frames",
      "stable.frames", "streaming.frames", "expandedLive.frames"] },
  { name: "tui", benchmark: "tui-responsiveness",
    files: ["tui.ts", "tui-scenario.ts", "tui-host.ts", "tui-output.ts",
      "../test-support/app.ts", "../test-support/controller.ts"],
    workload: ["color", "reducedMotion", "forcedGcBetweenPhases", "measurement",
      "results.*.columns", "results.*.rows", "results.*.historyBlocks", "results.*.warmup",
      "results.*.outputBytesPerSecond", "results.*.scenarios.*.samples"] },
] as const;

export type Probe = typeof probes[number];

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function workload(results: Record<string, unknown>, probe: Probe): unknown[] {
  return probe.workload.map((pattern) => {
    // New scenarios can be absent in a historical report. Their absence is part
    // of the signature, never equivalent to a present scenario or zero work.
    if (pattern.startsWith("?")) {
      try { return select(results, pattern.slice(1).split(".")); } catch { return null; }
    }
    return select(results, pattern.split("."));
  });
}

/** Stable headline metrics; tiny percentage changes must not crowd out the workload. */
export function primaryMetric(probe: string, path: string): boolean {
  if (probe === "context") return /^(tokenizer\.coldMaximumStallMilliseconds|mixedTokenizer\.medianMilliseconds|integrated\.(elapsedMilliseconds|resumeMilliseconds|terminationMilliseconds|(measurement|checkpoint|preparation)\.medianMilliseconds))$/.test(path);
  if (probe === "session") return /^(checkpoint\.results\.1|catalog\.results\.1|load\.results\.3|cardinality\.\d+)\.medianMilliseconds$/.test(path);
  if (probe === "search") return /^cases\.\d+\.medianMilliseconds$/.test(path);
  if (probe === "redaction") return /^cases\.\d+\.(setup|streaming)\.medianMilliseconds$/.test(path);
  if (probe === "transcript") return /^(streaming\.millisecondsPerFrame|viewports\.\d+\.firstViewportMilliseconds)$/.test(path);
  return /^results\.\d+\.scenarios\.typingWhileStreaming\.inputToFrameMilliseconds\.p95$/.test(path);
}

function select(value: unknown, parts: string[]): unknown {
  const [part, ...rest] = parts;
  if (part === undefined) return value;
  if (part === "*") {
    if (Array.isArray(value)) return value.map((entry) => select(entry, rest));
    if (isRecord(value)) return Object.keys(value).sort().map((key) => [key, select(value[key], rest)]);
  } else if (isRecord(value) && Object.hasOwn(value, part)) return select(value[part], rest);
  throw new Error("missing workload field");
}
