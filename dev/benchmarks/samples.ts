// Keep individual observations alongside summaries; assertions run after timing.
import assert from "node:assert/strict";
import { round } from "./report.ts";

export function distribution(values: readonly number[]) {
  assert.ok(values.length > 0 && values.every(value => Number.isFinite(value) && value >= 0));
  const ordered = values.toSorted((a, b) => a - b);
  return {
    medianMilliseconds: round(ordered[Math.floor(ordered.length / 2)]!),
    minimumMilliseconds: round(ordered[0]!),
    maximumMilliseconds: round(ordered.at(-1)!),
    samplesMilliseconds: values.map(round),
  };
}
