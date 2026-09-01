// Actionable provider failures for user-facing command and turn surfaces.

import type { Provider } from "./types.ts";
import { providerLabel } from "./provider-label.ts";

const CONNECTION_FAILURE =
  /network error calling|timed out waiting for response headers|response body was idle/i;

export function providerFailure(
  provider: Provider,
  error: Error,
  labelProvider = false,
): string {
  if (provider.id === "ollama" && CONNECTION_FAILURE.test(error.message)) {
    return provider.location?.() === "local"
      ? "Ollama is not reachable on this computer · start Ollama or choose cloud in /providers"
      : "Ollama is not reachable · check its connection in /providers";
  }
  return labelProvider ? `${providerLabel(provider.id)}: ${error.message}` : error.message;
}
