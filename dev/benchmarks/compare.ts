// Read bounded saved collections; never download or execute an artifact.

import { open, writeFile } from "node:fs/promises";
import { resolve, join } from "node:path";
import { collectionComplete, validateCollection } from "./collection.ts";
import { compare, markdown } from "./comparison.ts";

const [basePath, currentPath, outputDirectory, ...extra] = process.argv.slice(2);
if (basePath === undefined || currentPath === undefined || outputDirectory === undefined || extra.length !== 0) {
  throw new Error("usage: compare.ts <baseline.json> <current.json> <existing-output-directory>");
}
const baseline = await read(basePath);
const current = await read(currentPath);
const comparison = compare(baseline, current);
await writeFile(join(resolve(outputDirectory), "comparison.json"), `${JSON.stringify(comparison, null, 2)}\n`);
await writeFile(join(resolve(outputDirectory), "SUMMARY.md"), markdown(comparison));
// A complete negative measurement is evidence, not a new CI performance gate.
// Crashes, interruption, and unusable output still fail the acquisition workflow.
if (!collectionComplete(baseline) || !collectionComplete(current)) process.exitCode = 1;

async function read(file: string) {
  const handle = await open(file, "r");
  try {
    const limit = 8 * 1_048_576;
    if (!(await handle.stat()).isFile()) throw new Error("benchmark collection must be a regular file");
    const buffer = Buffer.alloc(limit + 1);
    let size = 0;
    while (size < buffer.length) {
      const { bytesRead } = await handle.read(buffer, size, buffer.length - size, null);
      if (bytesRead === 0) break;
      size += bytesRead;
    }
    if (size > limit) throw new Error("benchmark collection exceeds 8 MiB");
    return validateCollection(JSON.parse(buffer.subarray(0, size).toString("utf8")));
  } finally { await handle.close(); }
}
