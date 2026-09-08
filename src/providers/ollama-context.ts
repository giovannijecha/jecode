// Cloud model capacity, with bounded metadata and conservative safety headroom.

import type { ModelContextWindow } from "../types.ts";
import { createHash } from "node:crypto";
import { postJson } from "./http.ts";
import { OLLAMA_CLOUD_HOST } from "./ollama-endpoint.ts";

const CACHE_MS = 15 * 60_000;
const MISSING_CACHE_MS = 60_000;
const METADATA_TIMEOUT_MS = 2_000;
const MAX_CACHED_MODELS = 128;
const capacities = new Map<string, { value: ModelContextWindow | undefined; expiresAt: number }>();

export async function ollamaContextWindow(
  model: string,
  headers: Record<string, string>,
  signal?: AbortSignal,
  onStatus?: (status: string) => void,
): Promise<ModelContextWindow | undefined> {
  signal?.throwIfAborted();
  // A rejected probe under an old key must not mask metadata after reconnect.
  const cacheKey = createHash("sha256").update(JSON.stringify([model, headers["authorization"]])).digest("hex");
  const cached = capacities.get(cacheKey);
  if (cached !== undefined && cached.expiresAt > Date.now()) return cached.value;
  capacities.delete(cacheKey);
  const timeout = AbortSignal.timeout(METADATA_TIMEOUT_MS);
  const probeSignal = signal === undefined ? timeout : AbortSignal.any([signal, timeout]);
  let value: ModelContextWindow | undefined;
  try {
    const details = await postJson(
      `${OLLAMA_CLOUD_HOST}/api/show`, headers, { model }, probeSignal, onStatus,
    );
    const tokens = modelCapacity(details);
    if (tokens !== undefined) value = Object.freeze({ tokens: Math.floor(tokens * 95 / 100) });
  } catch (error) {
    if (signal?.aborted) throw error;
    // Missing metadata leaves budgeting to the controller's safe fallback.
  }
  signal?.throwIfAborted();
  if (capacities.size >= MAX_CACHED_MODELS) capacities.delete(capacities.keys().next().value!);
  capacities.set(cacheKey, { value, expiresAt: Date.now() + (value === undefined ? MISSING_CACHE_MS : CACHE_MS) });
  return value;
}

function modelCapacity(value: unknown): number | undefined {
  if (!record(value) || !record(value["model_info"])) return undefined;
  const counts = Object.entries(value["model_info"])
    .filter(([name, count]) => name.endsWith(".context_length") && validTokenCount(count))
    .map(([, count]) => count as number);
  return counts.length === 0 ? undefined : Math.max(...counts);
}

function validTokenCount(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) &&
    value >= 4_096 && value <= 10_000_000;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
