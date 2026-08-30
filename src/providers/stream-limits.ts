// Response streams are remote input. Bound the pieces that otherwise grow
// independently of the request's output-token setting.

export const MAX_SSE_EVENT_CHARS = 1_000_000;
export const MAX_SSE_STREAM_CHARS = 4_000_000;
export const MAX_TOOL_ARGUMENT_CHARS = 1_000_000;

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
