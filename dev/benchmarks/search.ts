// Manual performance probe for the bounded workspace text search.

import { performance } from "node:perf_hooks";
import assert from "node:assert/strict";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { searchText } from "../../src/tools/search.ts";
import { reportBenchmark } from "./report.ts";
import { distribution } from "./samples.ts";

const FILES = 600;
const FILE_BYTES = 4_096;
const ITERATIONS = 5;

const root = await fs.mkdtemp(path.join(os.tmpdir(), "jecode-search-bench-"));

try {
  const body = "ordinary source text\n".repeat(Math.ceil(FILE_BYTES / 21)).slice(0, FILE_BYTES);
  await Promise.all(Array.from({ length: FILES }, (_, index) => (
    fs.writeFile(path.join(root, `source-${String(index).padStart(4, "0")}.txt`), body, "utf8")
  )));

  const cases = [{ name: "miss", ...(await sample("absent-benchmark-needle", "[no matches]")) }];
  const nested = path.join(root, "nested", "source", "deep");
  await fs.mkdir(nested, { recursive: true });
  const needle = "present-benchmark-needle";
  await fs.writeFile(path.join(root, "early.txt"), `${needle}\n`);
  await fs.writeFile(path.join(nested, "late.txt"), `first line\n${needle}\n${needle}\n`);
  await fs.writeFile(path.join(root, "binary.dat"), `\0${needle}`);
  await fs.writeFile(path.join(root, "large.txt"), `${needle}${"x".repeat(1_000_001)}`);
  const matches = [`early.txt:1:${needle}`, `nested/source/deep/late.txt:2:${needle}`,
    `nested/source/deep/late.txt:3:${needle}`];
  cases.push({ name: "nested-matches", ...(await sample(needle, matches.join("\n"))) });
  cases.push({ name: "result-limit", ...(await sample(needle, matches.slice(0, 2).join("\n"), 2)) });
  await assert.rejects(searchText.run({ query: needle }, { root, signal: AbortSignal.abort() }),
    { name: "AbortError" });
  await assert.rejects(searchText.run({ query: needle, max_results: 0 }, { root }));

  reportBenchmark("workspace-search", {
    files: FILES,
    fileBytes: FILE_BYTES,
    inputBytes: FILES * FILE_BYTES,
    iterations: ITERATIONS,
    cases,
    correctness: { positiveMatches: 3, resultLimit: 2, skippedBinaryAndLarge: true, cancellation: true },
  });
} finally {
  await fs.rm(root, { recursive: true, force: true });
}

async function sample(query: string, expected: string, maxResults = 100) {
  const timings: number[] = [];
  for (let iteration = 0; iteration < ITERATIONS + 1; iteration++) {
    const started = performance.now();
    const result = await searchText.run({ query, max_results: maxResults }, { root });
    const elapsed = performance.now() - started;
    assert.equal(result.output.replaceAll("\\", "/"), expected, "search must return exactly the fixture matches");
    if (iteration > 0) timings.push(elapsed);
  }
  return distribution(timings);
}
