// Build the dependency-free runtime from a clean, repository-local target.

import { spawnSync } from "node:child_process";
import { access, readdir, rm } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import * as path from "node:path";

const projectRoot = path.resolve(fileURLToPath(new URL("..", import.meta.url)));
const target = path.join(projectRoot, "dist");

if (path.dirname(target) !== projectRoot || path.basename(target) !== "dist") {
  throw new Error(`refusing to replace unexpected release target: ${target}`);
}

await rm(target, { recursive: true, force: true });

const tsc = path.join(projectRoot, "node_modules", "typescript", "bin", "tsc");
const result = spawnSync(process.execPath, [tsc, "--project", "tsconfig.release.json"], {
  cwd: projectRoot,
  encoding: "utf8",
  windowsHide: true,
});
if (result.error !== undefined) throw result.error;
if (result.status !== 0) {
  throw new Error(`release compilation failed (${result.status})\n${result.stdout}\n${result.stderr}`);
}

await access(path.join(target, "main.js"));
const compiled = await files(target);
const unexpected = compiled.filter((file) => !file.endsWith(".js"));
if (unexpected.length > 0) {
  throw new Error(`unexpected release files: ${unexpected.join(", ")}`);
}

if (!process.argv.includes("--quiet")) {
  process.stdout.write(`release tree: ${compiled.length} compiled files generated\n`);
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
