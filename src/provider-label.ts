// Stable human-facing names for provider identifiers.

import type { Provider } from "./types.ts";

export function providerLabel(id: string): string {
  switch (id) {
    case "anthropic": return "Anthropic API";
    case "openai": return "OpenAI API";
    case "openai-codex": return "OpenAI Account";
    case "ollama": return "Ollama API";
    default: return id === "" ? "Provider" : `${id[0]?.toUpperCase() ?? ""}${id.slice(1)}`;
  }
}

/** Name the actual connection a model request will use. */
export function providerRouteLabel(provider: Provider): string {
  if (provider.id === "openai") return "OpenAI API · billed usage";
  return providerLabel(provider.id);
}
