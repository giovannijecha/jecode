import type { Provider } from "../types.ts";
import type { Config } from "../config.ts";
import { anthropic } from "./anthropic.ts";
import { openai } from "./openai.ts";
import { configureOllama, ollama } from "./ollama.ts";

export const PROVIDERS: readonly Provider[] = [anthropic, openai, ollama];

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

/** Apply provider-specific process state after startup precedence is resolved. */
export function configureProviders(config: Config): void {
  configureOllama(config.ollamaHost);
}
