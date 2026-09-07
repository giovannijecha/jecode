// Probe definitions and workload fields used to reject misleading comparisons.

export const probes = [
  { name: "context", benchmark: "context-responsiveness",
    files: ["context.ts", "context-workflow.ts"],
    workload: ["iterations", "tokenizer.inputCharacters", "request.inputCharacters",
      "planning.messages", "planning.inputCharacters", "workflows.*.reads", "workflows.*.outputCharactersPerRead"] },
  { name: "redaction", benchmark: "streaming-redaction", files: ["redaction.ts"],
    workload: ["secrets", "outputCharacters", "iterations"] },
  { name: "search", benchmark: "workspace-search", files: ["search.ts"],
    workload: ["files", "fileBytes", "inputBytes", "iterations"] },
  { name: "session", benchmark: "durable-session-store", files: ["session.ts"],
    workload: ["checkpoint.iterations", "checkpoint.results.*.nodes", "catalog.iterations",
      "catalog.sessions", "catalog.results.*.nodesPerSession", "load.iterations", "load.results.*.nodes"] },
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
  return probe.workload.map((pattern) => select(results, pattern.split(".")));
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
