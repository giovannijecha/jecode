// One process-wide cancellation boundary for operating-system signals.

const SHUTDOWN_GRACE_MS = 2_000;

const SIGNALS = [
  ["SIGINT", 2],
  ["SIGHUP", 1],
  ["SIGTERM", 15],
] as const;

export class ProcessSignalError extends Error {
  readonly signal: NodeJS.Signals;
  readonly exitCode: number;

  constructor(signal: NodeJS.Signals, exitCode: number) {
    super(`received ${signal}`);
    this.name = "ProcessSignalError";
    this.signal = signal;
    this.exitCode = exitCode;
  }
}

export function isProcessSignalError(error: unknown): error is ProcessSignalError {
  return error instanceof ProcessSignalError;
}

/**
 * Abort foreground work on the first fatal signal and reserve a bounded hard
 * exit for work that ignores cancellation. A second signal exits immediately.
 */
export async function withProcessShutdown<T>(
  work: (signal: AbortSignal) => Promise<T>,
): Promise<T> {
  const control = new AbortController();
  let exitCode: number | undefined;
  let forceTimer: NodeJS.Timeout | undefined;
  const listeners: Array<[NodeJS.Signals, () => void]> = [];

  for (const [name, number] of SIGNALS) {
    const listener = (): void => {
      if (control.signal.aborted) {
        process.exit(exitCode ?? 128 + number);
      }
      exitCode = 128 + number;
      process.exitCode = exitCode;
      control.abort(new ProcessSignalError(name, exitCode));
      forceTimer = setTimeout(() => process.exit(exitCode as number), SHUTDOWN_GRACE_MS);
    };
    listeners.push([name, listener]);
    process.on(name, listener);
  }

  try {
    return await work(control.signal);
  } finally {
    if (forceTimer !== undefined) clearTimeout(forceTimer);
    for (const [name, listener] of listeners) process.off(name, listener);
  }
}
