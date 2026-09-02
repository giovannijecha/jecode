// Ollama, spoken through its OpenAI-compatible Chat Completions endpoint.
//
// One provider covers both deployments. An explicit session endpoint wins;
// otherwise a configured key selects Ollama Cloud and no key selects the local
// daemon. There is no default model — the catalogue is whatever the host has
// pulled or the subscription grants — so the model has to be named with
// --model.

import type { Message, ModelContextWindow, Provider, SendRequest } from "../types.ts";
import { requireSupportedEffort } from "../effort.ts";
import { getJson, postJson, postSse } from "./http.ts";
import { listModels } from "./catalog.ts";
import { keyFor } from "../credentials.ts";
import { assembleOllama } from "./ollama-stream.ts";
import {
  OLLAMA_CLOUD_HOST,
  OLLAMA_LOCAL_HOST,
  ollamaConnectionKind,
  parseOllamaEndpoint,
} from "./ollama-endpoint.ts";
import type { OllamaEndpoint } from "./ollama-endpoint.ts";
import { fromWireReply, stopNotice, toWireMessages, toWireTool } from "./ollama-wire.ts";

const KEY = "OLLAMA_API_KEY";
// Ollama also accepts `none`; Jecode's product-wide reasoning floor is `low`.
const OLLAMA_EFFORTS = ["low", "medium", "high"] as const;
let configuredHost: string | undefined;
const CONTEXT_CACHE_MS = 30_000;
const contextByEndpoint = new Map<string, { value: ModelContextWindow; expiresAt: number }>();

export type OllamaConnection = OllamaEndpoint & {
  kind: "cloud" | "local" | "custom";
  inferred: boolean;
};

/** Set the endpoint selected for this process. Undefined restores key-aware inference. */
export function configureOllama(host: string | undefined): void {
  configuredHost = host === undefined ? undefined : parseOllamaEndpoint(host).baseUrl;
}

export function ollamaConnection(): OllamaConnection {
  const inferred = configuredHost === undefined;
  const endpoint = parseOllamaEndpoint(
    configuredHost ?? (apiKey() === undefined ? OLLAMA_LOCAL_HOST : OLLAMA_CLOUD_HOST),
  );
  return { ...endpoint, kind: ollamaConnectionKind(endpoint), inferred };
}

export const ollama: Provider = {
  id: "ollama",
  defaultModel: "",
  auth: { kind: "api-key", keyVar: KEY },

  // The only provider whose key is conditional: a daemon on this machine is
  // reached over loopback and asks for nothing, so demanding a key there
  // would be an invented requirement.
  blocked(): string | undefined {
    try {
      const at = endpoint();
      if (apiKey() !== undefined || at.loopback) return undefined;
      return `${KEY} is not set (required by ${at.baseUrl})`;
    } catch (error) {
      return (error as Error).message;
    }
  },

  // Whatever the daemon has pulled, or whatever the subscription grants.
  models(signal?: AbortSignal, onStatus?: (status: string) => void): Promise<string[]> {
    const at = endpoint();
    return listModels(`${at.baseUrl}/v1/models`, headers(at), signal, onStatus);
  },

  async efforts(): Promise<readonly string[]> {
    return OLLAMA_EFFORTS;
  },

  async contextWindow(
    model: string,
    signal?: AbortSignal,
    onStatus?: (status: string) => void,
  ): Promise<ModelContextWindow | undefined> {
    const at = endpoint();
    const cacheKey = `${at.baseUrl}\u0000${model}`;
    const cached = contextByEndpoint.get(cacheKey);
    if (cached !== undefined && cached.expiresAt > Date.now()) return cached.value;
    const observed = await nativeContextWindow(at, model, signal, onStatus);
    if (observed?.runtime === true) {
      contextByEndpoint.set(cacheKey, {
        value: observed.value,
        expiresAt: Date.now() + CONTEXT_CACHE_MS,
      });
    }
    return observed?.value;
  },

  location: () => {
    try {
      return endpoint().loopback ? "local" : "cloud";
    } catch {
      return "cloud";
    }
  },

  async send(req: SendRequest): Promise<Message> {
    const at = endpoint();
    const effort = requireSupportedEffort(req.model, req.effort, OLLAMA_EFFORTS);

    // The OpenAI-compatible endpoint accepts this vocabulary for thinking
    // models. Invalid levels are rejected locally instead of being rewritten.
    const events = await postSse(
      `${at.baseUrl}/v1/chat/completions`,
      headers(at),
      {
        model: req.model,
        messages: toWireMessages(req.system, req.messages),
        tools: req.tools.map(toWireTool),
        max_tokens: req.maxTokens,
        reasoning_effort: effort,
        stream: true,
      },
      req.maxTokens,
      req.signal,
      req.onStatus,
    );

    const reply = await assembleOllama(events, req.onStream);

    const notice = stopNotice(reply);
    if (notice !== undefined) req.onStream?.({ kind: "text", text: `\n${notice}` });

    return fromWireReply(reply);
  },
};

async function nativeContextWindow(
  at: ReturnType<typeof endpoint>,
  model: string,
  signal?: AbortSignal,
  onStatus?: (status: string) => void,
): Promise<{ value: ModelContextWindow; runtime: boolean } | undefined> {
  try {
    const running = await getJson(`${at.baseUrl}/api/ps`, headers(at), signal, onStatus);
    const allocated = runningContext(running, model);
    if (allocated !== undefined) return { value: usableContext(allocated), runtime: true };
  } catch (error) {
    throwIfAborted(signal, error);
  }

  try {
    const details = await postJson(
      `${at.baseUrl}/api/show`,
      headers(at),
      { model },
      signal,
      onStatus,
    );
    const capacity = modelCapacity(details);
    return capacity === undefined
      ? undefined
      : { value: usableContext(capacity), runtime: false };
  } catch (error) {
    throwIfAborted(signal, error);
    return undefined;
  }
}

function runningContext(value: unknown, model: string): number | undefined {
  if (!record(value) || !Array.isArray(value["models"])) return undefined;
  for (const entry of value["models"]) {
    if (!record(entry)) continue;
    if (entry["name"] !== model && entry["model"] !== model) continue;
    if (validTokenCount(entry["context_length"])) return entry["context_length"];
  }
  return undefined;
}

function modelCapacity(value: unknown): number | undefined {
  if (!record(value) || !record(value["model_info"])) return undefined;
  const capacities = Object.entries(value["model_info"])
    .filter(([name, count]) => name.endsWith(".context_length") && validTokenCount(count))
    .map(([, count]) => count as number);
  return capacities.length === 0 ? undefined : Math.max(...capacities);
}

function validTokenCount(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) &&
    value >= 4_096 && value <= 10_000_000;
}

function usableContext(tokens: number): ModelContextWindow {
  return Object.freeze({ tokens: Math.floor(tokens * 95 / 100) });
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function throwIfAborted(signal: AbortSignal | undefined, error: unknown): void {
  if (signal?.aborted === true) throw error;
}

function endpoint() {
  return ollamaConnection();
}

function apiKey(): string | undefined {
  return keyFor(KEY);
}

function headers(at: ReturnType<typeof endpoint>): Record<string, string> {
  if (at.loopback) return {};
  const key = apiKey();
  if (key !== undefined) return { authorization: `Bearer ${key}` };
  throw new Error(`${KEY} is not set (required by ${at.baseUrl})`);
}
