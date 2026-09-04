// Stable human-facing names for provider identifiers.

import type { Provider } from "./types.ts";

export function providerLabel(id: string): string {
  switch (id) {
    case "anthropic": return "Anthropic";
    case "openai": return "OpenAI API";
    case "openai-codex": return "ChatGPT";
    case "ollama": return "Ollama";
    default: return id === "" ? "Provider" : `${id[0]?.toUpperCase() ?? ""}${id.slice(1)}`;
  }
}

/** Name the actual connection a model request will use. */
export function providerRouteLabel(provider: Provider): string {
  if (provider.auth.kind === "oauth") return `${providerLabel(provider.id)} account`;
  if (provider.id === "openai") return "OpenAI API · billed usage";
  if (provider.id === "anthropic") return "Anthropic API";
  if (provider.id === "ollama") {
    return provider.location?.() === "local" ? "Ollama · local" : "Ollama API";
  }
  return providerLabel(provider.id);
}
