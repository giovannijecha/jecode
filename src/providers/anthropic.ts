// Anthropic Messages API, spoken directly.
//
// Wire contract (verified 2026-08-29): POST /v1/messages, auth via the
// `x-api-key` header plus `anthropic-version`. Thinking is adaptive — the
// `budget_tokens` form is rejected with a 400 on current models — and depth is
// steered by `output_config.effort` instead. `display: "summarized"` is set on
// purpose: the default omits the reasoning text, which on a screen reads as a
// long silence before anything appears.

import type { Message, ModelContextWindow, Provider, SendRequest } from "../types.ts";
import { postSse } from "./http.ts";
import { modelCatalog } from "./catalog.ts";
import { keyFor } from "../credentials.ts";
import { EFFORTS, requireSupportedEffort } from "../effort.ts";
import { assembleAnthropic } from "./anthropic-stream.ts";
import { fromWireResponse, stopNotice, toWireMessage, toWireTool } from "./anthropic-wire.ts";

const ENDPOINT = "https://api.anthropic.com/v1/messages";
const MODELS = "https://api.anthropic.com/v1/models?limit=100";
const API_VERSION = "2023-06-01";
const KEY = "ANTHROPIC_API_KEY";

const ADAPTIVE = /^claude-(?:fable-5|mythos-(?:5|preview)|opus-(?:5|4-[678])|sonnet-(?:5|4-6))(?:-|$)/;
const MAX_WITHOUT_XHIGH = ["low", "medium", "high", "max"] as const;
const ANTHROPIC_45_EFFORTS = ["low", "medium", "high"] as const;
let contextByModel = new Map<string, ModelContextWindow | undefined>();

export function supportsAdaptiveThinking(model: string): boolean {
  return ADAPTIVE.test(model);
}

export function anthropicEfforts(model: string): readonly string[] {
  if (/^claude-(?:(?:opus|sonnet)-4-6|mythos-preview)(?:-|$)/.test(model)) {
    return MAX_WITHOUT_XHIGH;
  }
  if (/^claude-opus-4-5(?:-|$)/.test(model)) return ANTHROPIC_45_EFFORTS;
  return supportsAdaptiveThinking(model) ? EFFORTS : [];
}

export const anthropic: Provider = {
  id: "anthropic",
  // Sonnet is the default because it is the one that can be left running.
  // Opus via `--model claude-opus-5`, Haiku via `--model claude-haiku-4-5`.
  defaultModel: "claude-sonnet-5",
  auth: { kind: "api-key", keyVar: KEY },

  blocked(): string | undefined {
    return apiKey() === undefined ? `${KEY} is not set` : undefined;
  },

  // Newest first is how the endpoint already answers, so the order is left
  // exactly as it arrives rather than re-sorted into something less useful.
  async models(signal?: AbortSignal, onStatus?: (status: string) => void): Promise<string[]> {
    const catalog = await loadModels(signal, onStatus);
    contextByModel = catalog.contexts;
    return catalog.ids;
  },

  async efforts(model: string): Promise<readonly string[]> {
    return anthropicEfforts(model);
  },

  async contextWindow(
    model: string,
    signal?: AbortSignal,
    onStatus?: (status: string) => void,
  ): Promise<ModelContextWindow | undefined> {
    if (contextByModel.has(model)) return contextByModel.get(model);
    const catalog = await loadModels(signal, onStatus);
    contextByModel = catalog.contexts;
    const context = contextByModel.get(model);
    if (!contextByModel.has(model)) contextByModel.set(model, undefined);
    return context;
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
    }
    const efforts = anthropicEfforts(req.model);
    if (efforts.length > 0) {
      body["output_config"] = {
        effort: requireSupportedEffort(req.model, req.effort, efforts),
      };
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

async function loadModels(
  signal?: AbortSignal,
  onStatus?: (status: string) => void,
): Promise<{
  ids: string[];
  contexts: Map<string, ModelContextWindow | undefined>;
}> {
  const entries = await modelCatalog(MODELS, headers(requireKey()), signal, onStatus);
  return {
    ids: entries.map((entry) => entry.id),
    contexts: new Map(entries.map((entry) => [
      entry.id,
      contextWindow(entry.metadata["max_input_tokens"]),
    ])),
  };
}

function contextWindow(value: unknown): ModelContextWindow | undefined {
  return validTokenCount(value) ? Object.freeze({ tokens: value }) : undefined;
}

function validTokenCount(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) &&
    value >= 4_096 && value <= 10_000_000;
}

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
