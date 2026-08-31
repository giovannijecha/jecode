// Information requests that finish before configuration or terminal takeover.

import { readFile } from "node:fs/promises";
import * as path from "node:path";

const HELP = `jecode — an owned coding agent for the terminal

Usage:
  jecode [options]

Options:
  --root <path>          workspace root (default: current directory)
  --provider <id>        anthropic, openai, openai-codex, or ollama
  --model <id>           model for the selected provider
  --ollama-host <url>    Ollama Cloud, local, or custom endpoint
  --effort <level>       low, medium, high, xhigh, or max
  --max-tokens <number>  output-token ceiling
  --max-steps <number>   tool-loop ceiling
  --reduced-motion       disable animated terminal states
  --auto-approve         allow dangerous tools for this process
  -h, --help             show this help
  -v, --version          show the installed version

Inside Jecode, type / to discover interactive commands.`;

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
