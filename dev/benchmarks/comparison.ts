// Compare repeated measurements only when environment, probe, and workload agree.

import type { Collection } from "./collection.ts";
import { isRecord, probes, workload } from "./probes.ts";

interface Distribution { median: number; min: number; max: number }
interface Metric {
  path: string;
  unit: "ms" | "bytes";
  baseline: Distribution;
  current: Distribution;
  changePercent: number | null;
  rangesOverlap: boolean;
}
export interface Comparison {
  schema: 1;
  baseline: string;
  current: string;
  environment: Collection["environment"];
  probes: Array<{ name: string; status: "compared" | "incompatible" | "failed";
    reason: string | null; metrics: Metric[]; unavailable: string[] }>;
}

export function compare(baseline: Collection, current: Collection): Comparison {
  const result: Comparison = { schema: 1, baseline: baseline.commit, current: current.commit,
    environment: current.environment, probes: [] };
  const environmentMatches = JSON.stringify(Object.entries(baseline.environment).sort()) ===
    JSON.stringify(Object.entries(current.environment).sort());
  for (const [index, probe] of probes.entries()) {
    const before = baseline.probes[index]!;
    const after = current.probes[index]!;
    const samples = [...before.samples, ...after.samples];
    const entry: Comparison["probes"][number] = { name: probe.name, status: "compared", reason: null,
      metrics: [], unavailable: [] };
    result.probes.push(entry);
    if (samples.some((sample) => sample.failure !== null || sample.exitCode !== 0 ||
      sample.results === null || sample.results["passed"] === false)) {
      entry.status = "failed";
      entry.reason = "A probe failed; inspect the raw samples and stderr.";
    } else if (!environmentMatches || baseline.dirty || current.dirty ||
      before.sourceHash !== after.sourceHash || baseline.repetitions !== current.repetitions) {
      entry.status = "incompatible";
      entry.reason = "Environment, clean checkout, probe source, or repetition count differs.";
    } else {
      try {
        const signature = JSON.stringify(workload(samples[0]!.results!, probe));
        if (samples.some((sample) => JSON.stringify(workload(sample.results!, probe)) !== signature)) {
          throw new Error("workload differs");
        }
        const baselineMetrics = before.samples.map((sample) => metrics(sample.results!));
        const currentMetrics = after.samples.map((sample) => metrics(sample.results!));
        const all = [...baselineMetrics, ...currentMetrics];
        const paths = new Set(all.flatMap((sample) => [...sample.keys()]));
        for (const path of [...paths].sort()) {
          if (all.some((sample) => !sample.has(path))) { entry.unavailable.push(path); continue; }
          const a = distribution(baselineMetrics.map((sample) => sample.get(path)!.value));
          const b = distribution(currentMetrics.map((sample) => sample.get(path)!.value));
          const percent = a.median === 0 ? null : (b.median / a.median - 1) * 100;
          entry.metrics.push({ path, unit: all[0]!.get(path)!.unit, baseline: a, current: b,
            changePercent: percent !== null && Number.isFinite(percent) ? percent : null,
            rangesOverlap: a.min <= b.max && b.min <= a.max });
        }
        if (entry.metrics.length === 0) throw new Error("no comparable measurements");
      } catch {
        entry.status = "incompatible";
        entry.reason = "Workload or measurement fields differ or are invalid.";
        entry.metrics = [];
      }
    }
  }
  return result;
}

function distribution(values: number[]): Distribution {
  const ordered = values.toSorted((a, b) => a - b);
  const middle = Math.floor(ordered.length / 2);
  return { median: ordered.length % 2 === 0 ? ordered[middle - 1]! / 2 + ordered[middle]! / 2 : ordered[middle]!,
    min: ordered[0]!, max: ordered.at(-1)! };
}

function metrics(results: Record<string, unknown>): Map<string, { value: number; unit: "ms" | "bytes" }> {
  const found = new Map<string, { value: number; unit: "ms" | "bytes" }>();
  const visit = (value: unknown, path: string[], depth: number): void => {
    if (depth > 24) throw new Error("report nesting limit");
    const key = path.at(-1) ?? "";
    if (key === "thresholds" || key === "diagnostics") return;
    const label = path.join(".");
    const unit = /Milliseconds|millisecondsPerFrame/.test(label) ? "ms" :
      path.includes("memory") || path.includes("bytesPerWrittenFrame") ||
      (path.includes("output") && (key === "bytes" || key === "peakQueuedBytes")) ? "bytes" : undefined;
    if (typeof value === "number" && unit !== undefined) {
      // Observed depth deltas can legitimately be negative; raw latency/bytes cannot.
      if (!Number.isFinite(value) || (value < 0 && !key.includes("Delta"))) throw new Error("invalid measurement");
      if (!key.includes("Delta")) found.set(label, { value, unit });
    } else if (Array.isArray(value)) value.forEach((entry, index) => visit(entry, [...path, String(index)], depth + 1));
    else if (isRecord(value)) Object.entries(value).forEach(([name, entry]) => visit(entry, [...path, name], depth + 1));
  };
  visit(results, [], 0);
  return found;
}

export function markdown(result: Comparison): string {
  const lines = ["# Benchmark comparison", "",
    `Base: \`${result.baseline}\` · Current: \`${result.current}\``, "",
    "Repeated synthetic measurements. Positive changes mean more time/bytes; they are investigation signals, not regression verdicts.",
    "Job success means evidence was collected, not that all probes passed. Failed probe outcomes remain below.",
    "No new timing gate. Hosted-runner hardware and load can vary; matching metadata does not prove identical conditions.", "",
    "| Probe | Outcome | Compared metrics |", "| --- | --- | ---: |"];
  for (const probe of result.probes) lines.push(`| ${probe.name} | ${probe.status} | ${probe.metrics.length} |`);
  for (const probe of result.probes) {
    lines.push("", `## ${probe.name}`, "");
    if (probe.reason !== null) { lines.push(probe.reason); continue; }
    lines.push("Metrics ordered by percentage change (up to 8); all measurements and ranges are in comparison.json.", "",
      "| Measurement | Unit | Base median [min, max] | Current median [min, max] | Change | Ranges overlap |",
      "| --- | --- | ---: | ---: | ---: | --- |");
    const selected = probe.metrics.toSorted((a, b) => (b.changePercent ?? -Infinity) - (a.changePercent ?? -Infinity)).slice(0, 8);
    for (const metric of selected) {
      const format = (value: number): string => value.toFixed(3);
      const range = (values: Distribution): string => `${format(values.median)} [${format(values.min)}, ${format(values.max)}]`;
      const percent = metric.changePercent === null ? "n/a (zero base)" : `${metric.changePercent.toFixed(1)}%`;
      const label = metric.path.replace(/[|\r\n`<>]/g, "_").slice(0, 200);
      lines.push(`| ${label} | ${metric.unit} | ${range(metric.baseline)} | ${range(metric.current)} | ${percent} | ${metric.rangesOverlap ? "yes" : "no"} |`);
    }
    if (probe.unavailable.length > 0) lines.push("", `${probe.unavailable.length} measurements lack complete numeric samples; no zero substitution.`);
  }
  return `${lines.join("\n")}\n`;
}
