// Stable human-facing names for provider identifiers.

export function providerLabel(id: string): string {
  switch (id) {
    case "anthropic": return "Anthropic";
    case "openai": return "OpenAI";
    case "openai-codex": return "OpenAI Codex";
    case "ollama": return "Ollama";
    default: return id === "" ? "Provider" : `${id[0]?.toUpperCase() ?? ""}${id.slice(1)}`;
  }
}
