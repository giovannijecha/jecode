// ChatGPT-backed Codex Responses, kept separate from the OpenAI API provider.

import { randomUUID } from "node:crypto";
import type { Message, Provider, SendRequest } from "../types.ts";
import { openAICodexAccount } from "../accounts.ts";
import { openAIAuthorization } from "../openai-account.ts";
import { applicationVersion } from "../version.ts";
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

export const openaiCodex: Provider = {
  id: ID,
  defaultModel: "",
  auth: { kind: "oauth", account: ID, label: "ChatGPT" },

  blocked(): string | undefined {
    return openAICodexAccount() === undefined ? "ChatGPT account is not connected" : undefined;
  },

  location: () => "cloud",

  async models(signal?: AbortSignal, onStatus?: (status: string) => void): Promise<string[]> {
    return withAuthorization(async (authorization) => {
      const body = await getJson(
        `${BASE}/models?client_version=${CATALOG_COMPATIBILITY_VERSION}`,
        headers(authorization, randomUUID()),
        signal,
        onStatus,
      );
      return modelIds(body);
    }, signal, onStatus);
  },

  async send(req: SendRequest): Promise<Message> {
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
          reasoning: { effort: req.effort, summary: "auto" },
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

function modelIds(value: unknown): string[] {
  const source = record(value) && Array.isArray(value["models"]) ? value["models"] : undefined;
  if (source === undefined) throw new Error("OpenAI Codex did not return a model list");

  const seen = new Set<string>();
  return source
    .slice(0, MAX_CATALOG_ITEMS)
    .flatMap((entry): { id: string; priority: number }[] => {
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
      return [{ id, priority: typeof entry["priority"] === "number" ? entry["priority"] : 0 }];
    })
    .sort((left, right) => left.priority - right.priority)
    .slice(0, MAX_MODELS)
    .map((entry) => entry.id);
}

function statusOf(error: unknown): number | undefined {
  return typeof error === "object" && error !== null && "status" in error
    ? (error as { status?: number }).status
    : undefined;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
