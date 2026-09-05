// Information requests that finish before configuration or terminal takeover.

import { readFile } from "node:fs/promises";
import * as path from "node:path";

const HELP = `jecode — an owned coding agent for the terminal

Usage:
  jecode [options]
  jecode -c [options]
  jecode resume [--last] [options]

Options:
  --root <path>          workspace root (default: current directory)
  --reduced-motion       disable decorative motion and keep the cursor steady
  --ephemeral            do not save this conversation
  -c                     resume the newest session in this workspace
  --last                 with resume, skip the session picker (same as -c)
  -h, --help             show this help
  -v, --version          show the installed version

Jecode requires an interactive terminal on stdin and stdout.
Inside Jecode, use /models, /settings, and /permissions to configure your session.
Type / to discover interactive commands.`;

export async function showCliInfo(
  args: readonly string[],
  applicationRoot: string,
  write: (text: string) => void,
): Promise<boolean> {
  if (args.includes("--help") || args.includes("-h")) {
    write(`${HELP}\n`);
    return true;
  }
  if (args.includes("--version") || args.includes("-v")) {
    write(`${await packageVersion(applicationRoot)}\n`);
    return true;
  }
  return false;
}

async function packageVersion(applicationRoot: string): Promise<string> {
  const manifest = JSON.parse(
    await readFile(path.join(applicationRoot, "package.json"), "utf8"),
  ) as { version?: unknown };
  if (typeof manifest.version !== "string" || manifest.version === "") {
    throw new Error("package version is missing");
  }
  return manifest.version;
}
