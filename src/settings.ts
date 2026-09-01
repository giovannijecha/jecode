// Persistent, non-secret defaults for interactive and batch sessions.

import { readFileSync } from "node:fs";
import { chmod, mkdir } from "node:fs/promises";
import * as path from "node:path";
import { atomicWrite } from "./atomic.ts";
import { MAX_COMPACTION_PERCENT, MIN_COMPACTION_PERCENT } from "./context/policy.ts";
import { EFFORTS } from "./effort.ts";
import { providerNames } from "./providers/index.ts";
import { parseOllamaEndpoint } from "./providers/ollama-endpoint.ts";
import { withStoreLock } from "./store-lock.ts";
import { userDataLabel, userDataPath } from "./user-data.ts";

export { EFFORTS } from "./effort.ts";

export type SavedSettings = {
  provider?: string;
  models?: Record<string, string>;
  ollamaHost?: string;
  effort?: string;
  reducedMotion?: boolean;
  maxTokens?: number;
  maxSteps?: number;
  compactionPercent?: number;
};

let saved: SavedSettings | undefined;

export function readSettings(): SavedSettings {
  if (saved === undefined) saved = readStore();
  return copy(saved);
}

export async function updateSettings(patch: Partial<SavedSettings>): Promise<string> {
  const file = settingsPath();
  const directory = path.dirname(file);
  await mkdir(directory, { recursive: true, mode: 0o700 });
  if (process.platform !== "win32") await chmod(directory, 0o700);
  return withStoreLock(file, async () => {
    const next = normalize({ ...readStore(file), ...patch });
    await atomicWrite(file, `${JSON.stringify(next, null, 2)}\n`, { mode: 0o600 });
    saved = next;
    return file;
  });
}

export function settingsPath(): string {
  return userDataPath("settings.json");
}

export function settingsLabel(): string {
  return userDataLabel("settings.json");
}

/** Forget the cached read so tests and explicit reloads see the disk again. */
export function reloadSettings(): void {
  saved = undefined;
}

function readStore(file = settingsPath()): SavedSettings {
  try {
    return normalize(JSON.parse(readFileSync(file, "utf8")) as unknown);
  } catch {
    // Missing, unreadable, and malformed stores all fall back safely. A bad
    // preference must never prevent the agent from starting.
    return {};
  }
}

function normalize(value: unknown): SavedSettings {
  if (!record(value)) return {};
  const providers = providerNames();
  const provider = member(value["provider"], providers);
  const models = modelsOf(value["models"], providers);
  const ollamaHost = endpoint(value["ollamaHost"]);
  const effort = member(value["effort"], EFFORTS);
  const reducedMotion = typeof value["reducedMotion"] === "boolean" ? value["reducedMotion"] : undefined;
  const maxTokens = positiveInteger(value["maxTokens"]);
  const maxSteps = positiveInteger(value["maxSteps"]);
  const compactionPercent = percentage(value["compactionPercent"]);

  return {
    ...(provider === undefined ? {} : { provider }),
    ...(models === undefined ? {} : { models }),
    ...(ollamaHost === undefined ? {} : { ollamaHost }),
    ...(effort === undefined ? {} : { effort }),
    ...(reducedMotion === undefined ? {} : { reducedMotion }),
    ...(maxTokens === undefined ? {} : { maxTokens }),
    ...(maxSteps === undefined ? {} : { maxSteps }),
    ...(compactionPercent === undefined ? {} : { compactionPercent }),
  };
}

function modelsOf(value: unknown, providers: readonly string[]): Record<string, string> | undefined {
  if (!record(value)) return undefined;
  const models = Object.fromEntries(
    Object.entries(value).filter(
      (entry): entry is [string, string] =>
        providers.includes(entry[0]) && typeof entry[1] === "string" && entry[1].trim() !== "",
    ),
  );
  return Object.keys(models).length === 0 ? undefined : models;
}

function member(value: unknown, values: readonly string[]): string | undefined {
  return typeof value === "string" && values.includes(value) ? value : undefined;
}

function endpoint(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  try {
    return parseOllamaEndpoint(value).baseUrl;
  } catch {
    return undefined;
  }
}

function positiveInteger(value: unknown): number | undefined {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0 ? value : undefined;
}

function percentage(value: unknown): number | undefined {
  return typeof value === "number" && Number.isSafeInteger(value) &&
      value >= MIN_COMPACTION_PERCENT && value <= MAX_COMPACTION_PERCENT
    ? value
    : undefined;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function copy(value: SavedSettings): SavedSettings {
  return {
    ...value,
    ...(value.models === undefined ? {} : { models: { ...value.models } }),
  };
}
