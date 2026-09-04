// Manual performance probe for the bounded workspace text search.

import { performance } from "node:perf_hooks";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { searchText } from "../src/tools/search.ts";

const FILES = 600;
const FILE_BYTES = 4_096;
const ITERATIONS = 5;

const root = await fs.mkdtemp(path.join(os.tmpdir(), "jecode-search-bench-"));

try {
  const body = "ordinary source text\n".repeat(Math.ceil(FILE_BYTES / 21)).slice(0, FILE_BYTES);
  await Promise.all(Array.from({ length: FILES }, (_, index) => (
    fs.writeFile(path.join(root, `source-${String(index).padStart(4, "0")}.txt`), body, "utf8")
  )));

  const bounded = await sample();

  process.stdout.write([
    `files: ${FILES.toLocaleString()} (${formatBytes(FILES * FILE_BYTES)})`,
    `iterations: ${ITERATIONS}`,
    `bounded search: ${formatMilliseconds(bounded)} median`,
  ].join("\n") + "\n");
} finally {
  await fs.rm(root, { recursive: true, force: true });
}

async function sample(): Promise<number> {
  const timings: number[] = [];
  for (let iteration = 0; iteration < ITERATIONS + 1; iteration++) {
    const started = performance.now();
    const result = await searchText.run({ query: "absent-benchmark-needle" }, { root });
    const elapsed = performance.now() - started;
    if (result.output !== "[no matches]") throw new Error("benchmark fixture unexpectedly matched");
    if (iteration > 0) timings.push(elapsed);
  }
  timings.sort((left, right) => left - right);
  return timings[Math.floor(timings.length / 2)] as number;
}

function formatMilliseconds(value: number): string {
  return `${value.toFixed(2)} ms`;
}

function formatBytes(value: number): string {
  return `${(value / (1024 * 1024)).toFixed(2)} MiB`;
}
