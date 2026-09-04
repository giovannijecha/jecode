// OpenAI Responses API, spoken directly.
//
// Responses wire contract verified against the official API reference on
// 2026-08-29. Keep final response events authoritative over display deltas.

import { randomUUID } from "node:crypto";
import type { Message, ModelContextWindow, Provider, SendRequest } from "../types.ts";
import { applicationVersion } from "../version.ts";
import { postSse } from "./http.ts";
import { listModels } from "./catalog.ts";
import { keyFor } from "../credentials.ts";
import { EFFORTS, requireSupportedEffort } from "../effort.ts";
import {
  isRetryableGenerationFailure,
  isRetryableReadFailure,
  throwProviderError,
} from "./failure.ts";
import { assembleOpenAI, openAIStreamProgress } from "./openai-stream.ts";
import {
  fromWireResponse,
  stopNotice,
  toWireItems,
  toWireTool,
} from "./openai-wire.ts";

const ENDPOINT = "https://api.openai.com/v1/responses";
const MODELS = "https://api.openai.com/v1/models";
const KEY = "OPENAI_API_KEY";
const ID = "openai";

const RESPONSES_REASONING_MODEL = /^(?:gpt-5(?:[.-]|$)|o(?:1|3|4)(?:[.-]|$)|codex-mini(?:[.-]|$))/;
// Jecode's transport always streams and always declares local tools. Hide
// catalog entries that cannot satisfy either half of that contract.
const INCOMPATIBLE_MODEL = /^(?:gpt-5(?:\.[1-3])?-chat-latest|gpt-5\.5-pro|o1-mini|o(?:1|3)-pro|o3-deep-research|o4-mini-deep-research)(?:-|$)/;
const STANDARD_EFFORTS = ["low", "medium", "high"] as const;
const XHIGH_EFFORTS = ["low", "medium", "high", "xhigh"] as const;
const PRO_EFFORTS = ["medium", "high", "xhigh"] as const;
const HIGH_ONLY_EFFORT = ["high"] as const;

export function supportsOpenAIModel(model: string): boolean {
  return RESPONSES_REASONING_MODEL.test(model) && !INCOMPATIBLE_MODEL.test(model);
}

export function openAIEfforts(model: string): readonly string[] {
  if (!supportsOpenAIModel(model)) return [];
  if (/^gpt-5-pro(?:-|$)/.test(model)) return HIGH_ONLY_EFFORT;
  if (/^gpt-5\.[2-5]-pro(?:-|$)/.test(model)) return PRO_EFFORTS;
  if (/^gpt-5\.6(?:[.-]|$)/.test(model)) return EFFORTS;
  if (/^gpt-5\.[2-5](?:[.-]|$)/.test(model)) return XHIGH_EFFORTS;
  if (/^(?:o(?:1|3|4)|codex-mini)(?:[.-]|$)/.test(model)) return STANDARD_EFFORTS;
  return STANDARD_EFFORTS;
}

/** Conservative capacities for the reasoning families accepted by this transport. */
export function openAIContextWindow(model: string): ModelContextWindow | undefined {
  if (/^gpt-5\.6(?:[.-]|$)/.test(model)) return usableContext(1_050_000);
  if (/^gpt-5(?:[.-]|$)/.test(model)) return usableContext(400_000);
  if (/^(?:o(?:1|3|4)|codex-mini)(?:[.-]|$)/.test(model)) {
    return usableContext(200_000);
  }
  return undefined;
}

function usableContext(tokens: number): ModelContextWindow {
  return Object.freeze({ tokens: Math.floor(tokens * 95 / 100) });
}

export const openai: Provider = {
  id: ID,
  defaultModel: "gpt-5",
  auth: { kind: "api-key", keyVar: KEY },

  blocked(): string | undefined {
    return apiKey() === undefined ? `${KEY} is not set` : undefined;
  },

  // The endpoint answers in no order worth keeping, so descending puts the
  // highest-numbered family — usually the newest — at the top of the menu.
  async models(signal?: AbortSignal, onStatus?: (status: string) => void): Promise<string[]> {
    try {
      const ids = await listModels(
        MODELS,
        headers(requireKey()),
        signal,
        onStatus,
        (error) => isRetryableReadFailure(ID, error),
      );
      return ids
        .filter(supportsOpenAIModel)
        .sort((a, b) => b.localeCompare(a));
    } catch (error) {
      throwProviderError(ID, signal, error);
    }
  },

  async efforts(model: string): Promise<readonly string[]> {
    return openAIEfforts(model);
  },

  async contextWindow(model: string): Promise<ModelContextWindow | undefined> {
    return openAIContextWindow(model);
  },

  location: () => "cloud",

  async send(req: SendRequest): Promise<Message> {
    const key = requireKey();
    const effort = requireSupportedEffort(req.model, req.effort, openAIEfforts(req.model));

    try {
      const events = await postSse(
        ENDPOINT,
        headers(key),
        {
          model: req.model,
          instructions: req.system,
          input: req.messages.flatMap((message) => toWireItems(message)),
          tools: req.tools.map(toWireTool),
          max_output_tokens: req.maxTokens,
          reasoning: { effort, summary: "auto" },
          store: false,
          include: ["reasoning.encrypted_content"],
          stream: true,
        },
        req.maxTokens,
        req.signal,
        req.onStatus,
        openAIStreamProgress,
        (error) => isRetryableGenerationFailure(ID, error),
      );

      const data = await assembleOpenAI(events, req.onStream, req.onStatus);

      const notice = stopNotice(data);
      if (notice !== undefined) req.onStream?.({ kind: "text", text: `\n${notice}` });

      return fromWireResponse(data);
    } catch (error) {
      throwProviderError(ID, req.signal, error);
    }
  },
};

function apiKey(): string | undefined {
  return keyFor(KEY);
}

function requireKey(): string {
  const key = apiKey();
  if (key === undefined) throw new Error(`${KEY} is not set`);
  return key;
}

function headers(key: string): Record<string, string> {
  return {
    authorization: `Bearer ${key}`,
    "user-agent": `jecode/${applicationVersion()} (${process.platform}; ${process.arch})`,
    "x-client-request-id": randomUUID(),
  };
}
