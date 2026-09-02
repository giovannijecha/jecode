// ChatGPT-backed Codex Responses, kept separate from the OpenAI API provider.

import { randomUUID } from "node:crypto";
import type { Message, ModelContextWindow, Provider, SendRequest } from "../types.ts";
import { openAICodexAccount } from "../accounts.ts";
import { openAIAuthorization } from "../openai-account.ts";
import { applicationVersion } from "../version.ts";
import { EFFORTS, isEffort, requireSupportedEffort } from "../effort.ts";
import { getJson, postSse } from "./http.ts";
import { assembleOpenAI } from "./openai-stream.ts";
import {
  fromWireResponse,
  stopNotice,
  toWireItems,
  toWireTool,
} from "./openai-wire.ts";

const ID = "openai-codex";
const BASE = "https://chatgpt.com/backend-api/codex";
// Jecode's product version is unrelated to the Codex protocol gate. OpenAI's
// own catalogue updater uses this sentinel to request the complete current
// manifest; Jecode then keeps only entries explicitly visible in that manifest.
const CATALOG_COMPATIBILITY_VERSION = "99.99.99";
const SESSION_ID = randomUUID();
const MAX_CATALOG_ITEMS = 4_000;
const MAX_MODELS = 1_000;
const MAX_MODEL_CHARS = 256;
const XHIGH_EFFORTS = ["low", "medium", "high", "xhigh"] as const;
let effortByModel = new Map<string, readonly string[]>();
let contextByModel = new Map<string, ModelContextWindow | undefined>();

export const openaiCodex: Provider = {
  id: ID,
  defaultModel: "",
  auth: { kind: "oauth", account: ID, label: "ChatGPT" },

  blocked(): string | undefined {
    return openAICodexAccount() === undefined ? "ChatGPT account is not connected" : undefined;
  },

  location: () => "cloud",

  async models(signal?: AbortSignal, onStatus?: (status: string) => void): Promise<string[]> {
    const catalog = await loadCatalog(signal, onStatus);
    rememberCatalog(catalog);
    return catalog.ids;
  },

  async efforts(
    model: string,
    signal?: AbortSignal,
    onStatus?: (status: string) => void,
  ): Promise<readonly string[]> {
    const cached = effortByModel.get(model);
    if (cached !== undefined) return cached;
    const catalog = await loadCatalog(signal, onStatus);
    rememberCatalog(catalog);
    return effortByModel.get(model) ?? fallbackEfforts(model);
  },

  async contextWindow(
    model: string,
    signal?: AbortSignal,
    onStatus?: (status: string) => void,
  ): Promise<ModelContextWindow | undefined> {
    if (contextByModel.has(model)) return contextByModel.get(model);
    const catalog = await loadCatalog(signal, onStatus);
    rememberCatalog(catalog);
    const context = contextByModel.get(model);
    if (!contextByModel.has(model)) contextByModel.set(model, undefined);
    return context;
  },

  async send(req: SendRequest): Promise<Message> {
    const efforts = effortByModel.get(req.model) ?? fallbackEfforts(req.model);
    const effort = requireSupportedEffort(req.model, req.effort, efforts);
    return withAuthorization(async (authorization) => {
      const events = await postSse(
        `${BASE}/responses`,
        {
          ...headers(authorization, randomUUID()),
          "openai-beta": "responses=experimental",
        },
        {
          model: req.model,
          store: false,
          stream: true,
          instructions: req.system,
          input: req.messages.flatMap((message) => toWireItems(message, ID)),
          tools: req.tools.map(toWireTool),
          tool_choice: "auto",
          parallel_tool_calls: true,
          reasoning: { effort, summary: "auto" },
          text: { verbosity: "low" },
          include: ["reasoning.encrypted_content"],
          prompt_cache_key: SESSION_ID,
        },
        req.maxTokens,
        req.signal,
        req.onStatus,
      );
      const data = await assembleOpenAI(events, req.onStream);
      const notice = stopNotice(data);
      if (notice !== undefined) req.onStream?.({ kind: "text", text: `\n${notice}` });
      return fromWireResponse(data, ID);
    }, req.signal, req.onStatus);
  },
};

async function loadCatalog(
  signal?: AbortSignal,
  onStatus?: (status: string) => void,
): Promise<ModelCatalog> {
  return withAuthorization(async (authorization) => {
    const body = await getJson(
      `${BASE}/models?client_version=${CATALOG_COMPATIBILITY_VERSION}`,
      headers(authorization, randomUUID()),
      signal,
      onStatus,
    );
    return modelCatalog(body);
  }, signal, onStatus);
}

type ModelCatalog = {
  ids: string[];
  efforts: Map<string, readonly string[]>;
  contexts: Map<string, ModelContextWindow | undefined>;
};

function rememberCatalog(catalog: ModelCatalog): void {
  effortByModel = catalog.efforts;
  contextByModel = catalog.contexts;
}

async function withAuthorization<T>(
  operation: (authorization: Awaited<ReturnType<typeof openAIAuthorization>>) => Promise<T>,
  signal?: AbortSignal,
  onStatus?: (status: string) => void,
): Promise<T> {
  let authorization = await openAIAuthorization(undefined, signal, onStatus);
  try {
    return await operation(authorization);
  } catch (error) {
    if (statusOf(error) !== 401) throw error;
    authorization = await openAIAuthorization(authorization.accessToken, signal, onStatus);
    return operation(authorization);
  }
}

function headers(
  authorization: Awaited<ReturnType<typeof openAIAuthorization>>,
  requestId: string,
): Record<string, string> {
  const version = applicationVersion();
  return {
    authorization: `Bearer ${authorization.accessToken}`,
    "chatgpt-account-id": authorization.accountId,
    originator: "jecode",
    "user-agent": `jecode/${version} (${process.platform}; ${process.arch})`,
    "session-id": SESSION_ID,
    "x-client-request-id": requestId,
  };
}

function modelCatalog(value: unknown): ModelCatalog {
  const source = record(value) && Array.isArray(value["models"]) ? value["models"] : undefined;
  if (source === undefined) throw new Error("OpenAI Codex did not return a model list");

  const seen = new Set<string>();
  const models = source
    .slice(0, MAX_CATALOG_ITEMS)
    .flatMap((entry): {
      id: string;
      priority: number;
      efforts: readonly string[];
      context: ModelContextWindow | undefined;
    }[] => {
      if (!record(entry)) return [];
      const id = entry["slug"];
      if (
        typeof id !== "string" ||
        id === "" ||
        id.length > MAX_MODEL_CHARS ||
        entry["visibility"] !== "list" ||
        seen.has(id)
      ) return [];
      seen.add(id);
      return [{
        id,
        priority: typeof entry["priority"] === "number" ? entry["priority"] : 0,
        efforts: reasoningLevels(entry, id),
        context: modelContextWindow(entry),
      }];
    })
    .sort((left, right) => left.priority - right.priority)
    .slice(0, MAX_MODELS);
  return {
    ids: models.map((entry) => entry.id),
    efforts: new Map(models.map((entry) => [entry.id, entry.efforts])),
    contexts: new Map(models.map((entry) => [entry.id, entry.context])),
  };
}

function modelContextWindow(entry: Record<string, unknown>): ModelContextWindow | undefined {
  const resolved = tokenCount(entry["context_window"]) ?? tokenCount(entry["max_context_window"]);
  if (resolved === undefined) return undefined;
  const percent = percentage(entry["effective_context_window_percent"]) ?? 95;
  const tokens = Math.floor(resolved * percent / 100);
  if (!validTokenCount(tokens)) return undefined;
  const automatic = Math.floor(resolved * 9 / 10);
  const advertised = tokenCount(entry["auto_compact_token_limit"]);
  return Object.freeze({
    tokens,
    compactAtTokens: Math.min(advertised ?? automatic, automatic, tokens),
  });
}

function tokenCount(value: unknown): number | undefined {
  return validTokenCount(value) ? value : undefined;
}

function validTokenCount(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) &&
    value >= 4_096 && value <= 10_000_000;
}

function percentage(value: unknown): number | undefined {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 1 && value <= 100
    ? value
    : undefined;
}

function reasoningLevels(entry: Record<string, unknown>, model: string): readonly string[] {
  const source = entry["supported_reasoning_levels"];
  if (!Array.isArray(source)) return fallbackEfforts(model);
  const seen = new Set<string>();
  const efforts = source.flatMap((level): string[] => {
    if (!record(level) || !isEffort(level["effort"]) || seen.has(level["effort"])) return [];
    seen.add(level["effort"]);
    return [level["effort"]];
  });
  return efforts.length === 0 ? fallbackEfforts(model) : efforts;
}

function fallbackEfforts(model: string): readonly string[] {
  if (/^gpt-5\.6-(?:sol|terra|luna)(?:-|$)/.test(model)) return EFFORTS;
  return XHIGH_EFFORTS;
}

function statusOf(error: unknown): number | undefined {
  return typeof error === "object" && error !== null && "status" in error
    ? (error as { status?: number }).status
    : undefined;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
