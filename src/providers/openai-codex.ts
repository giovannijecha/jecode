// ChatGPT-backed Codex Responses, kept separate from the OpenAI API provider.

import { randomUUID } from "node:crypto";
import type { Message, Provider, SendRequest } from "../types.ts";
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
    effortByModel = catalog.efforts;
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
    effortByModel = catalog.efforts;
    return effortByModel.get(model) ?? fallbackEfforts(model);
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
): Promise<{ ids: string[]; efforts: Map<string, readonly string[]> }> {
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

function modelCatalog(value: unknown): {
  ids: string[];
  efforts: Map<string, readonly string[]>;
} {
  const source = record(value) && Array.isArray(value["models"]) ? value["models"] : undefined;
  if (source === undefined) throw new Error("OpenAI Codex did not return a model list");

  const seen = new Set<string>();
  const models = source
    .slice(0, MAX_CATALOG_ITEMS)
    .flatMap((entry): { id: string; priority: number; efforts: readonly string[] }[] => {
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
      }];
    })
    .sort((left, right) => left.priority - right.priority)
    .slice(0, MAX_MODELS);
  return {
    ids: models.map((entry) => entry.id),
    efforts: new Map(models.map((entry) => [entry.id, entry.efforts])),
  };
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
