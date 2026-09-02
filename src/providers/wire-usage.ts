// Usage comes from remote JSON and eventually reaches the strict session
// codec. Normalize it at the wire boundary so one malformed counter cannot
// make an otherwise valid saved session unreadable.

export function wireTokenCount(value: unknown): number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : 0;
}
