// Process-local breaker for optional automatic compaction. It prevents an
// unchanged provider projection from opening the same failing summary request
// at consecutive checkpoint/request boundaries.

export type AutomaticCompactionReason = "budget" | "overflow";

export type AutomaticCompactionAttempt = Readonly<{
  key: string;
  reason: AutomaticCompactionReason;
  /** Stable branch/model scope; growth is measured in input tokens, not messages. */
  scope?: string;
  inputTokens?: number;
  retryGrowthTokens?: number;
}>;

export type AutomaticCompactionGate = Readonly<{
  allows(attempt: AutomaticCompactionAttempt): boolean;
  failed(attempt: AutomaticCompactionAttempt): void;
  succeeded(attempt: AutomaticCompactionAttempt): void;
  reset(): void;
}>;

const MAX_SETTLED_GENERATIONS = 4_096;

export function automaticCompactionGate(
): AutomaticCompactionGate {
  const settled = new Set<string>();
  const failedPressure = new Map<string, number>();

  return Object.freeze({
    allows(attempt) {
      if (settled.has(settledKey(attempt))) return false;
      if (attempt.reason !== "budget" || attempt.scope === undefined) return true;
      const retryAt = failedPressure.get(attempt.scope);
      return retryAt === undefined || (attempt.inputTokens ?? 0) >= retryAt;
    },
    failed(attempt) {
      remember(settled, settledKey(attempt));
      if (attempt.scope !== undefined && attempt.inputTokens !== undefined) {
        failedPressure.set(attempt.scope, attempt.inputTokens + (attempt.retryGrowthTokens ?? 1_024));
        if (failedPressure.size > MAX_SETTLED_GENERATIONS) {
          failedPressure.delete(failedPressure.keys().next().value as string);
        }
      }
    },
    succeeded(attempt) {
      remember(settled, settledKey(attempt));
      if (attempt.scope !== undefined) failedPressure.delete(attempt.scope);
    },
    reset() {
      settled.clear();
      failedPressure.clear();
    },
  });
}

function settledKey(attempt: AutomaticCompactionAttempt): string {
  return `${attempt.reason}\0${attempt.key}`;
}

function remember(settled: Set<string>, key: string): void {
  settled.add(key);
  if (settled.size <= MAX_SETTLED_GENERATIONS) return;
  const oldest = settled.values().next().value as string | undefined;
  if (oldest !== undefined) settled.delete(oldest);
}

export function automaticCompactionKey(
  providerId: string,
  model: string,
  nodeId: number,
  canonicalMessages: number,
): string {
  return `${providerId}\0${model}\0${nodeId}\0${canonicalMessages}`;
}
