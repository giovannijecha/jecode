// Saved runtime settings with a small set of process-only launch options.

import * as path from "node:path";
import {
  DEFAULT_COMPACTION_PERCENT,
  MAX_COMPACTION_PERCENT,
  MIN_COMPACTION_PERCENT,
} from "./context/policy.ts";
import { EFFORTS, readSettings } from "./settings.ts";
import type { SavedSettings } from "./settings.ts";
import { isLegacyOllamaCloudHost } from "./providers/ollama-endpoint.ts";

export type Config = {
  providerId: string;
  model: string;
  reducedMotion: boolean;
  effort: string;
  maxTokens: number;
  compactionPercent: number;
  root: string;
  ephemeral: boolean;
};

const VALUE_FLAGS = ["root"] as const;
const BOOLEAN_FLAGS = [
  "reduced-motion",
  "ephemeral",
] as const;
const FLAGS: readonly string[] = [...VALUE_FLAGS, ...BOOLEAN_FLAGS];
const RETIRED_FLAGS: Readonly<Record<string, string>> = {
  provider: "/models",
  model: "/models",
  effort: "/effort",
  "max-tokens": "/settings",
  "compaction-percent": "/settings",
  "auto-approve": "/permissions",
  "max-steps": "",
};

export function loadConfig(argv: string[], saved: SavedSettings = readSettings()): Config {
  const flags = parseFlags(argv);
  for (const [name, command] of Object.entries(RETIRED_FLAGS)) {
    const variable = `JECODE_${name.toUpperCase().replaceAll("-", "_")}`;
    if (process.env[variable] !== undefined && process.env[variable] !== "") {
      throw new Error(`${variable} is no longer supported; remove it${replacement(command)}`);
    }
  }
  assertRetiredOllamaSettings(saved);

  const providerId = saved.provider ?? "anthropic";
  const effort = saved.effort ?? "high";
  if (!(EFFORTS as readonly string[]).includes(effort)) {
    throw new Error(`unknown effort "${effort}" (expected one of: ${EFFORTS.join(", ")})`);
  }
  return {
    providerId,
    model: saved.models?.[providerId] ?? "",
    reducedMotion: bool(
      flags["reduced-motion"],
      process.env.JECODE_REDUCED_MOTION,
      saved.reducedMotion ?? false,
    ),
    effort,
    // This is a ceiling, not a target. Request budgeting may lower it to fit
    // the selected model, while provider rate and billing limits still apply.
    maxTokens: toInt(saved.maxTokens ?? 64000, "max output tokens"),
    compactionPercent: toPercent(saved.compactionPercent ?? DEFAULT_COMPACTION_PERCENT),
    root: path.resolve(flags.root ?? process.cwd()),
    ephemeral: bool(flags.ephemeral, process.env.JECODE_EPHEMERAL, false),
  };
}

function assertRetiredOllamaSettings(saved: SavedSettings): void {
  const host = process.env.OLLAMA_HOST;
  if (host !== undefined && host !== "" && !isLegacyOllamaCloudHost(host)) {
    throw new Error("OLLAMA_HOST no longer selects an endpoint; remove it to use Ollama API at https://ollama.com");
  }
  if (saved.ollamaHost !== undefined && !isLegacyOllamaCloudHost(saved.ollamaHost)) {
    throw new Error("settings.json has a retired Ollama endpoint; remove ollamaHost to use Ollama API at https://ollama.com");
  }
}

function bool(flag: string | undefined, env: string | undefined, fallback: boolean): boolean {
  if (flag !== undefined) return flag === "true" || flag === "1";
  if (env !== undefined && env !== "") return env === "true" || env === "1";
  return fallback;
}

function replacement(command: string): string {
  return command === "" ? "; interactive turns have no request limit" : ` and use ${command} in the TUI`;
}

function toInt(value: number, name: string): number {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new Error(`${name} in /settings must be a positive safe integer`);
  }
  return value;
}

function toPercent(percent: number): number {
  if (
    !Number.isSafeInteger(percent) ||
    percent < MIN_COMPACTION_PERCENT ||
    percent > MAX_COMPACTION_PERCENT
  ) {
    throw new Error(
      `context compaction in /settings must be an integer from ${MIN_COMPACTION_PERCENT} to ${MAX_COMPACTION_PERCENT}`,
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
    if (name === "ollama-host") {
      throw new Error("--ollama-host is no longer supported; remove it to use Ollama API at https://ollama.com");
    }
    if (Object.hasOwn(RETIRED_FLAGS, name)) {
      throw new Error(`--${name} is no longer supported; remove it${replacement(RETIRED_FLAGS[name] as string)}`);
    }
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
