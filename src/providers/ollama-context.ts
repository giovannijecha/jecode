// Cloud model capacity, with bounded metadata and conservative safety headroom.

import type { ModelContextWindow } from "../types.ts";
import { postJson } from "./http.ts";
import { OLLAMA_CLOUD_HOST } from "./ollama-endpoint.ts";

const CACHE_MS = 30_000;
const capacities = new Map<string, { value: ModelContextWindow; expiresAt: number }>();

export async function ollamaContextWindow(
  model: string,
  headers: Record<string, string>,
  signal?: AbortSignal,
  onStatus?: (status: string) => void,
): Promise<ModelContextWindow | undefined> {
  signal?.throwIfAborted();
  const cached = capacities.get(model);
  if (cached !== undefined && cached.expiresAt > Date.now()) return cached.value;
  capacities.delete(model);
  try {
    const details = await postJson(
      `${OLLAMA_CLOUD_HOST}/api/show`, headers, { model }, signal, onStatus,
    );
    const tokens = modelCapacity(details);
    if (tokens === undefined) return undefined;
    const value = Object.freeze({ tokens: Math.floor(tokens * 95 / 100) });
    capacities.set(model, { value, expiresAt: Date.now() + CACHE_MS });
    return value;
  } catch (error) {
    if (signal?.aborted) throw error;
    // Missing metadata leaves budgeting to the controller's safe fallback.
    return undefined;
  }
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
