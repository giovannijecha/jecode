// Persistent, non-secret defaults for interactive sessions.

import * as path from "node:path";
import { atomicWrite } from "./atomic.ts";
import {
  assertDirectoryAnchor,
  captureDirectDirectorySync,
  preparePrivateDirectory,
} from "./directory-anchor.ts";
import type { DirectoryAnchor } from "./directory-anchor.ts";
import { MAX_COMPACTION_PERCENT, MIN_COMPACTION_PERCENT } from "./context/policy.ts";
import { EFFORTS } from "./effort.ts";
import { providerNames } from "./providers/index.ts";
import { isLegacyOllamaCloudHost, OLLAMA_CLOUD_HOST } from "./providers/ollama-endpoint.ts";
import { withStoreLock } from "./store-lock.ts";
import { userDataLabel, userDataPath } from "./user-data.ts";
import {
  assertStoreText,
  readBoundedJsonForMutationSync,
  readBoundedJsonSync,
  USER_STORE_LIMITS,
} from "./user-store.ts";

export { EFFORTS } from "./effort.ts";

export type SavedSettings = {
  provider?: string;
  models?: Record<string, string>;
  /** Retired input retained only to prevent silent endpoint migration. */
  ollamaHost?: string;
  effort?: string;
  reducedMotion?: boolean;
  maxTokens?: number;
  compactionPercent?: number;
};

let saved: SavedSettings | undefined;

export function readSettings(): SavedSettings {
  if (saved === undefined) saved = readStore();
  return copy(saved);
}

export async function updateSettings(patch: Partial<SavedSettings>): Promise<string> {
  assertRetiredOllamaHost(patch);
  const file = settingsPath();
  const directory = path.dirname(file);
  const anchor = await preparePrivateDirectory(directory, "settings store directory");
  const anchoredFile = path.join(anchor.path, path.basename(file));
  return withStoreLock(anchoredFile, async () => {
    const next = normalize({ ...readStoreForMutation(anchoredFile, anchor), ...patch });
    delete next.ollamaHost;
    const text = `${JSON.stringify(next, null, 2)}\n`;
    assertStoreText(text, USER_STORE_LIMITS.settingsBytes);
    await atomicWrite(anchoredFile, text, {
      mode: 0o600,
      validate: async () => assertDirectoryAnchor(anchor),
    });
    saved = next;
    return file;
  }, undefined, async () => assertDirectoryAnchor(anchor));
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
    const directory = captureDirectDirectorySync(path.dirname(file), "settings store directory");
    const anchoredFile = path.join(directory.path, path.basename(file));
    return normalize(readBoundedJsonSync(
      anchoredFile,
      USER_STORE_LIMITS.settingsBytes,
      directory,
    ));
  } catch {
    // Missing, unreadable, and malformed stores fall back safely. Parsed
    // retired endpoint markers survive normalization for startup validation.
    return {};
  }
}

function readStoreForMutation(file: string, directory: DirectoryAnchor): SavedSettings {
  const value = readBoundedJsonForMutationSync(
    file,
    USER_STORE_LIMITS.settingsBytes,
    "settings store",
    directory,
  );
  if (value === undefined) return {};
  if (!record(value)) throw new Error("settings store has an unsupported structure");
  assertMutableSettings(value);
  return normalize(value);
}

function assertMutableSettings(value: Record<string, unknown>): void {
  const current = new Set([
    "provider",
    "models",
    "ollamaHost",
    "effort",
    "reducedMotion",
    "maxTokens",
    "compactionPercent",
  ]);
  const retired = new Set(["theme", "palette", "maxSteps"]);
  if (Object.keys(value).some((key) => !current.has(key) && !retired.has(key))) {
    throw new Error("settings store has an unsupported structure");
  }
  const providers = providerNames();
  if ("provider" in value && member(value["provider"], providers) === undefined) {
    throw new Error("settings store has an invalid provider");
  }
  if ("models" in value) {
    const models = value["models"];
    if (
      !record(models) || Object.keys(models).some((provider) => !providers.includes(provider)) ||
      Object.values(models).some((model) => !boundedNonempty(model, USER_STORE_LIMITS.model))
    ) throw new Error("settings store has invalid models");
  }
  assertRetiredOllamaHost(value);
  if ("effort" in value && member(value["effort"], EFFORTS) === undefined) {
    throw new Error("settings store has an invalid reasoning effort");
  }
  if ("reducedMotion" in value && typeof value["reducedMotion"] !== "boolean") {
    throw new Error("settings store has an invalid reduced-motion preference");
  }
  if ("maxTokens" in value && positiveInteger(value["maxTokens"]) === undefined) {
    throw new Error("settings store has an invalid token limit");
  }
  if (
    "compactionPercent" in value && percentage(value["compactionPercent"]) === undefined
  ) throw new Error("settings store has an invalid compaction threshold");
}

function normalize(value: unknown): SavedSettings {
  if (!record(value)) return {};
  const providers = providerNames();
  const provider = member(value["provider"], providers);
  const models = modelsOf(value["models"], providers);
  const ollamaHost = "ollamaHost" in value ? legacyOllamaHost(value["ollamaHost"]) : undefined;
  const effort = member(value["effort"], EFFORTS);
  const reducedMotion = typeof value["reducedMotion"] === "boolean" ? value["reducedMotion"] : undefined;
  const maxTokens = positiveInteger(value["maxTokens"]);
  const compactionPercent = percentage(value["compactionPercent"]);

  return {
    ...(provider === undefined ? {} : { provider }),
    ...(models === undefined ? {} : { models }),
    ...(ollamaHost === undefined ? {} : { ollamaHost }),
    ...(effort === undefined ? {} : { effort }),
    ...(reducedMotion === undefined ? {} : { reducedMotion }),
    ...(maxTokens === undefined ? {} : { maxTokens }),
    ...(compactionPercent === undefined ? {} : { compactionPercent }),
  };
}

function modelsOf(value: unknown, providers: readonly string[]): Record<string, string> | undefined {
  if (!record(value)) return undefined;
  const models = Object.fromEntries(
    Object.entries(value).filter(
      (entry): entry is [string, string] =>
        providers.includes(entry[0]) && boundedNonempty(entry[1], USER_STORE_LIMITS.model),
    ),
  );
  return Object.keys(models).length === 0 ? undefined : models;
}

function member(value: unknown, values: readonly string[]): string | undefined {
  return typeof value === "string" && values.includes(value) ? value : undefined;
}

function legacyOllamaHost(value: unknown): string {
  if (isLegacyOllamaCloudHost(value)) return OLLAMA_CLOUD_HOST;
  // Even an invalid retired value must not disappear and enable cloud use.
  return boundedNonempty(value, USER_STORE_LIMITS.endpoint) ? value : "unsupported legacy Ollama endpoint";
}

function assertRetiredOllamaHost(value: object): void {
  if ("ollamaHost" in value && !isLegacyOllamaCloudHost(value.ollamaHost)) {
    throw new Error("settings store has a retired Ollama endpoint; remove ollamaHost from settings.json to use Ollama API");
  }
}

function boundedNonempty(value: unknown, max: number): value is string {
  return typeof value === "string" && value.length <= max && value.trim() !== "";
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
