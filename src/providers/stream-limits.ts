// Response streams are remote input. Bound the pieces that otherwise grow
// independently, while letting a larger requested output carry its necessarily
// larger framing, terminal envelope, and opaque reasoning payloads.

export const MAX_SSE_EVENT_CHARS = 1_000_000;
export const MAX_SSE_STREAM_CHARS = 256_000_000;
export const MAX_TOOL_ARGUMENT_CHARS = 1_000_000;

const MIN_SSE_STREAM_CHARS = 4_000_000;
const SSE_CHARS_PER_OUTPUT_TOKEN = 512;

export function sseStreamCharacterLimit(maxOutputTokens: number): number {
  if (!Number.isSafeInteger(maxOutputTokens) || maxOutputTokens <= 0) {
    throw new Error("max output tokens must be a positive safe integer");
  }
  const scaled = maxOutputTokens > Math.floor(MAX_SSE_STREAM_CHARS / SSE_CHARS_PER_OUTPUT_TOKEN)
    ? MAX_SSE_STREAM_CHARS
    : maxOutputTokens * SSE_CHARS_PER_OUTPUT_TOKEN;
  return Math.max(MIN_SSE_STREAM_CHARS, scaled);
}

export function addBounded(
  total: number,
  added: number,
  maximum: number,
  label: string,
): number {
  if (added > maximum - total) {
    throw new Error(`${label} exceeded ${maximum} characters`);
  }
  return total + added;
}
