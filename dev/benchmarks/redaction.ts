// Manual performance probe for bounded streaming credential redaction.

import { performance } from "node:perf_hooks";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { reloadAccounts } from "../../src/accounts.ts";
import {
  credentialRedactor,
  MAX_REDACTION_SECRETS,
} from "../../src/credential-safety.ts";
import { reload as reloadCredentials } from "../../src/credentials.ts";
import { reportBenchmark, round } from "./report.ts";

const OUTPUT_CHARS = 30_000;
const ITERATIONS = 5;
const source = Object.fromEntries(Array.from(
  { length: MAX_REDACTION_SECRETS },
  (_, index) => [`TOKEN_${index}`, `fixture-secret-${index}`],
));
const output = "x".repeat(OUTPUT_CHARS);
const before = process.env["JECODE_HOME"];

// The path remains absent: the probe must not read or write the user's stores.
process.env["JECODE_HOME"] = path.join(
  tmpdir(),
  `jecode-redaction-bench-${process.pid}-${Date.now()}`,
);
reloadAccounts();
reloadCredentials();

try {
  sample();
  const timings = Array.from({ length: ITERATIONS }, sample)
    .sort((left, right) => left - right);
  const median = timings[Math.floor(timings.length / 2)] as number;

  reportBenchmark("streaming-redaction", {
    secrets: MAX_REDACTION_SECRETS,
    outputCharacters: OUTPUT_CHARS,
    iterations: ITERATIONS,
    medianMilliseconds: round(median),
  });
} finally {
  if (before === undefined) delete process.env["JECODE_HOME"];
  else process.env["JECODE_HOME"] = before;
  reloadAccounts();
  reloadCredentials();
}

function sample(): number {
  const started = performance.now();
  const redact = credentialRedactor(source);
  const actual = `${redact.write(output)}${redact.end()}`;
  const elapsed = performance.now() - started;
  if (actual !== output) throw new Error("benchmark output changed unexpectedly");
  return elapsed;
}
