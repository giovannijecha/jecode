// The fixed API endpoint and recognition of its retired saved setting.

export const OLLAMA_CLOUD_HOST = "https://ollama.com";

/** Recognize old official-cloud values without making endpoints configurable. */
export function isLegacyOllamaCloudHost(value: unknown): boolean {
  if (typeof value !== "string") return false;
  try {
    return new URL(value.trim()).href.replace(/\/+$/, "") === OLLAMA_CLOUD_HOST;
  } catch {
    return false;
  }
}
