// Runtime configuration: flags, environment, saved defaults, built-ins.

import * as path from "node:path";
import { EFFORTS, readSettings } from "./settings.ts";
import type { SavedSettings } from "./settings.ts";
import { parseOllamaEndpoint } from "./providers/ollama-endpoint.ts";

export type Config = {
  providerId: string;
  model: string;
  ollamaHost?: string;
  reducedMotion: boolean;
  effort: string;
  maxTokens: number;
  maxSteps: number;
  root: string;
  autoApprove: boolean;
};

const FLAGS = [
  "provider",
  "model",
  "ollama-host",
  "reduced-motion",
  "effort",
  "max-tokens",
  "max-steps",
  "root",
  "auto-approve",
];

export function loadConfig(argv: string[], saved: SavedSettings = readSettings()): Config {
  const flags = parseFlags(argv);

  // A flag nobody declared is a typo. Swallowed in silence it becomes a
  // setting the user believes is on, and the run that ignores it looks like
  // the feature is broken rather than misspelled.
  for (const name of Object.keys(flags)) {
    if (!FLAGS.includes(name)) {
      throw new Error(`unknown flag --${name} (known: ${FLAGS.map((f) => `--${f}`).join(", ")})`);
    }
  }

  const providerId = pick(flags.provider, process.env.JECODE_PROVIDER, saved.provider ?? "anthropic");
  const ollamaHost = optional(flags["ollama-host"], process.env.OLLAMA_HOST, saved.ollamaHost);
  const effort = pick(flags.effort, process.env.JECODE_EFFORT, saved.effort ?? "high");
  if (!(EFFORTS as readonly string[]).includes(effort)) {
    throw new Error(`unknown effort "${effort}" (expected one of: ${EFFORTS.join(", ")})`);
  }

  return {
    providerId,
    model: pick(flags.model, process.env.JECODE_MODEL, saved.models?.[providerId] ?? ""),
    ...(ollamaHost === undefined
      ? {}
      : { ollamaHost: parseOllamaEndpoint(ollamaHost).baseUrl }),
    reducedMotion: bool(
      flags["reduced-motion"],
      process.env.JECODE_REDUCED_MOTION,
      saved.reducedMotion ?? false,
    ),
    effort,
    // Every request streams, so a large ceiling costs nothing in timeout risk.
    maxTokens: toInt(
      pick(flags["max-tokens"], process.env.JECODE_MAX_TOKENS, String(saved.maxTokens ?? 64000)),
      "max-tokens",
    ),
    maxSteps: toInt(
      pick(flags["max-steps"], process.env.JECODE_MAX_STEPS, String(saved.maxSteps ?? 40)),
      "max-steps",
    ),
    root: path.resolve(pick(flags.root, undefined, process.cwd())),
    autoApprove: flags["auto-approve"] === "true" || process.env.JECODE_AUTO_APPROVE === "1",
  };
}

function bool(flag: string | undefined, env: string | undefined, fallback: boolean): boolean {
  if (flag !== undefined) return flag === "true" || flag === "1";
  if (env !== undefined && env !== "") return env === "true" || env === "1";
  return fallback;
}

function pick(flag: string | undefined, env: string | undefined, fallback: string): string {
  if (flag !== undefined && flag !== "") return flag;
  if (env !== undefined && env !== "") return env;
  return fallback;
}

function optional(
  flag: string | undefined,
  env: string | undefined,
  fallback: string | undefined,
): string | undefined {
  if (flag !== undefined && flag !== "") return flag;
  if (env !== undefined && env !== "") return env;
  return fallback;
}

function toInt(value: string, name: string): number {
  const n = Number(value);
  if (!Number.isInteger(n) || n <= 0) throw new Error(`--${name} must be a positive integer`);
  return n;
}

// Accepts --key value, --key=value, and bare --flag (which reads as "true").
function parseFlags(argv: string[]): Record<string, string> {
  const flags: Record<string, string> = {};

  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === undefined || !arg.startsWith("--")) continue;

    const body = arg.slice(2);
    const eq = body.indexOf("=");
    if (eq !== -1) {
      flags[body.slice(0, eq)] = body.slice(eq + 1);
      continue;
    }

    const next = argv[i + 1];
    if (next !== undefined && !next.startsWith("--")) {
      flags[body] = next;
      i++;
    } else {
      flags[body] = "true";
    }
  }

  return flags;
}
