import { anthropic } from "./anthropic.js";
import { openai } from "./openai.js";
import { configureOllama, ollama } from "./ollama.js";
export const PROVIDERS = [anthropic, openai, ollama];
export function providerNames() {
    return PROVIDERS.map((provider) => provider.id);
}
export function selectProvider(id) {
    const found = PROVIDERS.find((provider) => provider.id === id);
    if (found === undefined) {
        throw new Error(`unknown provider "${id}" (available: ${providerNames().join(", ")})`);
    }
    return found;
}
/** Apply provider-specific process state after startup precedence is resolved. */
export function configureProviders(config) {
    configureOllama(config.ollamaHost);
}
