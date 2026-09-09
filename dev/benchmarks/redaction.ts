// Manual performance probe for bounded streaming credential redaction.

import { performance } from "node:perf_hooks";
import assert from "node:assert/strict";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { reloadAccounts } from "../../src/accounts.ts";
import {
  credentialRedactor,
  MAX_REDACTION_SECRETS,
} from "../../src/credential-safety.ts";
import { reload as reloadCredentials } from "../../src/credentials.ts";
import { reportBenchmark } from "./report.ts";
import { distribution } from "./samples.ts";

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
  const matched = `${Object.values(source).join("|")}|fixture-secret-unmatched`;
  const expected = `${Array(MAX_REDACTION_SECRETS).fill("[credential redacted]").join("|")}|fixture-secret-unmatched`;
  const cases = [
    { name: "miss", chunks: [output], expected: output },
    { name: "matching", chunks: [matched], expected },
    { name: "split-shared-prefix", chunks: Array.from(matched), expected },
  ].map(fixture => {
    sample(fixture.chunks, fixture.expected);
    const observations = Array.from({ length: ITERATIONS }, () => sample(fixture.chunks, fixture.expected));
    return { name: fixture.name, chunks: fixture.chunks.length,
      inputCharacters: fixture.chunks.reduce((sum, chunk) => sum + chunk.length, 0),
      setup: distribution(observations.map(value => value.setup)),
      streaming: distribution(observations.map(value => value.streaming)) };
  });

  reportBenchmark("streaming-redaction", {
    secrets: MAX_REDACTION_SECRETS,
    outputCharacters: OUTPUT_CHARS,
    iterations: ITERATIONS,
    cases,
  });
} finally {
  if (before === undefined) delete process.env["JECODE_HOME"];
  else process.env["JECODE_HOME"] = before;
  reloadAccounts();
  reloadCredentials();
}

function sample(chunks: readonly string[], expected: string) {
  const started = performance.now();
  const redact = credentialRedactor(source);
  const setup = performance.now() - started;
  const streamingStart = performance.now();
  const actual = chunks.map(chunk => redact.write(chunk)).join("") + redact.end();
  const streaming = performance.now() - streamingStart;
  assert.equal(actual, expected, "streaming must redact complete and fragmented credentials exactly");
  return { setup, streaming };
}
