// One ordered vocabulary for reasoning depth. Providers expose the subset a
// selected model accepts; this module only validates and reconciles that data.

export const EFFORTS = ["low", "medium", "high", "xhigh", "max"] as const;

export type Effort = (typeof EFFORTS)[number];

export function isEffort(value: unknown): value is Effort {
  return typeof value === "string" && (EFFORTS as readonly string[]).includes(value);
}

export function requireSupportedEffort(
  model: string,
  requested: string,
  supported: readonly string[],
): string {
  if (supported.includes(requested)) return requested;
  const choices = supported.length === 0 ? "none" : supported.join(", ");
  throw new Error(`${model} does not support effort "${requested}" (available: ${choices})`);
}

/** Keep the requested depth when possible, otherwise choose the nearest lower level. */
export function compatibleEffort(
  requested: string,
  supported: readonly string[],
): string | undefined {
  if (supported.length === 0) return undefined;
  if (supported.includes(requested)) return requested;
  const requestedIndex = EFFORTS.indexOf(requested as Effort);
  const candidates = EFFORTS.filter((value) => supported.includes(value));
  if (requestedIndex < 0) return candidates[0];
  return [...candidates].reverse().find((value) => EFFORTS.indexOf(value) < requestedIndex)
    ?? candidates[0];
}
