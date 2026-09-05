// Keep generated release output out of the canonical Git tree.

import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import * as path from "node:path";

const projectRoot = path.resolve(fileURLToPath(new URL("..", import.meta.url)));
const tracked = runGit(["ls-files", "-z", "--", "dist"]);
const trackedFiles = tracked.stdout.split("\0").filter((file) => file !== "");

if (trackedFiles.length > 0) {
  throw new Error(`generated release files must stay untracked (${trackedFiles.length} found)`);
}

const ignored = runGit([
  "check-ignore",
  "--quiet",
  "--no-index",
  "--",
  "dist/.jecode-ignore-probe",
], [0, 1]);

if (ignored.status !== 0) {
  throw new Error("dist/ must stay ignored by Git");
}

process.stdout.write("source tree: dist is ignored and untracked\n");

function runGit(args: string[], allowed = [0]): { status: number; stdout: string } {
  const result = spawnSync("git", args, {
    cwd: projectRoot,
    encoding: "utf8",
    windowsHide: true,
  });
  if (result.error !== undefined) throw result.error;
  if (result.status === null || !allowed.includes(result.status)) {
    throw new Error(`git ${args[0]} failed (${result.status ?? "no status"})\n${result.stderr}`);
  }
  return { status: result.status, stdout: result.stdout };
}
