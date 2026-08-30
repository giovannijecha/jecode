// Keep the committed, dependency-free runtime exactly aligned with src/.

import { spawnSync } from "node:child_process";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";

const root = await mkdtemp(path.join(tmpdir(), "jecode-release-check-"));
const candidate = path.join(root, "dist");

try {
  const tsc = path.join(process.cwd(), "node_modules", "typescript", "bin", "tsc");
  const result = spawnSync(process.execPath, [
    tsc,
    "--project",
    "tsconfig.release.json",
    "--outDir",
    candidate,
  ], {
    cwd: process.cwd(),
    encoding: "utf8",
    windowsHide: true,
  });
  if (result.error !== undefined) throw result.error;
  if (result.status !== 0) {
    throw new Error(`release compilation failed (${result.status})\n${result.stdout}\n${result.stderr}`);
  }

  const expected = await files(candidate);
  const committed = await files(path.join(process.cwd(), "dist"));
  if (expected.join("\n") !== committed.join("\n")) {
    throw new Error("dist file list is stale; run npm run build:release");
  }
  for (const file of expected) {
    const [left, right] = await Promise.all([
      readFile(path.join(candidate, file)),
      readFile(path.join("dist", file)),
    ]);
    if (!left.equals(right)) {
      throw new Error(`dist/${file} is stale; run npm run build:release`);
    }
  }
  process.stdout.write(`release tree: ${expected.length} compiled files are current\n`);
} finally {
  await rm(root, { recursive: true, force: true });
}

async function files(directory: string, prefix = ""): Promise<string[]> {
  const names: string[] = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const relative = path.join(prefix, entry.name);
    if (entry.isDirectory()) names.push(...await files(path.join(directory, entry.name), relative));
    else if (entry.isFile()) names.push(relative.replaceAll("\\", "/"));
  }
  return names.sort();
}
