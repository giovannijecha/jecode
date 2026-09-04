// Runtime configuration: flags, environment, saved defaults, built-ins.

import * as path from "node:path";
import {
  DEFAULT_COMPACTION_PERCENT,
  MAX_COMPACTION_PERCENT,
  MIN_COMPACTION_PERCENT,
} from "./context/policy.ts";
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
  /** Optional process-only model-request budget for deterministic automation. */
  maxModelRequests?: number;
  compactionPercent: number;
  root: string;
  autoApprove: boolean;
  ephemeral: boolean;
};

const VALUE_FLAGS = [
  "provider",
  "model",
  "ollama-host",
  "effort",
  "max-tokens",
  "max-steps",
  "compaction-percent",
  "root",
] as const;
const BOOLEAN_FLAGS = [
  "reduced-motion",
  "auto-approve",
  "ephemeral",
] as const;
const FLAGS: readonly string[] = [...VALUE_FLAGS, ...BOOLEAN_FLAGS];

export function loadConfig(argv: string[], saved: SavedSettings = readSettings()): Config {
  const flags = parseFlags(argv);

  const providerId = pick(flags.provider, process.env.JECODE_PROVIDER, saved.provider ?? "anthropic");
  const ollamaHost = optional(flags["ollama-host"], process.env.OLLAMA_HOST, saved.ollamaHost);
  const effort = pick(flags.effort, process.env.JECODE_EFFORT, saved.effort ?? "high");
  if (!(EFFORTS as readonly string[]).includes(effort)) {
    throw new Error(`unknown effort "${effort}" (expected one of: ${EFFORTS.join(", ")})`);
  }
  const maxModelRequests = optional(
    flags["max-steps"],
    process.env.JECODE_MAX_STEPS,
    undefined,
  );

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
    // This is a ceiling, not a target. Request budgeting may lower it to fit
    // the selected model, while provider rate and billing limits still apply.
    maxTokens: toInt(
      pick(flags["max-tokens"], process.env.JECODE_MAX_TOKENS, String(saved.maxTokens ?? 64000)),
      "max-tokens",
    ),
    ...(maxModelRequests === undefined
      ? {}
      : { maxModelRequests: toInt(maxModelRequests, "max-steps") }),
    compactionPercent: toPercent(
      pick(
        flags["compaction-percent"],
        process.env.JECODE_COMPACTION_PERCENT,
        String(saved.compactionPercent ?? DEFAULT_COMPACTION_PERCENT),
      ),
    ),
    root: path.resolve(pick(flags.root, undefined, process.cwd())),
    autoApprove: autoApproval(flags["auto-approve"], process.env.JECODE_AUTO_APPROVE),
    ephemeral: bool(flags.ephemeral, process.env.JECODE_EPHEMERAL, false),
  };
}

function bool(flag: string | undefined, env: string | undefined, fallback: boolean): boolean {
  if (flag !== undefined) return flag === "true" || flag === "1";
  if (env !== undefined && env !== "") return env === "true" || env === "1";
  return fallback;
}

function autoApproval(flag: string | undefined, env: string | undefined): boolean {
  if (flag !== undefined) return flag === "true" || flag === "1";
  return env === "1";
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
  if (!Number.isSafeInteger(n) || n <= 0) {
    throw new Error(`--${name} must be a positive safe integer`);
  }
  return n;
}

function toPercent(value: string): number {
  const percent = Number(value);
  if (
    !Number.isSafeInteger(percent) ||
    percent < MIN_COMPACTION_PERCENT ||
    percent > MAX_COMPACTION_PERCENT
  ) {
    throw new Error(
      `--compaction-percent must be an integer from ${MIN_COMPACTION_PERCENT} to ${MAX_COMPACTION_PERCENT}`,
    );
  }
  return percent;
}

// Value flags accept --key value and --key=value. Boolean flags are bare or
// take a real boolean value, so an accidental positional argument is never
// swallowed as configuration.
function parseFlags(argv: string[]): Record<string, string> {
  const flags: Record<string, string> = {};

  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === undefined) continue;
    if (!arg.startsWith("--")) throw new Error(`unexpected argument "${arg}"`);

    const body = arg.slice(2);
    const eq = body.indexOf("=");
    const name = eq === -1 ? body : body.slice(0, eq);
    const inline = eq === -1 ? undefined : body.slice(eq + 1);
    if (!FLAGS.includes(name)) {
      throw new Error(`unknown flag --${name} (known: ${FLAGS.map((flag) => `--${flag}`).join(", ")})`);
    }

    if ((BOOLEAN_FLAGS as readonly string[]).includes(name)) {
      if (inline === undefined) {
        const next = argv[i + 1];
        if (next !== undefined && ["true", "false", "1", "0"].includes(next)) {
          flags[name] = next;
          i++;
        } else {
          flags[name] = "true";
        }
      } else if (["true", "false", "1", "0"].includes(inline)) {
        flags[name] = inline;
      } else {
        throw new Error(`--${name} must be true or false`);
      }
      continue;
    }

    if (inline !== undefined) {
      if (inline === "") throw new Error(`--${name} requires a value`);
      flags[name] = inline;
      continue;
    }
    const next = argv[i + 1];
    if (next === undefined || next.startsWith("--")) {
      throw new Error(`--${name} requires a value`);
    }
    flags[name] = next;
    i++;
  }

  return flags;
}
