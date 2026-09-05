import type { Provider } from "../types.ts";
import { anthropic } from "./anthropic.ts";
import { openai } from "./openai.ts";
import { openaiCodex } from "./openai-codex.ts";
import { ollama } from "./ollama.ts";

export const PROVIDERS: readonly Provider[] = [anthropic, openai, openaiCodex, ollama];

export function providerNames(): string[] {
  return PROVIDERS.map((provider) => provider.id);
}

export function selectProvider(id: string): Provider {
  const found = PROVIDERS.find((provider) => provider.id === id);
  if (found === undefined) {
    throw new Error(`unknown provider "${id}" (available: ${providerNames().join(", ")})`);
  }
  return found;
}
