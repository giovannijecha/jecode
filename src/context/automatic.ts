// Process-local breaker for optional automatic compaction. It prevents an
// unchanged provider projection from opening the same failing summary request
// at consecutive checkpoint/request boundaries.

export type AutomaticCompactionReason = "budget" | "overflow";

export type AutomaticCompactionAttempt = Readonly<{
  key: string;
  reason: AutomaticCompactionReason;
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

  return Object.freeze({
    allows(attempt) {
      return !settled.has(settledKey(attempt));
    },
    failed(attempt) {
      remember(settled, settledKey(attempt));
    },
    succeeded(attempt) {
      remember(settled, settledKey(attempt));
    },
    reset() {
      settled.clear();
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
