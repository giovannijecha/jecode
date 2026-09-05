// Ollama Cloud through its OpenAI-compatible streaming Chat Completions API.

import type { Message, Provider, SendRequest } from "../types.ts";
import { requireSupportedEffort } from "../effort.ts";
import { postSse } from "./http.ts";
import { listModels } from "./catalog.ts";
import { keyFor } from "../credentials.ts";
import { assembleOllama } from "./ollama-stream.ts";
import { ollamaContextWindow } from "./ollama-context.ts";
import {
  isRetryableGenerationFailure,
  isRetryableReadFailure,
  throwProviderError,
} from "./failure.ts";
import { OLLAMA_CLOUD_HOST } from "./ollama-endpoint.ts";
import { fromWireReply, stopNotice, toWireMessages, toWireTool } from "./ollama-wire.ts";

const KEY = "OLLAMA_API_KEY";
const ID = "ollama";
// Ollama also accepts `none`; Jecode's product-wide reasoning floor is `low`.
const OLLAMA_EFFORTS = ["low", "medium", "high"] as const;

export const ollama: Provider = {
  id: ID,
  defaultModel: "",
  auth: { kind: "api-key", keyVar: KEY },

  blocked(): string | undefined {
    return keyFor(KEY) === undefined ? `${KEY} is not set` : undefined;
  },

  async models(signal?: AbortSignal, onStatus?: (status: string) => void): Promise<string[]> {
    try {
      return await listModels(
        `${OLLAMA_CLOUD_HOST}/v1/models`,
        headers(),
        signal,
        onStatus,
        (error) => isRetryableReadFailure(ID, error),
      );
    } catch (error) {
      throwProviderError(ID, signal, error);
    }
  },

  async efforts(): Promise<readonly string[]> {
    return OLLAMA_EFFORTS;
  },

  async contextWindow(model, signal, onStatus) {
    return ollamaContextWindow(model, headers(), signal, onStatus);
  },

  async send(req: SendRequest): Promise<Message> {
    const effort = requireSupportedEffort(req.model, req.effort, OLLAMA_EFFORTS);
    try {
      const events = await postSse(
        `${OLLAMA_CLOUD_HOST}/v1/chat/completions`,
        headers(),
        {
          model: req.model,
          messages: toWireMessages(req.system, req.messages),
          tools: req.tools.map(toWireTool),
          max_tokens: req.maxTokens,
          reasoning_effort: effort,
          stream: true,
          stream_options: { include_usage: true },
        },
        req.maxTokens,
        req.signal,
        req.onStatus,
        undefined,
        (error) => isRetryableGenerationFailure(ID, error),
      );
      const reply = await assembleOllama(events, req.onStream);
      const notice = stopNotice(reply);
      if (notice !== undefined) req.onStream?.({ kind: "text", text: `\n${notice}` });
      return fromWireReply(reply);
    } catch (error) {
      throwProviderError(ID, req.signal, error);
    }
  },
};

function headers(): Record<string, string> {
  const key = keyFor(KEY);
  if (key === undefined) throw new Error(`${KEY} is not set`);
  return { authorization: `Bearer ${key}` };
}
