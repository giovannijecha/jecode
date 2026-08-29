// Anthropic Messages API, spoken directly.
//
// Wire contract (verified 2026-08-29): POST /v1/messages, auth via the
// `x-api-key` header plus `anthropic-version`. Thinking is adaptive — the
// `budget_tokens` form is rejected with a 400 on current models — and depth is
// steered by `output_config.effort` instead. `display: "summarized"` is set on
// purpose: the default omits the reasoning text, which on a screen reads as a
// long silence before anything appears.

import type { Message, Provider, SendRequest } from "../types.ts";
import { postSse } from "./http.ts";
import { listModels } from "./catalog.ts";
import { keyFor } from "../credentials.ts";
import { assembleAnthropic } from "./anthropic-stream.ts";
import { fromWireResponse, stopNotice, toWireMessage, toWireTool } from "./anthropic-wire.ts";

const ENDPOINT = "https://api.anthropic.com/v1/messages";
const MODELS = "https://api.anthropic.com/v1/models?limit=100";
const API_VERSION = "2023-06-01";
const KEY = "ANTHROPIC_API_KEY";

// Adaptive thinking and `output_config.effort` exist on the 4.6-and-later
// families only. Older models reject both with a 400, so `/model claude-haiku-4-5`
// would fail on send if the request shape were fixed. It follows the model.
const ADAPTIVE = /^claude-(fable-5|opus-(5|4-[678])|sonnet-(5|4-6))/;

export function supportsAdaptiveThinking(model: string): boolean {
  return ADAPTIVE.test(model);
}

export const anthropic: Provider = {
  id: "anthropic",
  // Sonnet is the default because it is the one that can be left running.
  // Opus via `--model claude-opus-5`, Haiku via `--model claude-haiku-4-5`.
  defaultModel: "claude-sonnet-5",
  keyVar: KEY,

  blocked(): string | undefined {
    return apiKey() === undefined ? `${KEY} is not set` : undefined;
  },

  // Newest first is how the endpoint already answers, so the order is left
  // exactly as it arrives rather than re-sorted into something less useful.
  models(signal?: AbortSignal, onStatus?: (status: string) => void): Promise<string[]> {
    return listModels(MODELS, headers(requireKey()), signal, onStatus);
  },

  location: () => "cloud",

  async send(req: SendRequest): Promise<Message> {
    const key = requireKey();

    const body: Record<string, unknown> = {
      model: req.model,
      max_tokens: req.maxTokens,
      system: req.system,
      messages: req.messages.map(toWireMessage),
      tools: req.tools.map(toWireTool),
      stream: true,
    };

    if (supportsAdaptiveThinking(req.model)) {
      body["thinking"] = { type: "adaptive", display: "summarized" };
      body["output_config"] = { effort: req.effort };
    }

    const events = await postSse(ENDPOINT, headers(key), body, req.signal, req.onStatus);

    const data = await assembleAnthropic(events, req.onStream);

    // A refusal or a truncation never arrives as streamed text, so it has to
    // be announced separately or the user watches the turn end in silence.
    const notice = stopNotice(data);
    if (notice !== undefined) req.onStream?.({ kind: "text", text: `\n${notice}` });

    return fromWireResponse(data);
  },
};

// Read at the moment it is used, never captured at import: a key typed into
// the running window has to count, and so does one exported after startup.
function apiKey(): string | undefined {
  return keyFor(KEY);
}

function requireKey(): string {
  const key = apiKey();
  if (key === undefined) throw new Error(`${KEY} is not set`);
  return key;
}

function headers(key: string): Record<string, string> {
  return { "x-api-key": key, "anthropic-version": API_VERSION };
}
